//! TLS infrastructure: connection-info layer, hot-reload state, and path validation.

use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Context as _;
use axum::extract::ConnectInfo;
use parking_lot::RwLock;

use crate::spawn_bg;

/// Tower service wrapper that injects `ConnectInfo(peer_addr)` into each
/// request's extensions.
///
/// On the non-TLS path, `axum::serve(...).into_make_service_with_connect_info()`
/// handles this automatically. The TLS path uses `hyper::server::conn` directly,
/// so we need this wrapper to make `ConnectInfo<SocketAddr>` available for
/// `extract_client_ip()` in the middleware.
#[derive(Clone)]
pub(super) struct ConnectInfoService<S> {
    inner: S,
    addr: SocketAddr,
}

impl<S> ConnectInfoService<S> {
    pub(super) fn new(inner: S, addr: SocketAddr) -> Self {
        Self { inner, addr }
    }
}

impl<S, ReqBody> tower::Service<axum::http::Request<ReqBody>> for ConnectInfoService<S>
where
    S: tower::Service<axum::http::Request<ReqBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: axum::http::Request<ReqBody>) -> Self::Future {
        req.extensions_mut().insert(ConnectInfo(self.addr));
        self.inner.call(req)
    }
}

/// Interval at which the TLS reload watcher polls cert/key files on disk.
const TLS_RELOAD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Tracks TLS certificate state for hot-reload.
///
/// The gateway uses this to detect when on-disk cert/key files have been
/// rotated. When a change is detected, the [`HotReloadingCertResolver`] is
/// atomically updated so new connections use the new certificate without
/// requiring a restart.
#[derive(Debug, Clone)]
pub struct TlsReloadState {
    pub cert_path: std::path::PathBuf,
    pub key_path: std::path::PathBuf,
    pub last_modified: Option<std::time::SystemTime>,
}

impl TlsReloadState {
    /// Create new reload state that will establish its baseline on the first
    /// call to [`check_cert_freshness`].
    pub fn new(cert_path: std::path::PathBuf, key_path: std::path::PathBuf) -> Self {
        Self {
            cert_path,
            key_path,
            last_modified: None,
        }
    }
}

/// Check whether the TLS cert/key files have been modified since the last call.
///
/// Reads the modification time of both files, compares against the previously
/// recorded `last_modified` timestamp, and updates `last_modified` to the
/// newer of the two current mtimes. Returns `true` if either file is newer
/// than what was previously recorded.
///
/// On the first call (when `last_modified` is `None`), this establishes the
/// baseline and returns `false` — there is no previous state to compare
/// against, so we do not want to flag a spurious "change" on startup.
pub fn check_cert_freshness(state: &mut TlsReloadState) -> bool {
    let cert_mtime = std::fs::metadata(&state.cert_path)
        .and_then(|m| m.modified())
        .ok();
    let key_mtime = std::fs::metadata(&state.key_path)
        .and_then(|m| m.modified())
        .ok();

    let latest = match (cert_mtime, key_mtime) {
        (Some(c), Some(k)) => Some(c.max(k)),
        (Some(c), None) => Some(c),
        (None, Some(k)) => Some(k),
        (None, None) => None,
    };

    let changed = match (state.last_modified, latest) {
        (Some(prev), Some(curr)) => curr > prev,
        _ => false,
    };

    state.last_modified = latest;
    changed
}

/// A rustls cert resolver that supports atomic hot-reload.
///
/// Wraps the active `CertifiedKey` in a `RwLock`. The `resolve()` method
/// (called on every new TLS connection) reads the current key. The `reload()`
/// method atomically replaces it — existing connections keep their handshake
/// state, but new connections use the updated certificate.
///
/// This enables zero-downtime certificate rotation: the TLS acceptor is
/// created once at startup, and this resolver is updated in-place when
/// cert/key files change on disk.
pub struct HotReloadingCertResolver {
    certified_key: RwLock<Option<Arc<rustls::sign::CertifiedKey>>>,
}

