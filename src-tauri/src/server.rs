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
use crate::model_registry::{refresh_from_endpoint, ModelRegistry};
use crate::provider_budget::ProviderBudgetStore;
use crate::proxy;
use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
use crate::proxy::payload_rules::ChannelPayloadRules;
use crate::proxy::rate_limiter::RateLimiter;
use crate::proxy::{
    AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
    SecurityState,
};
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
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio::sync::{oneshot, Notify};
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

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
    let logger = Arc::new(DispatchLogger::with_persistence(
        config.gateway.log_max_entries,
        log_file,
        config.gateway.log_max_file_size_mb,
        config.gateway.log_max_files,
    ));

    // Build shared components
    let active_requests = Arc::new(ActiveRequests::new());
    let latency_tracker = Arc::new(crate::router::latency_tracker::LatencyTracker::new());
    let cache_mode = CacheMode::from_str(&config.gateway.cache_mode);
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

    // Build shared state (used by all proxy handlers including Gemini)
    let state = Arc::new(AppState {
        channel_mgr: Arc::clone(&channel_mgr),
        credential_store,
        logger: Arc::clone(&logger),
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
        },
        router: RouterState {
            session_affinity: SessionAffinity::default(),
            active_requests: Arc::clone(&active_requests),
            latency_tracker: Arc::clone(&latency_tracker),
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
            allowed_origins: config.gateway.allowed_origins.clone(),
        },
        started_at: std::time::Instant::now(),
    });

    spawn_persistence_tasks(&state);

    spawn_background_services(&state, &config);

    spawn_config_watcher(&state, &watcher_config_path);
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

/// Spawn tasks that load persisted state and periodically save it to disk.
fn spawn_persistence_tasks(state: &Arc<AppState>) {
    // Load persisted dispatch logs at startup
    let boot_logger = Arc::clone(&state.logger);
    spawn_bg(async move {
        boot_logger.load_from_file().await;
    });

    // Load persisted quota data (token usage survives restarts)
    {
        let boot_quota = Arc::clone(&state.billing.quota_store);
        spawn_bg(async move {
            boot_quota.load_from_file().await;
        });
    }

    // Periodic quota persistence (every 60s)
    {
        let persist_quota = Arc::clone(&state.billing.quota_store);
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
        let boot_vk = Arc::clone(&state.billing.virtual_key_store);
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
        let persist_vk = Arc::clone(&state.billing.virtual_key_store);
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

    // Load persisted provider budget spend (so limits + spend survive restarts)
    {
        let boot_pb = Arc::clone(&state.billing.provider_budgets);
        spawn_bg(async move {
            if let Err(e) = boot_pb.load().await {
                tracing::warn!(error = %e, "Failed to load provider budgets");
            } else {
                tracing::info!("Loaded provider budget spend from disk");
            }
        });
    }

    // Periodic provider budget persistence (every 60s)
    {
        let persist_pb = Arc::clone(&state.billing.provider_budgets);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Err(e) = persist_pb.persist().await {
                    tracing::warn!(error = %e, "Failed to persist provider budgets");
                }
            }
        });
    }
}

