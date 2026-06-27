//! Gateway server bootstrap: services initialization, Axum router, and start.

mod routes;
mod services;
mod tls;

// Re-export public API for backward compatibility.
pub use tls::{check_cert_freshness, start_tls_reload_watcher, TlsReloadState};
// Re-export so existing callers (and tests in this file) can reach it.
pub use routes::build_router;

use crate::admin::audit::AuditLog;
use crate::channel::manager::ChannelManager;
use crate::config;
use crate::config::AppConfig;
use crate::credential::create_credential_store;
use crate::guardrails::GuardrailsChecker;
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::model_registry::ModelRegistry;
use crate::notification::NotificationService;
use crate::provider_budget::ProviderBudgetStore;
use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::{
    AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
    SecurityState,
};
use crate::quota::{QuotaStore, RedemptionCodeStore};
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::shutdown::shutdown_signal;
use crate::spawn_bg;
use crate::virtual_key::VirtualKeyStore;
use crate::GatewayHandles;

use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};

/// Build gateway state and start background services.
/// Callable from both Tauri setup and CLI mode.
/// If `config_path` is provided, loads config from that path instead of the default.
pub fn start_gateway_services(config_path: Option<std::path::PathBuf>) -> GatewayHandles {
    let watcher_config_path = config_path.clone();
    let (config, http_pool) = build_infra(config_path);

    let port = config.gateway.port;
    let host = config.gateway.host.clone();
    let max_retries = config.gateway.max_retries;
    let model_fallbacks = config.gateway.model_fallbacks.clone();
    let context_window_fallbacks = config.gateway.context_window_fallbacks.clone();
    let model_aliases = config.gateway.model_aliases.clone();
    let routing_strategy = config.gateway.routing_strategy;

    // Initialize channel manager and logger
    let credential_store = create_credential_store();
    let channel_mgr = Arc::new(ChannelManager::new(&config, Arc::clone(&credential_store)));
    let log_file = config::app_config_dir().join("logs.ndjson");

    // Run data migrations before loading stores — old data files from
    // previous versions are upgraded in-place with a `.bak` backup.
    let config_dir = config::app_config_dir();
    if let Err(e) = crate::migration::migrate_data_file(&config_dir.join("virtual_keys.json")) {
        tracing::error!("Failed to migrate virtual_keys.json: {e}");
    }
    if let Err(e) = crate::migration::migrate_data_file(&config_dir.join("audit.ndjson")) {
        tracing::error!("Failed to migrate audit.ndjson: {e}");
    }

    let logger = Arc::new(DispatchLogger::with_persistence(
        config.gateway.log_max_entries,
        log_file,
        config.gateway.log_max_file_size_mb,
        config.gateway.log_max_files,
    ));

    // Build shared components
    let active_requests = Arc::new(ActiveRequests::new());
    let latency_tracker = Arc::new(crate::router::latency_tracker::LatencyTracker::new());
    let cache_mode = CacheMode::parse_mode(&config.gateway.cache_mode);
    tracing::info!(cache_mode = ?cache_mode, "Request cache mode");
    let request_cache = Arc::new(RequestCache::new(
        std::time::Duration::from_secs(config.gateway.cache_ttl_secs),
        config.gateway.max_cache_entries,
        cache_mode,
    ));
    let payload_rules = Arc::new(ChannelPayloadRules::new());
    let rate_limiter = Arc::new(RateLimiter::new(None));
    let quota_store = Arc::new(QuotaStore::new());
    let virtual_key_store = Arc::new(VirtualKeyStore::new());
    let provider_budget_store = Arc::new(ProviderBudgetStore::new());
    let in_flight = Arc::new(InFlightRequests::new());

    // Build MCP manager and load server configs (does NOT auto-start servers)
    let mcp_manager = Arc::new(McpManager::new());
    let mcp_configs = config.mcp_servers.clone();
    let mcp_mgr_for_load = Arc::clone(&mcp_manager);
    spawn_bg(async move {
        mcp_mgr_for_load.load_configs(&mcp_configs).await;
    });

    // Resolve admin token: env > config
    let admin_token = std::env::var("MODELSWITCH_ADMIN_TOKEN")
        .ok()
        .or(config.gateway.admin_token.clone());

    // Resolve role-based admin tokens for RBAC.
    let admin_roles: Vec<(String, crate::middleware::rbac::Role)> = config
        .gateway
        .admin_tokens
        .iter()
        .filter_map(|entry| {
            crate::middleware::rbac::Role::parse_role(&entry.role)
                .map(|role| (entry.token.clone(), role))
        })
        .collect();

    // Configure per-channel rate limits and payload rules from config
    for ch in &config.channels {
        let id = match uuid::Uuid::parse_str(&ch.id) {
            Ok(id) => id,
            Err(_) => continue,
        };
        if let Some(rpm) = ch.rpm_limit {
            rate_limiter.set_channel_rpm_limit(id, rpm);
        }
        if let Some(tpm) = ch.tpm_limit {
            rate_limiter.set_channel_tpm_limit(id, tpm);
        }
        if let Some(ref rules) = ch.payload_rules {
            use crate::proxy::payload_rules::PayloadRules;
            payload_rules.add(
                id,
                PayloadRules {
                    defaults: rules.defaults.clone(),
                    overrides: rules.overrides.clone(),
                    strip: rules.strip.clone(),
                },
            );
            if !rules.model_rules.is_empty() {
                payload_rules.set_model_rules(id, rules.model_rules.clone());
            }
        }
    }

    // Populate per-provider budget limits from config
    let pb_store = Arc::clone(&provider_budget_store);
    let pb_configs = config.gateway.provider_budgets.clone();
    spawn_bg(async move {
        for (provider, budget_config) in &pb_configs {
            pb_store.set_budget(provider, budget_config.clone()).await;
        }
        if !pb_configs.is_empty() {
            tracing::info!(
                count = pb_configs.len(),
                "Loaded provider budget limits from config"
            );
        }
    });

    // Audit log: ring buffer + NDJSON persistence (survives restarts).
    let audit_log_file = config::app_config_dir().join("audit.ndjson");
    let audit_log = Arc::new(AuditLog::with_default_capacity_and_persistence(
        audit_log_file,
    ));

    // Build shared state (used by all proxy handlers including Gemini)
    let state = Arc::new(AppState {
        channel_mgr: Arc::clone(&channel_mgr),
        credential_store,
        logger: Arc::clone(&logger),
        audit_log: Arc::clone(&audit_log),
        http_pool: http_pool.clone(),
        model_registry: Arc::new(parking_lot::RwLock::new(ModelRegistry::new())),
        gateway: ProxyParams {
            request_timeout_secs: config.gateway.request_timeout_secs,
            stream_keepalive_secs: config.gateway.stream_keepalive_secs,
            stream_ttft_timeout_secs: config.gateway.stream_ttft_timeout_secs,
            max_retries,
            model_fallbacks,
            context_window_fallbacks,
            model_aliases,
            routing_strategy,
            retry_base_ms: config.gateway.retry_base_ms,
            retry_max_ms: config.gateway.retry_max_ms,
            model_retry_overrides: config.gateway.model_retry_overrides.clone(),
            nonstream_keepalive_interval_secs: config.gateway.nonstream_keepalive_interval_secs,
            passthrough_headers: config.gateway.effective_passthrough_headers(),
            stream_bootstrap_retries: config.gateway.stream_bootstrap_retries,
            disable_image_generation: config.gateway.disable_image_generation,
            model_groups: config.gateway.model_groups.clone(),
            model_pricing: config.gateway.model_pricing.clone(),
            completion_ratios: config.gateway.completion_ratios.clone(),
        },
        router: RouterState {
            session_affinity: SessionAffinity::default(),
            active_requests: Arc::clone(&active_requests),
            latency_tracker: Arc::clone(&latency_tracker),
            cooldown_tracker: Arc::new(crate::router::cooldown::CooldownTracker::new()),
        },
        cache: CacheState {
            request_cache: Arc::clone(&request_cache),
            in_flight: Arc::clone(&in_flight),
        },
        limits: LimitsState {
            payload_rules: Arc::clone(&payload_rules),
            rate_limiter: Arc::clone(&rate_limiter),
        },
        billing: BillingState {
            quota_store: Arc::clone(&quota_store),
            virtual_key_store: Arc::clone(&virtual_key_store),
            provider_budgets: Arc::clone(&provider_budget_store),
            key_rate_limiter: Arc::new(crate::proxy::rate_limiter::KeyRateLimiter::new()),
        },
        mcp: McpState {
            mcp_manager: Arc::clone(&mcp_manager),
            mcp_max_iterations: config.gateway.mcp_max_iterations,
            mcp_auto_inject: config.gateway.mcp_auto_inject,
            mcp_gateway_enabled: config.gateway.mcp_gateway_enabled,
        },
        security: SecurityState {
            admin_token,
            admin_roles,
            sanitizer_config: config.gateway.sanitizer.clone(),
            allowed_origins: config.gateway.allowed_origins.clone(),
            trust_forwarded_headers: false,
        },
        guardrails: Arc::new(GuardrailsChecker::new(
            crate::guardrails::GuardrailsConfig::default(),
        )),
        redemption_codes: Arc::new(RedemptionCodeStore::new()),
        notifications: Arc::new(NotificationService::new(
            config.gateway.notification.clone(),
        )),
        completion_ratios: Arc::new(parking_lot::RwLock::new(
            config.gateway.completion_ratios.clone(),
        )),
        ldap_config: config.gateway.auth.ldap.clone(),
        started_at: std::time::Instant::now(),
    });

    services::spawn_persistence_tasks(&state);

    services::spawn_background_services(&state, &config);

    services::spawn_config_watcher(&state, &watcher_config_path);
    GatewayHandles {
        state,
        host,
        port,
        drain_timeout_secs: config.gateway.drain_timeout_secs,
        web_console_dir: config.gateway.web_console_dir.clone(),
        tls: config.gateway.tls.clone(),
    }
}

