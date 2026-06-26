//! TLS infrastructure: connection-info layer, hot-reload state, and path validation.

use std::net::SocketAddr;
use std::task::{Context, Poll};

use axum::extract::ConnectInfo;

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
/// rotated, so it can warn operators that a restart is required to pick up
/// the new credentials. Full live reload (rebuilding the acceptor without
/// dropping connections) is not yet implemented; for now we only detect and
/// log.
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

/// Start a background watcher that polls cert/key files for changes every
/// [`TLS_RELOAD_POLL_INTERVAL`] seconds.
///
/// When a change is detected, logs a warning. Actually reloading TLS requires
/// rebuilding the acceptor state, which is complex. For now, we only detect
/// and warn so operators know a restart is needed.
///
/// Exits cleanly when the `shutdown` watch receives `true` or the sender is
/// dropped.
pub fn start_tls_reload_watcher(
    mut state: TlsReloadState,
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
                        tracing::warn!(
                            "TLS certificate files changed — restart required \
                             to apply new certificates"
                        );
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