/// Spawn background services: health checker, quota poller, session affinity cleanup, cache sweep.
fn spawn_background_services(state: &Arc<AppState>, config: &AppConfig) {
    // Start background health checker
    if config.gateway.health_check_enabled {
        let hc_mgr = Arc::clone(&state.channel_mgr);
        let hc_client = state.http_pool.first().clone();
        let hc_interval = config.gateway.health_check_interval_secs;
        health::start_health_checker(hc_mgr, hc_interval, hc_client);
    }

    // Start background quota poller
    {
        let qp_mgr = Arc::clone(&state.channel_mgr);
        let qp_store = Arc::clone(&state.billing.quota_store);
        let qp_client = state.http_pool.first().clone();
        let qp_interval = config.gateway.quota_poll_interval_secs;
        let qp_registry = Arc::new(QuotaProviderRegistry::new(
            quota::collectors::default_registry(),
        ));
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

    // Periodic cache sweep — bulk-evict expired entries every 60s so that
    // `get()` only needs a lazy per-key TTL check.
    {
        let sweep_cache = Arc::clone(&state.cache.request_cache);
        spawn_bg(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                sweep_cache.sweep_expired();
            }
        });
    }

    // Periodic cleanup of expired per-model cooldowns (every 5 minutes)
    {
        let channel_mgr_cleanup = Arc::clone(&state.channel_mgr);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await; // Skip first immediate tick
            loop {
                interval.tick().await;
                channel_mgr_cleanup.clean_expired_model_cooldowns().await;
            }
        });
    }

    // Start background model discovery for channels with models_endpoint configured
    {
        let channel_mgr = Arc::clone(&state.channel_mgr);
        let registry = Arc::clone(&state.model_registry);
        spawn_bg(async move {
            run_model_discovery(channel_mgr, registry).await;
        });
    }
}

/// Start the hot config reload watcher.
fn spawn_config_watcher(state: &Arc<AppState>, watcher_config_path: &Option<std::path::PathBuf>) {
    // Start hot config reload watcher
    let watcher_path = watcher_config_path
        .clone()
        .or_else(|| AppConfig::config_path().ok());
    if let Some(path) = watcher_path {
        config::watcher::start_config_watcher(
            path,
            Arc::clone(&state.channel_mgr),
            Arc::clone(&state.mcp.mcp_manager),
            Arc::clone(&state.limits.rate_limiter),
            Arc::clone(&state.limits.payload_rules),
        );
    }
}

/// Background model discovery for channels with `models_endpoint` configured.
///
/// Scans all channels on startup, finds those with a configured endpoint, and
/// spawns a per-channel tokio task that periodically fetches available models
/// and updates the shared registry.
async fn run_model_discovery(
    channel_mgr: Arc<ChannelManager>,
    registry: Arc<parking_lot::RwLock<ModelRegistry>>,
) {
    use uuid::Uuid;

    // Collect channels with models_endpoint
    let configs: Vec<(Uuid, String, u64)> = {
        let channels = channel_mgr.channels();
        let guard = channels.read().await;
        guard
            .values()
            .filter_map(|ch_arc| {
                let ch = ch_arc.read();
                ch.models_endpoint
                    .as_ref()
                    .map(|ep| (ch.id, ep.clone(), ch.models_refresh_interval_secs))
            })
            .collect()
    };

    for (channel_id, endpoint, interval_secs) in configs {
        let mgr = Arc::clone(&channel_mgr);
        let reg = Arc::clone(&registry);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            // First tick completes immediately, subsequent ticks wait for the interval
            loop {
                ticker.tick().await;
                let api_key = mgr.get_credential(channel_id).await;
                if let Some(key) = api_key {
                    match refresh_from_endpoint(&endpoint, &key).await {
                        Ok(models) => {
                            tracing::info!(
                                channel_id = %channel_id,
                                count = models.len(),
                                "Discovered {} models from endpoint {}",
                                models.len(),
                                endpoint,
                            );
                            reg.write().update_models(models, &endpoint);
                        }
                        Err(e) => {
                            tracing::warn!(
                                channel_id = %channel_id,
                                error = %e,
                                "Failed to refresh models from endpoint {}",
                                endpoint,
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        channel_id = %channel_id,
                        "No API key found for model discovery",
                    );
                }
            }
        });
    }
}

