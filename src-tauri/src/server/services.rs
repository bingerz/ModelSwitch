//! Gateway services: state initialization, infra setup, and background task spawning.

use std::sync::Arc;

use crate::admin::audit::AuditLog;
use crate::channel::manager::ChannelManager;
use crate::config::{self, AppConfig};
use crate::credential::create_credential_store;
use crate::guardrails::{GuardrailsChecker, GuardrailsConfig};
use crate::health;
use crate::log::DispatchLogger;
use crate::mcp::McpManager;
use crate::model_registry::{refresh_from_endpoint, ModelRegistry};
use crate::notification::NotificationService;
use crate::provider_budget::ProviderBudgetStore;
use crate::proxy::cache::{CacheMode, InFlightRequests, RequestCache};
use crate::proxy::payload_rules::{ChannelPayloadRules, PayloadRules};
use crate::proxy::rate_limiter::{KeyRateLimiter, RateLimiter};
use crate::proxy::{
    AppState, BillingState, CacheState, LimitsState, McpState, ProxyParams, RouterState,
    SecurityState,
};
use crate::quota::registry::QuotaProviderRegistry;
use crate::quota::{self, QuotaStore, RedemptionCodeStore};
use crate::router::active_requests::ActiveRequests;
use crate::router::affinity::SessionAffinity;
use crate::spawn_bg;
use crate::virtual_key::VirtualKeyStore;
use crate::GatewayHandles;

/// Spawn tasks that load persisted state and periodically save it to disk.
pub(super) fn spawn_persistence_tasks(state: &Arc<AppState>) {
    // Load persisted dispatch logs at startup
    let boot_logger = Arc::clone(&state.logger);
    spawn_bg(async move {
        boot_logger.load_from_file().await;
    });

    // Load persisted audit log at startup (administrative history survives restarts)
    let boot_audit = Arc::clone(&state.audit_log);
    spawn_bg(async move {
        boot_audit.load_from_file().await;
    });

    // Load persisted quota data (token usage survives restarts)
    {
        let boot_quota = Arc::clone(&state.billing.quota_store);
        spawn_bg(async move {
            boot_quota.load_from_file().await;
        });
    }

    // Periodic quota persistence (every 10s)
    {
        let persist_quota = Arc::clone(&state.billing.quota_store);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
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

    // Periodic virtual key persistence (every 10s)
    {
        let persist_vk = Arc::clone(&state.billing.virtual_key_store);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
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

    // Periodic provider budget persistence (every 10s)
    {
        let persist_pb = Arc::clone(&state.billing.provider_budgets);
        spawn_bg(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
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
pub(super) fn spawn_background_services(state: &Arc<AppState>, config: &AppConfig) {
    // Start background health probe (P2-7: simplified periodic connectivity check).
    // When health_check_interval_secs > 0, spawns a lightweight probe that
    // verifies each channel's base URL is reachable without sending API requests.
    if config.gateway.health_check_enabled && config.gateway.health_check_interval_secs > 0 {
        let probe_state = Arc::clone(state);
        let probe_interval = config.gateway.health_check_interval_secs;
        spawn_bg(async move {
            health::run_periodic_probe(probe_state, probe_interval).await;
        });
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
pub(super) fn spawn_config_watcher(
    state: &Arc<AppState>,
    watcher_config_path: &Option<std::path::PathBuf>,
) {
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
            key_rate_limiter: Arc::new(KeyRateLimiter::new()),
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
        guardrails: Arc::new(GuardrailsChecker::new(GuardrailsConfig::default())),
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
