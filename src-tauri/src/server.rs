//! Gateway server bootstrap: services initialization, Axum router, and start.

use crate::admin;
use crate::channel::manager::ChannelManager;
use crate::config;
use crate::config::AppConfig;
use crate::credential::create_credential_store;
use crate::health;
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::middleware;
use crate::proxy;
use crate::proxy::cache::{InFlightRequests, RequestCache};
use crate::proxy::openai::{
    AppState, BillingState, CacheState, GatewayParams, LimitsState, McpState, RouterState,
    SecurityState,
};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::quota;
use crate::quota::registry::QuotaProviderRegistry;
use crate::quota::QuotaStore;
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::shutdown::shutdown_signal;
use crate::spawn_bg;
use crate::virtual_key::VirtualKeyStore;
use crate::GatewayHandles;

use axum::routing::{delete, get, post, put};
use axum::Router;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Build gateway state and start background services.
/// Callable from both Tauri setup and CLI mode.
/// If `config_path` is provided, loads config from that path instead of the default.
pub fn start_gateway_services(config_path: Option<std::path::PathBuf>) -> GatewayHandles {
    // Initialize tracing (no-op if already initialized, e.g. by CLI main)
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .try_init();

    // Load config
    let watcher_config_path = config_path.clone();
    let config = match config_path {
        Some(path) => AppConfig::load_from(path).unwrap_or_else(|e| {
            eprintln!("Failed to load config: {e}");
            std::process::exit(1);
        }),
        None => AppConfig::load().unwrap_or_default(),
    };
    let port = config.gateway.port;
    let host = config.gateway.host.clone();
    let max_retries = config.gateway.max_retries;
    let model_fallbacks = config.gateway.model_fallbacks.clone();
    let routing_strategy = config.gateway.routing_strategy.clone();

    // Build shared HTTP client (connection pooling)
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            config.gateway.http_timeout_secs,
        ))
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(20)
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .tcp_nodelay(true)
        .build()
        .expect("Failed to build HTTP client");

    // Initialize channel manager and logger
    let credential_store = create_credential_store();
    let channel_mgr = Arc::new(ChannelManager::new(&config, Arc::clone(&credential_store)));
    let log_file = config::app_config_dir().join("logs.ndjson");
    let logger = Arc::new(DispatchLogger::with_persistence(
        config.gateway.log_max_entries,
        log_file,
    ));

    // Build shared components
    let active_requests = Arc::new(ActiveRequests::new());
    let request_cache = Arc::new(RequestCache::new(
        std::time::Duration::from_secs(config.gateway.cache_ttl_secs),
        config.gateway.max_cache_entries,
    ));
    let payload_rules = Arc::new(ChannelPayloadRules::new());
    let rate_limiter = Arc::new(RateLimiter::new(None));
    let quota_store = Arc::new(QuotaStore::new());
    let virtual_key_store = Arc::new(VirtualKeyStore::new());
    let quota_registry = Arc::new(QuotaProviderRegistry::new(
        quota::collectors::default_registry(),
    ));
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
        }
    }

    // Build shared state (used by all proxy handlers including Gemini)
    let state = Arc::new(AppState {
        channel_mgr: Arc::clone(&channel_mgr),
        credential_store,
        logger: Arc::clone(&logger),
        http_client: http_client.clone(),
        gateway: GatewayParams {
            request_timeout_secs: config.gateway.request_timeout_secs,
            stream_keepalive_secs: config.gateway.stream_keepalive_secs,
            max_retries,
            model_fallbacks,
            routing_strategy,
        },
        router: RouterState {
            session_affinity: SessionAffinity::default(),
            active_requests: Arc::clone(&active_requests),
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
        },
        mcp: McpState {
            mcp_manager: Arc::clone(&mcp_manager),
            mcp_max_iterations: config.gateway.mcp_max_iterations,
            mcp_auto_inject: config.gateway.mcp_auto_inject,
            mcp_gateway_enabled: config.gateway.mcp_gateway_enabled,
        },
        security: SecurityState {
            admin_token,
            sanitizer_config: config.gateway.sanitizer.clone(),
        },
        started_at: std::time::Instant::now(),
    });

    // Load persisted dispatch logs at startup
    let boot_logger = Arc::clone(&logger);
    spawn_bg(async move {
        boot_logger.load_from_file().await;
    });

    // Load persisted quota data (token usage survives restarts)
    {
        let boot_quota = Arc::clone(&quota_store);
        spawn_bg(async move {
            boot_quota.load_from_file().await;
        });
    }

    // Periodic quota persistence (every 60s)
    {
        let persist_quota = Arc::clone(&quota_store);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                persist_quota.persist_to_file().await;
            }
        });
    }

    // Load persisted virtual keys (so keys + spend survive restarts)
    {
        let boot_vk = Arc::clone(&virtual_key_store);
        spawn_bg(async move {
            if let Err(e) = boot_vk.load().await {
                tracing::warn!(error = %e, "Failed to load virtual keys");
            } else {
                let count = boot_vk.list().await.len();
                tracing::info!(count, "Loaded virtual keys from disk");
            }
        });
    }

    // Periodic virtual key persistence (every 60s)
    {
        let persist_vk = Arc::clone(&virtual_key_store);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Err(e) = persist_vk.persist().await {
                    tracing::warn!(error = %e, "Failed to persist virtual keys");
                }
            }
        });
    }

    // Start background health checker
    if config.gateway.health_check_enabled {
        let hc_mgr = Arc::clone(&channel_mgr);
        let hc_client = http_client.clone();
        let hc_interval = config.gateway.health_check_interval_secs;
        health::start_health_checker(hc_mgr, hc_interval, hc_client);
    }

    // Start background quota poller
    {
        let qp_mgr = Arc::clone(&channel_mgr);
        let qp_store = Arc::clone(&quota_store);
        let qp_client = http_client.clone();
        let qp_interval = config.gateway.quota_poll_interval_secs;
        let qp_registry = Arc::clone(&quota_registry);
        // Build per-channel quota config map
        let qp_configs: std::collections::HashMap<String, config::QuotaConfig> = config
            .channels
            .iter()
            .filter_map(|ch| ch.quota.as_ref().map(|q| (ch.id.clone(), q.clone())))
            .collect();
        spawn_bg(async move {
            quota::poller::start_quota_poller(
                qp_mgr,
                qp_store,
                qp_client,
                qp_interval,
                qp_registry,
                qp_configs,
            )
            .await;
        });
    }

    // Periodic session affinity cleanup
    {
        let affinity_cleanup = state.router.session_affinity.clone();
        spawn_bg(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                affinity_cleanup.cleanup().await;
            }
        });
    }

    // Start hot config reload watcher
    let watcher_path = watcher_config_path.or_else(|| AppConfig::config_path().ok());
    if let Some(path) = watcher_path {
        config::watcher::start_config_watcher(
            path,
            Arc::clone(&channel_mgr),
            Arc::clone(&mcp_manager),
            Arc::clone(&rate_limiter),
            Arc::clone(&payload_rules),
        );
    }
    GatewayHandles {
        state,
        host,
        port,
        drain_timeout_secs: config.gateway.drain_timeout_secs,
    }
}