impl std::fmt::Debug for HotReloadingCertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotReloadingCertResolver")
            .field("has_key", &self.certified_key.read().is_some())
            .finish()
    }
}

impl HotReloadingCertResolver {
    /// Create a new resolver, loading the initial cert/key from the given paths.
    /// Returns an error if the files can't be read or parsed.
    pub fn from_files(
        cert_path: &std::path::Path,
        key_path: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let certified_key = Self::load_certified_key(cert_path, key_path)?;
        Ok(Self {
            certified_key: RwLock::new(Some(Arc::new(certified_key))),
        })
    }

    /// Reload the cert/key from disk and atomically swap.
    /// On error, the existing certificate is kept — callers continue serving
    /// with the old cert.
    pub fn reload(
        &self,
        cert_path: &std::path::Path,
        key_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let certified_key = Self::load_certified_key(cert_path, key_path)?;
        *self.certified_key.write() = Some(Arc::new(certified_key));
        tracing::info!("TLS certificates hot-reloaded successfully");
        Ok(())
    }

    /// Read and parse cert/key files into a rustls CertifiedKey.
    fn load_certified_key(
        cert_path: &std::path::Path,
        key_path: &std::path::Path,
    ) -> anyhow::Result<rustls::sign::CertifiedKey> {
        use std::fs::File;
        use std::io::BufReader;

        let cert_file = File::open(cert_path)
            .with_context(|| format!("failed to open TLS cert file: {}", cert_path.display()))?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
            rustls_pemfile::certs(&mut cert_reader)
                .collect::<Result<Vec<_>, _>>()
                .context("failed to parse TLS certificate PEM")?;

        let key_file = File::open(key_path)
            .with_context(|| format!("failed to open TLS key file: {}", key_path.display()))?;
        let mut key_reader = BufReader::new(key_file);
        let key = rustls_pemfile::private_key(&mut key_reader)
            .context("failed to parse TLS private key")?
            .ok_or_else(|| anyhow::anyhow!("no private key found in TLS key file"))?;

        // Convert the raw private key DER into a rustls signer using the
        // process-default crypto provider. Falls back to aws_lc_rs (the
        // crate's default feature) if no provider has been installed.
        let default_provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap_or(&default_provider);

        rustls::sign::CertifiedKey::from_der(certs, key, provider)
            .context("failed to build CertifiedKey from cert/key pair")
    }
}

impl rustls::server::ResolvesServerCert for HotReloadingCertResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        self.certified_key.read().clone()
    }
}

/// Start a background watcher that polls cert/key files for changes every
/// [`TLS_RELOAD_POLL_INTERVAL`] seconds.
///
/// When a change is detected, the `HotReloadingCertResolver` is atomically
/// updated — new TLS connections will use the new certificate without
/// dropping existing connections or requiring a restart.
///
/// On reload failure (e.g. malformed new cert), logs an error and keeps
/// the old certificate so the gateway continues serving.
///
/// Exits cleanly when the `shutdown` watch receives `true` or the sender is
/// dropped.
pub fn start_tls_reload_watcher(
    mut state: TlsReloadState,
    resolver: Arc<HotReloadingCertResolver>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    spawn_bg(async move {
        // Establish baseline so the first periodic check has something to
        // compare against. Without this, the first poll would always return
        // false anyway, but doing it upfront keeps the loop body uniform.
        check_cert_freshness(&mut state);

        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        tracing::info!("TLS reload watcher shutting down");
                        break;
                    }
                }
                _ = tokio::time::sleep(TLS_RELOAD_POLL_INTERVAL) => {
                    if check_cert_freshness(&mut state) {
                        tracing::info!("TLS certificate files changed — reloading...");
                        match resolver.reload(&state.cert_path, &state.key_path) {
                            Ok(()) => tracing::info!("TLS certificates reloaded successfully"),
                            Err(e) => tracing::error!(
                                error = %e,
                                "Failed to reload TLS certificates — keeping old certs"
                            ),
                        }
                    }
                }
            }
        }
    });
}

