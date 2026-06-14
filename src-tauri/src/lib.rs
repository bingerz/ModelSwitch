mod admin;
mod channel;
pub mod config;
pub mod credential;
mod health;
mod log;
mod mcp;
mod middleware;
mod proxy;
mod quota;
mod router;
pub mod virtual_key;
#[cfg(feature = "tauri")]
mod webview_login;

use channel::manager::ChannelManager;
#[cfg(feature = "tauri")]
use channel::ChannelStatus;
use config::AppConfig;
use credential::create_credential_store;
use log::DispatchLogger;
use mcp::McpManager;
use proxy::cache::RequestCache;
use proxy::openai::AppState;
use proxy::payload_rules::ChannelPayloadRules;
use proxy::rate_limiter::RateLimiter;
use quota::registry::QuotaProviderRegistry;
use quota::QuotaStore;
use router::active_requests::ActiveRequests;
use router::affinity::SessionAffinity;
use std::sync::Arc;
use virtual_key::VirtualKeyStore;

use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use tokio::sync::oneshot;
use tokio::sync::Notify;

#[cfg(feature = "tauri")]
use serde::Serialize;
#[cfg(feature = "tauri")]
use std::sync::Mutex;

/// Spawn a background task, working both in CLI (tokio runtime) and Tauri (main thread without runtime).
fn spawn_bg<F>(future: F)
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(future);
        }
        Err(_) => {
            #[cfg(feature = "tauri")]
            {
                tauri::async_runtime::spawn(future);
            }
            #[cfg(not(feature = "tauri"))]
            {
                static RT: std::sync::OnceLock<tokio::runtime::Runtime> =
                    std::sync::OnceLock::new();
                let rt = RT.get_or_init(|| {
                    tokio::runtime::Runtime::new().expect("Failed to create background runtime")
                });
                rt.spawn(future);
            }
        }
    }
}

/// Shared gateway state returned by [`start_gateway_services`].
pub struct GatewayHandles {
    pub state: Arc<AppState>,
    pub host: String,
    pub port: u16,
    pub drain_timeout_secs: u64,
}

// ── Gateway lifecycle management (Tauri only) ──────────────────────────────

#[cfg(feature = "tauri")]
struct GwInner {
    running: bool,
    shutdown: Option<Arc<Notify>>,
    stopped_rx: Option<oneshot::Receiver<()>>,
}

#[cfg(feature = "tauri")]
struct GatewayManager {
    app_state: Arc<AppState>,
    host: String,
    port: u16,
    drain_timeout_secs: u64,
    inner: Mutex<GwInner>,
}

#[cfg(feature = "tauri")]
impl GatewayManager {
    fn new(handles: GatewayHandles) -> Self {
        Self {
            app_state: handles.state,
            host: handles.host,
            port: handles.port,
            drain_timeout_secs: handles.drain_timeout_secs,
            inner: Mutex::new(GwInner {
                running: false,
                shutdown: None,
                stopped_rx: None,
            }),
        }
    }

    /// Synchronous shutdown: fires the Notify and cleans up PID file.
    /// Called from Drop or window Destroyed event — no async context available.
    fn force_shutdown(&self) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.running {
            return;
        }
        tracing::info!("Gateway force-shutdown triggered");
        inner.running = false;

        // Persist quota data before shutting down (synchronous, no async needed)
        self.app_state.quota_store.persist_sync();

        if let Some(shutdown) = inner.shutdown.take() {
            shutdown.notify_waiters();
            tracing::info!("Gateway force-shutdown triggered");
        }
        inner.stopped_rx = None;

        // Clean up PID file
        let pid_path = config::app_config_dir().join("gateway.pid");
        let _ = std::fs::remove_file(&pid_path);
    }
}

#[cfg(feature = "tauri")]
impl Drop for GatewayManager {
    fn drop(&mut self) {
        self.force_shutdown();
    }
}