/// Build the Axum Router with all proxy and admin routes.
/// Proxy routes use optional virtual-key auth (pass-through when no keys configured);
/// admin routes use optional Bearer token auth.
pub fn build_router(state: Arc<AppState>) -> Router {
    let proxy_state = Arc::clone(&state);
    let proxy_auth_state = Arc::clone(&state);
    let sanitizer_state = Arc::clone(&state);
    let admin_route_state = Arc::clone(&state);
    let admin_auth_state = Arc::clone(&state);

    // Proxy routes -- virtual-key auth (pass-through when no virtual keys configured)
    let proxy_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(proxy::openai::handle_chat_completions),
        )
        .route("/v1/models", get(proxy::openai::handle_list_models))
        .route("/v1/tools", get(proxy::openai::handle_list_tools))
        .route("/v1/messages", post(proxy::anthropic::handle_messages))
        .route(
            "/v1beta/models/{*path}",
            post(proxy::gemini::handle_gemini),
        )
        .route("/health", get(proxy::openai::health_check))
        // Claude Code Protocol -- provider-prefixed routes for agentic tools
        .route(
            "/api/provider/{provider}/v1/chat/completions",
            post(proxy::openai::handle_chat_completions),
        )
        .route(
            "/api/provider/{provider}/v1/messages",
            post(proxy::anthropic::handle_messages),
        )
        .route(
            "/api/provider/{provider}/v1/models",
            get(proxy::openai::handle_list_models),
        )
        .layer(axum::middleware::from_fn_with_state(
            proxy_auth_state,
            middleware::virtual_key::virtual_key_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            sanitizer_state,
            middleware::sanitizer::sanitizer_middleware,
        ))
        .with_state(proxy_state);

    // Admin routes -- optional Bearer token auth
    let admin_router = Router::new()
        .route("/api/channels", get(admin::list_channels))
        .route("/api/channels", post(admin::create_channel))
        .route("/api/channels/{id}", put(admin::update_channel))
        .route("/api/channels/{id}", delete(admin::delete_channel))
        .route("/api/channels/{id}/ping", post(admin::ping_channel))
        .route("/api/channels/{id}/status", get(admin::channel_status))
        .route("/api/logs", get(admin::get_logs))
        .route("/api/stats", get(admin::get_stats))
        .route("/api/stats/cost", get(admin::get_cost_stats))
        .route("/api/stats/usage", get(admin::get_usage_history))
        .route("/api/quota", get(admin::get_quota))
        .route("/api/auth/cookies", post(admin::receive_login_cookies))
        .route("/api/auth/pending-cookies", get(admin::get_pending_cookies))
        .route(
            "/api/channels/{id}/reset-circuit",
            post(admin::reset_circuit),
        )
        .route(
            "/api/channels/{id}/payload-rules",
            put(admin::set_payload_rules),
        )
        .route("/api/cache/flush", post(admin::flush_cache))
        .route("/api/config/reload", post(admin::reload_config))
        .route("/api/mcp/servers", get(admin::list_mcp_servers))
        .route("/api/mcp/servers", post(admin::create_mcp_server))
        .route("/api/mcp/servers/{id}", put(admin::update_mcp_server))
        .route("/api/mcp/servers/{id}", delete(admin::delete_mcp_server))
        .route(
            "/api/mcp/servers/{id}/start",
            post(admin::start_mcp_server),
        )
        .route(
            "/api/mcp/servers/{id}/stop",
            post(admin::stop_mcp_server),
        )
        .route(
            "/api/mcp/servers/{id}/tools",
            get(admin::list_mcp_server_tools),
        )
        .route("/api/mcp/tools", get(admin::list_all_mcp_tools))
        .route("/api/virtual-keys", get(admin::list_virtual_keys))
        .route("/api/virtual-keys", post(admin::create_virtual_key))
        .route("/api/virtual-keys/{id}", put(admin::update_virtual_key))
        .route(
            "/api/virtual-keys/{id}",
            delete(admin::delete_virtual_key),
        )
        .with_state(admin_route_state)
        .layer(axum::middleware::from_fn_with_state(
            admin_auth_state,
            middleware::auth::admin_auth_middleware,
        ));

    let base_router = Router::new().merge(proxy_router).merge(admin_router);

    // Conditionally mount MCP Gateway Mode endpoint.
    let router = if state.mcp.mcp_gateway_enabled {
        use crate::mcp::McpGatewayHandler;
        use rmcp::transport::streamable_http_server::{
            session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
        };
        let mcp_manager = Arc::clone(&state.mcp.mcp_manager);
        let service: StreamableHttpService<McpGatewayHandler, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(McpGatewayHandler::new(Arc::clone(&mcp_manager))),
                Arc::new(LocalSessionManager::default()),
                StreamableHttpServerConfig::default(),
            );
        base_router.nest_service("/mcp", service)
    } else {
        base_router
    };

    router
        .layer(axum::middleware::from_fn(
            middleware::request_id::request_id_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024))
}

