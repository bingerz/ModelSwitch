//! Gateway server bootstrap: services initialization, Axum router, and start.

mod routes;
mod services;
mod tls;

// Re-export public API for backward compatibility.
pub use tls::{check_cert_freshness, start_tls_reload_watcher, TlsReloadState};
// Re-export so existing callers (and tests in this file) can reach it.
pub use routes::build_router;
// Re-export gateway state builder (implemented in `services`).
pub use services::start_gateway_services;

use crate::config;
use crate::proxy::AppState;
use crate::shutdown::shutdown_signal;

use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};

/// Start the Axum gateway server with graceful shutdown.
/// Binds to `host:port` and drains in-flight requests on SIGINT/SIGTERM.
/// Drain is bounded by `drain_timeout_secs` to prevent hanging.
/// If `shutdown_notify` is provided, uses it instead of signal-based shutdown.
#[allow(clippy::too_many_arguments)]
pub async fn start_gateway(
    state: Arc<AppState>,
    host: &str,
    port: u16,
    drain_timeout_secs: u64,
    shutdown_notify: Option<Arc<Notify>>,
    bind_notify: Option<oneshot::Sender<Result<(), String>>>,
    web_console_dir: Option<&str>,
    tls_config: config::TlsConfig,
) {
    // Security gate: refuse to start in web console mode without admin_token
    // on a non-loopback bind address. This prevents accidentally exposing
    // unauthenticated admin endpoints to the network.
    if state.security.admin_token.is_none() && web_console_dir.is_some() {
        let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
        if !is_loopback {
            let msg = format!(
                "Refusing to start: web console mode without admin_token on non-loopback address ({}:{}). \
                 Set [security] admin_token in config.toml or bind to 127.0.0.1.",
                host, port
            );
            tracing::error!("{msg}");
            if let Some(tx) = bind_notify {
                let _ = tx.send(Err(msg));
            }
            return;
        }
    }

    let app = build_router(state.clone(), web_console_dir);
    let quota_for_shutdown = Arc::clone(&state.billing.quota_store);
    let provider_budgets_for_shutdown = Arc::clone(&state.billing.provider_budgets);
    let mcp_for_shutdown = Arc::clone(&state.mcp.mcp_manager);
    let addr = format!("{}:{}", host, port);

    // Write PID file for CLI management
    let pid_path = config::app_config_dir().join("gateway.pid");
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let pid = std::process::id();
    if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
        tracing::warn!("Failed to write PID file: {}", e);
    } else {
        tracing::debug!("PID file written: {} (pid={})", pid_path.display(), pid);
    }

    let listener = match bind_with_retry(&addr, bind_notify).await {
        Some(l) => l,
        None => return,
    };

    if tls_config.enable {
        let tls_acceptor = build_tls_acceptor(&tls_config);

        let shutdown_fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            match shutdown_notify {
                Some(notify) => Box::pin(async move {
                    notify.notified().await;
                    tracing::info!("Gateway TLS shutdown requested via notify");
                }),
                None => Box::pin(shutdown_signal()),
            };

        // TLS accept loop: wrap each connection with TLS, then serve via hyper HTTP/1.1
        let tls_serve = async {
            tokio::pin!(shutdown_fut);
            loop {
                tokio::select! {
                    biased;
                    _ = &mut shutdown_fut => {
                        tracing::info!("Gateway TLS accept loop shutting down");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, peer)) => {
                                let acceptor = tls_acceptor.clone();
                                let app_clone = app.clone();
                                tokio::spawn(async move {
                                    match acceptor.accept(stream).await {
                                        Ok(tls_stream) => {
                                            let io = hyper_util::rt::TokioIo::new(tls_stream);
                                            let svc = hyper_util::service::TowerToHyperService::new(
                                                tls::ConnectInfoService::new(app_clone, peer),
                                            );
                                            if let Err(e) = hyper::server::conn::http1::Builder::new()
                                                .serve_connection(io, svc)
                                                .await
                                            {
                                                tracing::error!("TLS connection error from {}: {}", peer, e);
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("TLS handshake failed from {}: {}", peer, e);
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("Accept error on TLS listener: {}", e);
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
        };

        match tokio::time::timeout(
            std::time::Duration::from_secs(drain_timeout_secs),
            tls_serve,
        )
        .await
        {
            Ok(()) => {}
            Err(_elapsed) => {
                tracing::warn!(
                    "Graceful drain timed out after {drain_timeout_secs}s — forcing shutdown"
                );
            }
        }
        tracing::info!("Gateway TLS server exited");
    } else {
        let shutdown_fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            match shutdown_notify {
                Some(notify) => Box::pin(async move {
                    notify.notified().await;
                    tracing::info!("Gateway shutdown requested via notify");
                }),
                None => Box::pin(shutdown_signal()),
            };
        let server = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_fut);

        match tokio::time::timeout(std::time::Duration::from_secs(drain_timeout_secs), server).await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::error!("Gateway error: {e}");
            }
            Err(_elapsed) => {
                tracing::warn!(
                    "Graceful drain timed out after {drain_timeout_secs}s — forcing shutdown"
                );
            }
        }
        tracing::info!("Gateway server exited");
    }

    // Persist quota data on shutdown
    quota_for_shutdown.persist_sync();

    // Persist provider budget spend on shutdown
    provider_budgets_for_shutdown.persist_sync();

    // Stop all MCP server subprocesses
    mcp_for_shutdown.stop_all().await;

    // Clean up PID file on shutdown
    let _ = std::fs::remove_file(&pid_path);
}

/// Bind to `addr` with retry, signalling result via `bind_notify`.
/// Returns `Some(listener)` on success, `None` on failure (caller should return).
async fn bind_with_retry(
    addr: &str,
    bind_notify: Option<oneshot::Sender<Result<(), String>>>,
) -> Option<tokio::net::TcpListener> {
    const MAX_BIND_RETRIES: u32 = 10;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
    let mut last_err = String::new();
    for attempt in 1..=MAX_BIND_RETRIES {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => {
                if attempt > 1 {
                    tracing::info!("Gateway bound to {} after {} retries", addr, attempt - 1);
                }
                tracing::info!("Gateway listening on {}", addr);
                if let Some(tx) = bind_notify {
                    let _ = tx.send(Ok(()));
                }
                return Some(l);
            }
            Err(e) => {
                last_err = format!("{e}");
                if attempt < MAX_BIND_RETRIES {
                    tracing::warn!(
                        "Bind attempt {}/{} failed on {} — retrying in {}ms: {}",
                        attempt,
                        MAX_BIND_RETRIES,
                        addr,
                        RETRY_DELAY.as_millis(),
                        e
                    );
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }
    let msg = format!("Failed to bind gateway on {addr}: {last_err}");
    tracing::error!("{msg}");
    if let Some(tx) = bind_notify {
        let _ = tx.send(Err(msg));
    }
    None
}

/// Load TLS cert/key from config and build a TlsAcceptor.
/// Panics on startup-time misconfiguration (missing/unparseable cert or key).
fn build_tls_acceptor(tls_config: &config::TlsConfig) -> tokio_rustls::TlsAcceptor {
    let cert_path = tls::validate_tls_path(&tls_config.cert, "cert");
    let key_path = tls::validate_tls_path(&tls_config.key, "key");

    let cert_file = File::open(&cert_path).unwrap_or_else(|_| {
        panic!("TLS cert file cannot be opened");
    });
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|e| {
                tracing::error!("Failed to parse TLS certificate: {}", e);
                panic!("TLS cert parse error");
            });

    let key_file = File::open(&key_path).unwrap_or_else(|_| {
        panic!("TLS key file cannot be opened");
    });
    let mut key_reader = BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .unwrap_or_else(|e| {
            tracing::error!("Failed to parse TLS private key: {}", e);
            panic!("TLS key parse error");
        })
        .expect("No private key found in TLS key file");

    let tls_server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap_or_else(|e| {
            tracing::error!("Failed to build TLS config: {}", e);
            panic!("TLS config error: {}", e);
        });
    tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls_server_config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_state;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_endpoint_returns_200() {
        let state = build_test_state(vec![]);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_200() {
        let state = build_test_state(vec![]);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/metrics")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_channels_route_registered() {
        let state = build_test_state(vec![]);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/channels")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Should NOT be 404 — route is registered
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn admin_virtual_keys_route_registered() {
        let state = build_test_state(vec![]);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/virtual-keys")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let state = build_test_state(vec![]);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/nonexistent/path")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn versioned_admin_channels_route_registered() {
        let state = build_test_state(vec![]);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/api/channels")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Should NOT be 404 — versioned route is registered
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn versioned_and_unversioned_admin_routes_equivalent() {
        let state = build_test_state(vec![]);
        let state_copy = Arc::clone(&state);
        let app_unversioned = build_router(state, None);
        let app_versioned = build_router(state_copy, None);

        // GET /v1/api/channels returns same status as /api/channels
        let resp_unversioned = app_unversioned
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/channels")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        let resp_versioned = app_versioned
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/api/channels")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp_unversioned.status(), resp_versioned.status());

        // Verify response bodies match (same shared state)
        let body_unversioned = axum::body::to_bytes(resp_unversioned.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body_versioned = axum::body::to_bytes(resp_versioned.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            body_unversioned, body_versioned,
            "versioned and unversioned /api/channels bodies must match"
        );
    }

    #[tokio::test]
    async fn versioned_metrics_route_matches_unversioned() {
        let state = build_test_state(vec![]);
        let state_copy = Arc::clone(&state);
        let app_unversioned = build_router(state, None);
        let app_versioned = build_router(state_copy, None);

        let resp_unversioned = app_unversioned
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/metrics")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        let resp_versioned = app_versioned
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/metrics")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp_unversioned.status(), resp_versioned.status());

        let body_unversioned = axum::body::to_bytes(resp_unversioned.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body_versioned = axum::body::to_bytes(resp_versioned.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            body_unversioned, body_versioned,
            "versioned and unversioned /metrics bodies must match"
        );
    }

    // ----- TLS reload tests -------------------------------------------------

    use std::sync::atomic::{AtomicU64, Ordering};

    /// Counter used to generate unique temp file names per test process/run.
    static TLS_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Create a unique temp file under the system temp dir with the given
    /// extension. Cleans up on test failure is best-effort via explicit
    /// `remove_file` in each test.
    fn tls_test_temp_file(ext: &str) -> std::path::PathBuf {
        let id = TLS_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "modelswitch_test_tls_{}_{}.{}",
            std::process::id(),
            id,
            ext
        ));
        std::fs::write(&path, b"initial content").expect("create temp file");
        path
    }

    /// Bump a file's modification time forward so `check_cert_freshness` can
    /// detect a change without needing to `sleep` through filesystem mtime
    /// granularity (which is 1s on many filesystems).
    fn tls_bump_mtime(path: &std::path::Path) {
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
        let times = std::fs::FileTimes::new().set_modified(future);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file for set_times");
        f.set_times(times).expect("set file modification time");
    }

    #[test]
    fn tls_reload_state_initializes_correctly() {
        let cert = std::path::PathBuf::from("/nonexistent/cert.pem");
        let key = std::path::PathBuf::from("/nonexistent/key.pem");
        let state = TlsReloadState::new(cert.clone(), key.clone());
        assert_eq!(state.cert_path, cert);
        assert_eq!(state.key_path, key);
        assert!(
            state.last_modified.is_none(),
            "last_modified should start None"
        );
    }

    #[test]
    fn check_cert_freshness_no_change() {
        let cert = tls_test_temp_file("pem");
        let key = tls_test_temp_file("key");
        let mut state = TlsReloadState::new(cert.clone(), key.clone());

        // First call establishes baseline and must not flag a change.
        let first = check_cert_freshness(&mut state);
        assert!(!first, "first check should establish baseline (false)");

        // Second call with no modification must also be false.
        let second = check_cert_freshness(&mut state);
        assert!(
            !second,
            "no modification between checks should return false"
        );

        let _ = std::fs::remove_file(&cert);
        let _ = std::fs::remove_file(&key);
    }

    #[test]
    fn check_cert_freshness_detects_change() {
        let cert = tls_test_temp_file("pem");
        let key = tls_test_temp_file("key");
        let mut state = TlsReloadState::new(cert.clone(), key.clone());

        // Establish baseline.
        assert!(!check_cert_freshness(&mut state));

        // Bump the cert file's mtime forward.
        tls_bump_mtime(&cert);

        // Should now detect a change.
        let detected = check_cert_freshness(&mut state);
        assert!(detected, "modification should be detected");

        // A subsequent check with no further modification should return false.
        let again = check_cert_freshness(&mut state);
        assert!(!again, "no further modification should return false");

        // Modifying the key file should also be detected.
        tls_bump_mtime(&key);
        assert!(
            check_cert_freshness(&mut state),
            "key modification detected"
        );

        let _ = std::fs::remove_file(&cert);
        let _ = std::fs::remove_file(&key);
    }

    // ----- validate_tls_path tests -----------------------------------------

    /// RAII guard that removes a file when dropped — ensures cleanup even on panic.
    struct TempFileGuard(std::path::PathBuf);

    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn validate_tls_path_accepts_pem_cert() {
        let cert = tls_test_temp_file("pem");
        let _guard = TempFileGuard(cert.clone());
        let canonical = tls::validate_tls_path(cert.to_str().unwrap(), "cert");
        assert!(canonical.exists(), "canonical path should exist");
    }

    #[test]
    fn validate_tls_path_accepts_crt_cert() {
        let cert = tls_test_temp_file("crt");
        let _guard = TempFileGuard(cert.clone());
        let canonical = tls::validate_tls_path(cert.to_str().unwrap(), "cert");
        assert!(canonical.exists(), "canonical path should exist");
    }

    #[test]
    fn validate_tls_path_accepts_pem_key() {
        let key = tls_test_temp_file("pem");
        let _guard = TempFileGuard(key.clone());
        let canonical = tls::validate_tls_path(key.to_str().unwrap(), "key");
        assert!(canonical.exists(), "canonical path should exist");
    }

    #[test]
    fn validate_tls_path_accepts_key_extension() {
        let key = tls_test_temp_file("key");
        let _guard = TempFileGuard(key.clone());
        let canonical = tls::validate_tls_path(key.to_str().unwrap(), "key");
        assert!(canonical.exists(), "canonical path should exist");
    }

    #[test]
    #[should_panic(expected = "empty")]
    fn validate_tls_path_rejects_empty_path() {
        tls::validate_tls_path("", "cert");
    }

    #[test]
    #[should_panic(expected = "not found")]
    fn validate_tls_path_rejects_nonexistent_file() {
        tls::validate_tls_path("/nonexistent/path/cert.pem", "cert");
    }

    #[test]
    #[should_panic(expected = "extension")]
    fn validate_tls_path_rejects_wrong_extension_cert() {
        let cert = tls_test_temp_file("txt");
        let _guard = TempFileGuard(cert.clone());
        tls::validate_tls_path(cert.to_str().unwrap(), "cert");
    }

    #[test]
    #[should_panic(expected = "extension")]
    fn validate_tls_path_rejects_wrong_extension_key() {
        let key = tls_test_temp_file("txt");
        let _guard = TempFileGuard(key.clone());
        tls::validate_tls_path(key.to_str().unwrap(), "key");
    }

    // ----- build_cors_layer tests ------------------------------------------

    #[test]
    fn build_cors_layer_handles_none() {
        let _layer = routes::build_cors_layer(&None);
    }

    #[test]
    fn build_cors_layer_handles_empty_vec() {
        let _layer = routes::build_cors_layer(&Some(vec![]));
    }

    #[test]
    fn build_cors_layer_handles_valid_origins() {
        let _layer = routes::build_cors_layer(&Some(vec!["https://example.com".into()]));
    }

    #[test]
    fn build_cors_layer_handles_invalid_origin() {
        let _layer = routes::build_cors_layer(&Some(vec!["not a url".into()]));
    }

    // ----- check_cert_freshness edge case ----------------------------------

    #[test]
    fn check_cert_freshness_missing_files_returns_false() {
        let cert = std::path::PathBuf::from("/nonexistent/cert.pem");
        let key = std::path::PathBuf::from("/nonexistent/key.pem");
        let mut state = TlsReloadState::new(cert, key);

        let changed = check_cert_freshness(&mut state);
        assert!(!changed, "missing files should not be flagged as changed");
        assert!(
            state.last_modified.is_none(),
            "last_modified should remain None when both files are missing"
        );
    }
}