/// Build the Axum Router with all proxy and admin routes.
/// Proxy routes use optional virtual-key auth (pass-through when no keys configured);
/// admin routes use optional Bearer token auth.
pub fn build_router(state: Arc<AppState>, web_console_dir: Option<&str>) -> Router {
    // Warn loudly when web console is active without admin_token protection.
    if state.security.admin_token.is_none() && web_console_dir.is_some() {
        tracing::warn!("==========================================================");
        tracing::warn!("  WARNING: Web console is active but admin_token is");
        tracing::warn!("    not set. All admin endpoints are OPEN to the network.");
        tracing::warn!("    Set [security] admin_token in config.toml immediately.");
        tracing::warn!("==========================================================");
    }

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
        .route("/v1/responses", post(proxy::responses::handle_responses))
        .route("/v1/embeddings", post(proxy::embeddings::handle_embeddings))
        .route(
            "/v1/images/generations",
            post(proxy::images::handle_image_generation),
        )
        .route(
            "/v1/images/edits",
            post(proxy::images::handle_image_edits),
        )
        .route("/v1/models", get(proxy::openai::handle_list_models))
        .route(
            "/v1/models/{model_id}",
            get(proxy::openai::handle_get_model),
        )
        .route("/v1/tools", get(proxy::openai::handle_list_tools))
        .route("/v1/messages", post(proxy::anthropic::handle_messages))
        .route("/v1beta/models/{*path}", post(proxy::gemini::handle_gemini))
        .route("/health", get(proxy::openai::health_check))
        // Claude Code Protocol -- provider-prefixed routes for agentic tools
        .route(
            "/api/provider/{provider}/v1/chat/completions",
            post(proxy::openai::handle_chat_completions),
        )
        .route(
            "/api/provider/{provider}/v1/embeddings",
            post(proxy::embeddings::handle_embeddings),
        )
        .route(
            "/api/provider/{provider}/v1/messages",
            post(proxy::anthropic::handle_messages),
        )
        .route(
            "/api/provider/{provider}/v1/models",
            get(proxy::openai::handle_list_models),
        )
        .route(
            "/api/provider/{provider}/v1/models/{model_id}",
            get(proxy::openai::handle_get_model),
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
        .route("/api/cache/stats", get(admin::cache_stats))
        .route("/api/config/reload", post(admin::reload_config))
        .route("/api/mcp/servers", get(admin::list_mcp_servers))
        .route("/api/mcp/servers", post(admin::create_mcp_server))
        .route("/api/mcp/servers/{id}", put(admin::update_mcp_server))
        .route("/api/mcp/servers/{id}", delete(admin::delete_mcp_server))
        .route("/api/mcp/servers/{id}/start", post(admin::start_mcp_server))
        .route("/api/mcp/servers/{id}/stop", post(admin::stop_mcp_server))
        .route(
            "/api/mcp/servers/{id}/tools",
            get(admin::list_mcp_server_tools),
        )
        .route("/api/mcp/tools", get(admin::list_all_mcp_tools))
        .route("/api/virtual-keys", get(admin::list_virtual_keys))
        .route("/api/virtual-keys", post(admin::create_virtual_key))
        .route("/api/virtual-keys/{id}", put(admin::update_virtual_key))
        .route("/api/virtual-keys/{id}", delete(admin::delete_virtual_key))
        .route("/api/provider-budgets", get(admin::list_provider_budgets))
        .route(
            "/api/provider-budgets/{provider}",
            put(admin::set_provider_budget),
        )
        .route(
            "/api/provider-budgets/{provider}",
            delete(admin::delete_provider_budget),
        )
        .route("/api/gateway/info", get(admin::gateway_info))
        .with_state(admin_route_state)
        .layer(axum::middleware::from_fn_with_state(
            admin_auth_state,
            middleware::auth::admin_auth_middleware,
        ));

    let base_router = Router::new()
        .merge(proxy_router)
        .merge(admin_router)
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz_handler));

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

    // Build CORS layer — restrictive in production, permissive only when no origins configured
    let cors_layer = build_cors_layer(&state.security.allowed_origins);

    let router = router
        .layer(axum::middleware::from_fn(
            middleware::request_id::request_id_middleware,
        ))
        .layer(axum::middleware::from_fn(
            middleware::security_headers::security_headers_middleware,
        ))
        .layer(
            CompressionLayer::new()
                .gzip(true)
                .br(true)
                .zstd(true)
                .deflate(true),
        )
        .layer(cors_layer)
        .layer(TraceLayer::new_for_http())
        .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024));

    // Serve web console static files if configured.
    // Explicit API routes (above) take precedence — this only catches unmatched paths,
    // which is exactly SPA routing behavior.
    let router = if let Some(dir) = web_console_dir {
        use tower_http::services::{ServeDir, ServeFile};
        let index_path = format!("{}/index.html", dir);
        let serve_dir = ServeDir::new(dir).fallback(ServeFile::new(index_path));
        router.fallback_service(serve_dir)
    } else {
        router
    };

    router
}