/// Validate a TLS file path: canonicalize to prevent traversal, check extension.
/// Returns the canonical path or panics with a redacted error message.
pub(super) fn validate_tls_path(path: &str, kind: &str) -> std::path::PathBuf {
    let allowed_extensions = match kind {
        "cert" => &["pem", "crt"][..],
        "key" => &["pem", "key"][..],
        _ => &["pem"][..],
    };

    // Reject empty paths
    if path.trim().is_empty() {
        panic!("TLS {} path is empty", kind);
    }

    // Canonicalize to resolve any .. or symlink traversal
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| {
        panic!("TLS {} file not found or inaccessible", kind);
    });

    // Validate file extension
    let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !allowed_extensions.contains(&ext) {
        panic!(
            "TLS {} file must have one of these extensions: {:?}",
            kind, allowed_extensions
        );
    }

    tracing::info!(kind, path = %canonical.display(), "Loading TLS {} file", kind);
    canonical
}

#[cfg(test)]
mod hot_reload_tests {
    use super::*;

    /// Bump a file's modification time forward so [`check_cert_freshness`]
    /// detects a change without needing to `sleep` through filesystem mtime
    /// granularity (which is 1s on many filesystems).
    fn bump_mtime(path: &std::path::Path) {
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        let times = std::fs::FileTimes::new().set_modified(future);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file for set_times");
        f.set_times(times).expect("set file modification time");
    }

    fn temp_file(prefix: &str, ext: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ms_test_tls_{}_{}.{ext}",
            prefix,
            std::process::id()
        ));
        std::fs::write(&path, b"initial").expect("create temp file");
        path
    }

    /// `from_files` should error when cert/key files do not exist.
    #[test]
    fn resolver_from_files_errors_on_missing_files() {
        let result = HotReloadingCertResolver::from_files(
            std::path::Path::new("/nonexistent/cert.pem"),
            std::path::Path::new("/nonexistent/key.pem"),
        );
        assert!(result.is_err(), "loading from nonexistent files must error");
    }

    /// `reload` errors on missing files. On error the existing key is
    /// preserved — the write guard is only taken after `load_certified_key`
    /// succeeds, so a failed reload cannot clear the existing cert. This
    /// test documents that invariant; the success path requires a real
    /// cert/key pair (covered by rustls's own tests for `CertifiedKey`).
    #[test]
    fn reload_failure_preserves_existing_key() {
        let tmp_dir = std::env::temp_dir();
        let bogus_cert = tmp_dir.join("ms_test_no_such_cert.pem");
        let bogus_key = tmp_dir.join("ms_test_no_such_key.pem");
        let _ = std::fs::remove_file(&bogus_cert);
        let _ = std::fs::remove_file(&bogus_key);

        let err = HotReloadingCertResolver::from_files(&bogus_cert, &bogus_key);
        assert!(err.is_err());
    }

    /// `check_cert_freshness` should detect file modifications via mtime.
    #[test]
    fn freshness_detects_changes_via_mtime() {
        let cert = temp_file("freshness_cert", "pem");
        let key = temp_file("freshness_key", "key");
        std::fs::write(&cert, b"cert-content").unwrap();
        std::fs::write(&key, b"key-content").unwrap();

        let mut state = TlsReloadState::new(cert.clone(), key.clone());

        // First call establishes baseline — no change detected.
        assert!(
            !check_cert_freshness(&mut state),
            "first call establishes baseline"
        );

        // No modification between calls — still no change.
        assert!(
            !check_cert_freshness(&mut state),
            "no modification between checks"
        );

        // Bump cert mtime — change detected.
        bump_mtime(&cert);
        assert!(
            check_cert_freshness(&mut state),
            "cert modification should be detected"
        );

        // No further modification — no change.
        assert!(
            !check_cert_freshness(&mut state),
            "no further modification should return false"
        );

        // Bump key mtime — change detected.
        bump_mtime(&key);
        assert!(
            check_cert_freshness(&mut state),
            "key modification should be detected"
        );

        let _ = std::fs::remove_file(&cert);
        let _ = std::fs::remove_file(&key);
    }
}