/// Start the Axum gateway server with graceful shutdown.
/// Binds to `host:port` and drains in-flight requests on SIGINT/SIGTERM.
/// Drain is bounded by `drain_timeout_secs` to prevent hanging.
/// If `shutdown_notify` is provided, uses it instead of signal-based shutdown.
pub async fn start_gateway(
    state: Arc<AppState>,
    host: &str,
    port: u16,
    _drain_timeout_secs: u64,
    shutdown_notify: Option<Arc<Notify>>,
    bind_notify: Option<oneshot::Sender<Result<(), String>>>,
) {
    let app = build_router(state.clone());
    let quota_for_shutdown = Arc::clone(&state.billing.quota_store);
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

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => {
            tracing::info!("Gateway listening on {}", addr);
            if let Some(tx) = bind_notify {
                let _ = tx.send(Ok(()));
            }
            l
        }
        Err(e) => {
            let msg = format!("Failed to bind gateway on {addr}: {e}");
            tracing::error!("{msg}");
            if let Some(tx) = bind_notify {
                let _ = tx.send(Err(msg));
            }
            return;
        }
    };

    let shutdown_fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        match shutdown_notify {
            Some(notify) => Box::pin(async move {
                notify.notified().await;
                tracing::info!("Gateway shutdown requested via notify");
            }),
            None => Box::pin(shutdown_signal()),
        };
    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_fut);

    if let Err(e) = server.await {
        tracing::error!("Gateway error: {e}");
    }
    tracing::info!("Gateway server exited");

    // Persist quota data on shutdown
    quota_for_shutdown.persist_sync();

    // Stop all MCP server subprocesses
    mcp_for_shutdown.stop_all().await;

    // Clean up PID file on shutdown
    let _ = std::fs::remove_file(&pid_path);
}