/// Build a configurable CORS layer.
///
/// * When `allowed_origins` is configured (non-empty), only those origins are allowed.
/// * When `allowed_origins` is `None` or empty:
///   - **Debug builds**: `CorsLayer::permissive()` — convenient for local development.
///   - **Release builds**: localhost-only (127.0.0.1:8080, localhost:8080).
fn build_cors_layer(allowed_origins: &Option<Vec<String>>) -> CorsLayer {
    use axum::http::header;
    use axum::http::{HeaderValue, Method};
    use tower_http::cors::AllowOrigin;

    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
        Method::HEAD,
    ];
    let headers = [
        header::AUTHORIZATION,
        header::CONTENT_TYPE,
        header::ACCEPT,
        header::ORIGIN,
    ];

    match allowed_origins {
        Some(origins) if !origins.is_empty() => {
            let origin_values: Vec<HeaderValue> = origins
                .iter()
                .filter_map(|o| match o.parse::<HeaderValue>() {
                    Ok(v) => Some(v),
                    Err(_) => {
                        tracing::warn!("Invalid CORS origin in config: {o}");
                        None
                    }
                })
                .collect();

            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origin_values))
                .allow_methods(methods)
                .allow_headers(headers)
        }
        _ => {
            // No origins configured — localhost-only in release, permissive in debug
            #[cfg(debug_assertions)]
            {
                CorsLayer::permissive()
            }
            #[cfg(not(debug_assertions))]
            {
                let localhost_origins: Vec<HeaderValue> = vec![
                    "http://127.0.0.1:8080".parse().unwrap(),
                    "http://localhost:8080".parse().unwrap(),
                ];
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(localhost_origins))
                    .allow_methods(methods)
                    .allow_headers(headers)
            }
        }
    }
}

/// Handler for the `/metrics` Prometheus scrape endpoint.
async fn metrics_handler() -> axum::response::Response {
    let body = crate::metrics::render();
    axum::response::Response::builder()
        .header("Content-Type", "text/plain; version=0.0.4")
        .body(axum::body::Body::from(body))
        .expect("valid response")
}

/// Lightweight liveness probe returning a minimal `{"status":"ok"}` body.
/// Intended for Kubernetes-style liveness checks that only need a 200 OK
/// without the overhead of the full `/health` endpoint. Unauthenticated.
async fn healthz_handler() -> axum::response::Response {
    crate::proxy::stream::json_response(
        axum::http::StatusCode::OK,
        r#"{"status":"ok"}"#.to_string(),
    )
}

/// Validate a TLS file path: canonicalize to prevent traversal, check extension.
/// Returns the canonical path or panics with a redacted error message.
fn validate_tls_path(path: &str, kind: &str) -> std::path::PathBuf {
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
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !allowed_extensions.contains(&ext) {
        panic!(
            "TLS {} file must have one of these extensions: {:?}",
            kind, allowed_extensions
        );
    }

    tracing::info!(kind, path = %canonical.display(), "Loading TLS {} file", kind);
    canonical
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
        let cert_path = validate_tls_path(&tls_config.cert, "cert");
        let key_path = validate_tls_path(&tls_config.key, "key");

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
                                            let svc = hyper_util::service::TowerToHyperService::new(app_clone);
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
        let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_fut);

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
}