#[cfg(feature = "tauri")]
#[derive(Serialize)]
struct GatewayStatus {
    running: bool,
    host: String,
    port: u16,
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn gateway_status(
    manager: tauri::State<'_, GatewayManager>,
) -> Result<GatewayStatus, String> {
    let inner = manager.inner.lock().unwrap();
    Ok(GatewayStatus {
        running: inner.running,
        host: manager.host.clone(),
        port: manager.port,
    })
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn gateway_start(manager: tauri::State<'_, GatewayManager>) -> Result<(), String> {
    {
        let inner = manager.inner.lock().unwrap();
        if inner.running {
            return Err("Gateway already running".into());
        }
    }

    let shutdown = Arc::new(Notify::new());
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let (bind_ok_tx, bind_ok_rx) = oneshot::channel::<Result<(), String>>();

    let state = Arc::clone(&manager.app_state);
    let host = manager.host.clone();
    let port = manager.port;
    let drain = manager.drain_timeout_secs;
    let shutdown_clone = Arc::clone(&shutdown);

    tauri::async_runtime::spawn(async move {
        start_gateway(
            state,
            &host,
            port,
            drain,
            Some(shutdown_clone),
            Some(bind_ok_tx),
        )
        .await;
        let _ = stopped_tx.send(());
    });

    // Wait for the spawned task to confirm bind succeeded (or failed)
    match bind_ok_rx.await {
        Ok(Ok(())) => {
            let mut inner = manager.inner.lock().unwrap();
            inner.running = true;
            inner.shutdown = Some(shutdown);
            inner.stopped_rx = Some(stopped_rx);
            tracing::info!(
                "Gateway started successfully on {}:{}",
                manager.host,
                manager.port
            );
            Ok(())
        }
        Ok(Err(e)) => {
            tracing::error!("Gateway bind failed: {e}");
            Err(e)
        }
        Err(_) => {
            let msg = "Gateway task exited before bind".to_string();
            tracing::error!("{msg}");
            Err(msg)
        }
    }
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn gateway_stop(manager: tauri::State<'_, GatewayManager>) -> Result<(), String> {
    let (stopped_rx, shutdown) = {
        let mut inner = manager.inner.lock().unwrap();
        if !inner.running {
            return Err("Gateway not running".into());
        }
        inner.running = false;

        // Persist quota data before shutting down
        manager.app_state.quota_store.persist_sync();

        let rx = inner.stopped_rx.take();
        let shutdown = inner.shutdown.take();
        (rx, shutdown)
    }; // MutexGuard dropped here

    if let Some(shutdown) = shutdown {
        shutdown.notify_waiters();
    }

    if let Some(rx) = stopped_rx {
        let timeout = std::time::Duration::from_secs(35);
        if tokio::time::timeout(timeout, rx).await.is_err() {
            tracing::warn!("Gateway stop timed out after {}s", timeout.as_secs());
        }
    }

    tracing::info!("Gateway stopped");
    Ok(())
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn gateway_restart(manager: tauri::State<'_, GatewayManager>) -> Result<(), String> {
    gateway_stop(manager.clone()).await?;
    // Small delay to ensure port is released
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    gateway_start(manager).await
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn app_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn app_hide(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn mcp_list_servers(
    manager: tauri::State<'_, GatewayManager>,
) -> Result<Vec<admin::McpServerResponse>, String> {
    let state = &manager.app_state;
    let statuses = state.mcp_manager.list_status().await;
    let mut responses = Vec::with_capacity(statuses.len());
    for (id, _name, status) in statuses {
        if let Some(config) = state.mcp_manager.get_config(&id).await {
            responses.push(admin::McpServerResponse {
                id: config.id,
                name: config.name,
                command: config.command,
                args: config.args,
                env: config.env,
                cwd: config.cwd,
                enabled: config.enabled,
                expose_tools: config.expose_tools,
                status,
            });
        }
    }
    Ok(responses)
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn mcp_start_server(
    manager: tauri::State<'_, GatewayManager>,
    server_id: String,
) -> Result<(), String> {
    manager
        .app_state
        .mcp_manager
        .start_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn mcp_stop_server(
    manager: tauri::State<'_, GatewayManager>,
    server_id: String,
) -> Result<(), String> {
    manager
        .app_state
        .mcp_manager
        .stop_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(feature = "tauri")]
#[tauri::command]
async fn mcp_list_tools(
    manager: tauri::State<'_, GatewayManager>,
    server_id: String,
) -> Result<Vec<admin::McpToolResponse>, String> {
    let tools = manager
        .app_state
        .mcp_manager
        .list_tools(&server_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(tools
        .iter()
        .map(|t| admin::McpToolResponse {
            name: t.name.to_string(),
            description: t.description.as_deref().map(String::from),
        })
        .collect())
}

/// Build gateway state and start background services.
/// Callable from both Tauri setup and CLI mode.
/// If `config_path` is provided, loads config from that path instead of the default.
pub fn start_gateway_services(config_path: Option<std::path::PathBuf>) -> GatewayHandles {
    // Initialize tracing (no-op if already initialized, e.g. by CLI main)
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
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
    let in_flight = Arc::new(proxy::cache::InFlightRequests::new());

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
        admin_token,
        request_timeout_secs: config.gateway.request_timeout_secs,
        stream_keepalive_secs: config.gateway.stream_keepalive_secs,
        logger: Arc::clone(&logger),
        http_client: http_client.clone(),
        max_retries,
        model_fallbacks,
        routing_strategy,
        session_affinity: SessionAffinity::default(),
        active_requests: Arc::clone(&active_requests),
        request_cache: Arc::clone(&request_cache),
        payload_rules: Arc::clone(&payload_rules),
        rate_limiter: Arc::clone(&rate_limiter),
        quota_store: Arc::clone(&quota_store),
        virtual_key_store: Arc::clone(&virtual_key_store),
        in_flight: Arc::clone(&in_flight),
        mcp_manager: Arc::clone(&mcp_manager),
        mcp_max_iterations: config.gateway.mcp_max_iterations,
        mcp_auto_inject: config.gateway.mcp_auto_inject,
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
            let path = virtual_key::persistence_path();
            if let Err(e) = boot_vk.load(&path).await {
                tracing::warn!(error = %e, ?path, "Failed to load virtual keys");
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
                let path = virtual_key::persistence_path();
                if let Err(e) = persist_vk.persist(&path).await {
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
        let affinity_cleanup = state.session_affinity.clone();
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
    let admin_route_state = Arc::clone(&state);
    let admin_auth_state = Arc::clone(&state);

    // Proxy routes — virtual-key auth (pass-through when no virtual keys configured)
    let proxy_router = Router::new()
        .route(
            "/v1/chat/completions",
            post(proxy::openai::handle_chat_completions),
        )
        .route("/v1/models", get(proxy::openai::handle_list_models))
        .route("/v1/tools", get(proxy::openai::handle_list_tools))
        .route("/v1/messages", post(proxy::anthropic::handle_messages))
        .route("/v1beta/models/*path", post(proxy::gemini::handle_gemini))
        .route("/health", get(proxy::openai::health_check))
        .layer(axum::middleware::from_fn_with_state(
            proxy_auth_state,
            middleware::virtual_key::virtual_key_middleware,
        ))
        .with_state(proxy_state);

    // Admin routes — optional Bearer token auth
    let admin_router = Router::new()
        .route("/api/channels", get(admin::list_channels))
        .route("/api/channels", post(admin::create_channel))
        .route("/api/channels/:id", put(admin::update_channel))
        .route("/api/channels/:id", delete(admin::delete_channel))
        .route("/api/channels/:id/ping", post(admin::ping_channel))
        .route("/api/channels/:id/status", get(admin::channel_status))
        .route("/api/logs", get(admin::get_logs))
        .route("/api/stats", get(admin::get_stats))
        .route("/api/stats/cost", get(admin::get_cost_stats))
        .route("/api/stats/usage", get(admin::get_usage_history))
        .route("/api/quota", get(admin::get_quota))
        .route("/api/auth/cookies", post(admin::receive_login_cookies))
        .route("/api/auth/pending-cookies", get(admin::get_pending_cookies))
        .route(
            "/api/channels/:id/reset-circuit",
            post(admin::reset_circuit),
        )
        .route(
            "/api/channels/:id/payload-rules",
            put(admin::set_payload_rules),
        )
        .route("/api/cache/flush", post(admin::flush_cache))
        .route("/api/config/reload", post(admin::reload_config))
        .route("/api/mcp/servers", get(admin::list_mcp_servers))
        .route("/api/mcp/servers", post(admin::create_mcp_server))
        .route("/api/mcp/servers/:id", put(admin::update_mcp_server))
        .route("/api/mcp/servers/:id", delete(admin::delete_mcp_server))
        .route("/api/mcp/servers/:id/start", post(admin::start_mcp_server))
        .route("/api/mcp/servers/:id/stop", post(admin::stop_mcp_server))
        .route(
            "/api/mcp/servers/:id/tools",
            get(admin::list_mcp_server_tools),
        )
        .route("/api/mcp/tools", get(admin::list_all_mcp_tools))
        .route("/api/virtual-keys", get(admin::list_virtual_keys))
        .route("/api/virtual-keys", post(admin::create_virtual_key))
        .route("/api/virtual-keys/:id", put(admin::update_virtual_key))
        .route("/api/virtual-keys/:id", delete(admin::delete_virtual_key))
        .with_state(admin_route_state)
        .layer(axum::middleware::from_fn_with_state(
            admin_auth_state,
            middleware::auth::admin_auth_middleware,
        ));

    Router::new()
        .merge(proxy_router)
        .merge(admin_router)
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
    drain_timeout_secs: u64,
    shutdown_notify: Option<Arc<Notify>>,
    bind_notify: Option<oneshot::Sender<Result<(), String>>>,
) {
    let app = build_router(state.clone());
    let quota_for_shutdown = Arc::clone(&state.quota_store);
    let mcp_for_shutdown = Arc::clone(&state.mcp_manager);
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

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C, shutting down"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }
}

/// Start gateway from pre-built handles (convenience for CLI).
pub async fn run_gateway(handles: GatewayHandles) {
    let GatewayHandles {
        state,
        host,
        port,
        drain_timeout_secs,
    } = handles;
    start_gateway(state, &host, port, drain_timeout_secs, None, None).await;
}

// ── Tauri entry point ──────────────────────────────────────────────────────

#[cfg(feature = "tauri")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::Manager;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            webview_login::open_login_webview,
            quota::webview_scrape::scrape_webview_quota,
            gateway_status,
            gateway_start,
            gateway_stop,
            gateway_restart,
            app_quit,
            app_hide,
            mcp_list_servers,
            mcp_start_server,
            mcp_stop_server,
            mcp_list_tools,
        ])
        .setup(|app| {
            let handles = start_gateway_services(None);
            let manager = GatewayManager::new(handles);
            let state = Arc::clone(&manager.app_state);

            app.manage(manager);

            // Setup system tray
            let tray = tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("ModelSwitch - Starting...")
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Background task: update tray tooltip with channel health
            let mgr = Arc::clone(&state.channel_mgr);
            spawn_bg(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let channels = mgr.list().await;
                    let total = channels.len();
                    let healthy = channels
                        .iter()
                        .filter(|c| c.status == ChannelStatus::Healthy && c.enabled)
                        .count();
                    let tooltip = format!("ModelSwitch – {}/{} channels healthy", healthy, total);
                    let _ = tray.set_tooltip(Some(&tooltip));
                }
            });

            // Intercept window close: prevent default and let frontend decide
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    use tauri::Emitter;
                    match event {
                        tauri::WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            let _ = app_handle.emit("close-requested", ());
                        }
                        tauri::WindowEvent::Destroyed => {
                            tracing::warn!("Window Destroyed event — calling force_shutdown");
                            // Safety net: ensure gateway stops when window is destroyed
                            // (e.g. OS shutdown, task manager close)
                            let manager = app_handle.state::<GatewayManager>();
                            manager.force_shutdown();
                        }
                        _ => {}
                    }
                });

                // Open DevTools in debug builds to help diagnose rendering issues
                #[cfg(debug_assertions)]
                {
                    let _ = window.open_devtools();
                }
            }

            // Gateway auto-start is handled by the frontend (App.tsx useEffect)
            // to avoid a race between Rust-side auto-start and frontend invoke.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