/// Initialize tracing, load config, and build the HTTP connection pool.
fn build_infra(config_path: Option<std::path::PathBuf>) -> (AppConfig, crate::http_pool::HttpPool) {
    // Initialize tracing (no-op if already initialized, e.g. by CLI main)
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .try_init();

    // Log OTLP status if configured
    crate::telemetry::init_tracing();

    // Load config
    let config = match config_path {
        Some(path) => AppConfig::load_from(path).unwrap_or_else(|e| {
            eprintln!("Failed to load config: {e}");
            std::process::exit(1);
        }),
        None => AppConfig::load().unwrap_or_default(),
    };

    // Build HTTP connection pool — multiple reqwest::Client instances to work around
    // HTTP/2 single-connection-per-host limits under high concurrency.
    let pool_size = config.gateway.http_pool_size.max(1);
    let http_pool = crate::http_pool::HttpPool::new(pool_size, || {
        reqwest::Client::builder()
            // Overall request timeout (includes connect, write, read). Default 120s.
            // This is a safety net — per-read and per-connect overrides are tighter.
            .timeout(std::time::Duration::from_secs(
                config.gateway.http_timeout_secs,
            ))
            // Connect timeout: 10s is generous enough for cloud LLM APIs without
            // holding a connection slot too long on a dead upstream.
            .connect_timeout(std::time::Duration::from_secs(10))
            // Per-read timeout: 300s (5 min) ensures a single stalled byte does not
            // tie up a connection forever; the SSE stream-layer timeout (120s) fires
            // first for streaming paths.
            .read_timeout(std::time::Duration::from_secs(300))
            // Idle connections are kept alive for 90s before being closed by the
            // client. Balances reconnection cost vs. holding idle resources.
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            // Keep up to 20 idle connections per host to absorb traffic bursts
            // without reopening connections (reqwest default is pool_max_idle_per_host).
            .pool_max_idle_per_host(20)
            // TCP keepalive at 60s to detect dead upstream connections early and
            // avoid hanging requests on half-open sockets.
            .tcp_keepalive(std::time::Duration::from_secs(60))
            // Disable Nagle's algorithm for reduced latency on small request
            // payloads (chat completions are frequently sub-MTU).
            .tcp_nodelay(true)
    })
    .expect("Failed to build HTTP client pool");
    tracing::info!(pool_size, "HTTP connection pool created");

    (config, http_pool)
}

/// Start the Axum gateway server with graceful shutdown.
/// Binds to `host:port` and drains in-flight requests on SIGINT/SIGTERM.
/// Drain is bounded by `drain_timeout_secs` to prevent hanging.
/// If `shutdown_notify` is provided, uses it instead of signal-based shutdown.
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

    let listener = {
        const MAX_BIND_RETRIES: u32 = 10;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
        let mut last_err = String::new();
        let mut bound = None;
        for attempt in 1..=MAX_BIND_RETRIES {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => {
                    if attempt > 1 {
                        tracing::info!("Gateway bound to {} after {} retries", addr, attempt - 1);
                    }
                    tracing::info!("Gateway listening on {}", addr);
                    bound = Some(l);
                    break;
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
        match bound {
            Some(l) => {
                if let Some(tx) = bind_notify {
                    let _ = tx.send(Ok(()));
                }
                l
            }
            None => {
                let msg = format!("Failed to bind gateway on {addr}: {last_err}");
                tracing::error!("{msg}");
                if let Some(tx) = bind_notify {
                    let _ = tx.send(Err(msg));
                }
                return;
            }
        }
    };

    if tls_config.enable {
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
        let tls_acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls_server_config));

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
