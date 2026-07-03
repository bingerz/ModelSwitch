//! Axum router assembly: admin, portal, auth, and proxy route definitions.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{any, delete, get, post, put};
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::admin;
use crate::middleware;
use crate::proxy;
use crate::proxy::AppState;

/// Build admin route tree under the given path prefix (e.g., `/api` or `/v1/api`).
/// Both prefixes are mounted to ensure backward compatibility during versioning migration.
fn admin_routes(prefix: &str) -> Router<Arc<AppState>> {
    Router::new()
        .route(&format!("{prefix}/channels"), get(admin::list_channels))
        .route(&format!("{prefix}/channels"), post(admin::create_channel))
        .route(
            &format!("{prefix}/channels/{{id}}"),
            put(admin::update_channel),
        )
        .route(
            &format!("{prefix}/channels/{{id}}"),
            delete(admin::delete_channel),
        )
        .route(
            &format!("{prefix}/channels/{{id}}/ping"),
            post(admin::ping_channel),
        )
        .route(
            &format!("{prefix}/channels/{{id}}/status"),
            get(admin::channel_status),
        )
        .route(
            &format!("{prefix}/channels/batch/enable"),
            post(admin::batch_enable_channels),
        )
        .route(
            &format!("{prefix}/channels/batch/disable"),
            post(admin::batch_disable_channels),
        )
        .route(
            &format!("{prefix}/channels/batch/delete"),
            post(admin::batch_delete_channels),
        )
        .route(
            &format!("{prefix}/channels/batch/tags"),
            put(admin::batch_update_tags),
        )
        .route(&format!("{prefix}/logs"), get(admin::get_logs))
        .route(&format!("{prefix}/stats"), get(admin::get_stats))
        .route(&format!("{prefix}/stats/cost"), get(admin::get_cost_stats))
        .route(
            &format!("{prefix}/stats/usage"),
            get(admin::get_usage_history),
        )
        .route(&format!("{prefix}/quota"), get(admin::get_quota))
        .route(
            &format!("{prefix}/auth/cookies"),
            post(admin::receive_login_cookies),
        )
        .route(
            &format!("{prefix}/auth/pending-cookies"),
            get(admin::get_pending_cookies),
        )
        .route(
            &format!("{prefix}/channels/{{id}}/reset-circuit"),
            post(admin::reset_circuit),
        )
        .route(
            &format!("{prefix}/channels/{{id}}/payload-rules"),
            get(admin::get_payload_rules).put(admin::set_payload_rules),
        )
        .route(&format!("{prefix}/cache/flush"), post(admin::flush_cache))
        .route(&format!("{prefix}/cache/stats"), get(admin::cache_stats))
        .route(
            &format!("{prefix}/cache/mode"),
            put(admin::update_cache_mode),
        )
        .route(
            &format!("{prefix}/config/reload"),
            post(admin::reload_config),
        )
        .route(
            &format!("{prefix}/mcp/servers"),
            get(admin::list_mcp_servers),
        )
        .route(
            &format!("{prefix}/mcp/servers"),
            post(admin::create_mcp_server),
        )
        .route(
            &format!("{prefix}/mcp/servers/{{id}}"),
            put(admin::update_mcp_server),
        )
        .route(
            &format!("{prefix}/mcp/servers/{{id}}"),
            delete(admin::delete_mcp_server),
        )
        .route(
            &format!("{prefix}/mcp/servers/{{id}}/start"),
            post(admin::start_mcp_server),
        )
        .route(
            &format!("{prefix}/mcp/servers/{{id}}/stop"),
            post(admin::stop_mcp_server),
        )
        .route(
            &format!("{prefix}/mcp/servers/{{id}}/tools"),
            get(admin::list_mcp_server_tools),
        )
        .route(
            &format!("{prefix}/mcp/tools"),
            get(admin::list_all_mcp_tools),
        )
        .route(
            &format!("{prefix}/virtual-keys"),
            get(admin::list_virtual_keys),
        )
        .route(
            &format!("{prefix}/virtual-keys"),
            post(admin::create_virtual_key),
        )
        .route(
            &format!("{prefix}/virtual-keys/batch"),
            post(admin::batch_create_virtual_keys),
        )
        .route(
            &format!("{prefix}/virtual-keys/{{id}}"),
            put(admin::update_virtual_key),
        )
        .route(
            &format!("{prefix}/virtual-keys/{{id}}"),
            delete(admin::delete_virtual_key),
        )
        .route(
            &format!("{prefix}/virtual-keys/groups"),
            get(admin::list_virtual_key_groups),
        )
        .route(
            &format!("{prefix}/provider-budgets"),
            get(admin::list_provider_budgets),
        )
        .route(
            &format!("{prefix}/provider-budgets/{{provider}}"),
            put(admin::set_provider_budget),
        )
        .route(
            &format!("{prefix}/provider-budgets/{{provider}}"),
            delete(admin::delete_provider_budget),
        )
        .route(&format!("{prefix}/gateway/info"), get(admin::gateway_info))
        .route(
            &format!("{prefix}/gateway/model-routing"),
            get(admin::model_routing_info),
        )
        .route(
            &format!("{prefix}/gateway/routing-strategy"),
            put(admin::update_routing_strategy),
        )
        .route(&format!("{prefix}/audit-log"), get(admin::get_audit_log))
        // ── Feature module routes ────────────────────────────
        // Guardrails config
        .route(
            &format!("{prefix}/guardrails"),
            get(admin::get_guardrails_config),
        )
        .route(
            &format!("{prefix}/guardrails"),
            put(admin::update_guardrails_config),
        )
        // Redemption codes
        .route(
            &format!("{prefix}/redemption-codes"),
            get(admin::list_redemption_codes),
        )
        .route(
            &format!("{prefix}/redemption-codes"),
            post(admin::create_redemption_code),
        )
        .route(
            &format!("{prefix}/redemption-codes/redeem"),
            post(admin::redeem_code),
        )
        .route(
            &format!("{prefix}/redemption-codes/{{code}}"),
            delete(admin::delete_redemption_code),
        )
        // Notifications
        .route(
            &format!("{prefix}/notifications"),
            get(admin::get_notification_config),
        )
        .route(
            &format!("{prefix}/notifications"),
            put(admin::update_notification_config),
        )
        // Channel auto-test
        .route(
            &format!("{prefix}/channels/{{id}}/test"),
            post(admin::test_channel),
        )
        .route(
            &format!("{prefix}/channels/test-all"),
            post(admin::test_all_channels),
        )
        // Channel enhanced diagnostics
        .route(
            &format!("{prefix}/channels/{{id}}/diagnostics"),
            post(admin::channel_diagnostics),
        )
        // Channel cooldown status
        .route(
            &format!("{prefix}/channels/{{id}}/cooldown"),
            get(admin::get_channel_cooldown),
        )
        // MCP health
        .route(&format!("{prefix}/mcp/health"), get(admin::get_mcp_health))
        // Completion ratios
        .route(
            &format!("{prefix}/completion-ratios"),
            get(admin::get_completion_ratios),
        )
        .route(
            &format!("{prefix}/completion-ratios"),
            put(admin::update_completion_ratios),
        )
        // Model registry inspection
        .route(
            &format!("{prefix}/model-registry"),
            get(admin::get_model_registry),
        )
        // Usage reports (JSON + CSV export)
        .route(
            &format!("{prefix}/reports/usage"),
            get(admin::get_usage_report),
        )
        .route(
            &format!("{prefix}/reports/usage/csv"),
            get(admin::get_usage_report_csv),
        )
        // Sanitizer (privacy guardrail) config
        .route(
            &format!("{prefix}/sanitizer"),
            get(admin::get_sanitizer_config),
        )
        .route(
            &format!("{prefix}/sanitizer"),
            put(admin::update_sanitizer_config),
        )
        .route(&format!("{prefix}/auth/me"), get(admin::auth::auth_me))
        .route(&format!("{prefix}/auth/status"), get(admin::auth_status))
}

/// Portal routes — employee self-service, authenticated by virtual key.
/// These are NOT protected by admin_auth_middleware.
fn portal_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/portal/usage", get(admin::portal::portal_usage))
        .route("/api/portal/logs", get(admin::portal::portal_logs))
        .route("/api/portal/test", get(admin::portal::portal_test))
}

/// Public authentication routes — NOT protected by admin_auth_middleware.
/// These endpoints handle their own authentication internally (e.g. LDAP bind).
///
/// Rate-limiting middleware is applied to prevent brute-force attacks on
/// publicly accessible auth endpoints (e.g. LDAP credential stuffing).
fn auth_routes(state: Arc<AppState>) -> Router {
    let auth_rate_limit_state = Arc::clone(&state);
    Router::new()
        .route("/api/auth/ldap/login", post(admin::auth::ldap_login))
        .route("/api/auth/oidc/login", get(admin::auth::oidc_login))
        .route("/api/auth/oidc/callback", get(admin::auth::oidc_callback))
        .layer(axum::middleware::from_fn_with_state(
            auth_rate_limit_state,
            middleware::auth::auth_rate_limit_middleware,
        ))
        .with_state(state)
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
        .route("/v1/images/edits", post(proxy::images::handle_image_edits))
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

    // Admin routes -- optional Bearer token auth.
    // Mounted under both `/api` (backward compat) and `/v1/api` (versioned).
    let admin_router = admin_routes("/api")
        .merge(admin_routes("/v1/api"))
        // Metrics endpoints are mounted behind admin auth so that only
        // authenticated callers (Bearer token) can scrape per-model /
        // per-channel traffic, cost, and error data.
        .route("/metrics", get(metrics_handler))
        .route("/v1/metrics", get(metrics_handler))
        .with_state(admin_route_state)
        .layer(axum::middleware::from_fn(middleware::rbac::rbac_middleware))
        .layer(axum::middleware::from_fn_with_state(
            admin_auth_state,
            middleware::auth::admin_auth_middleware,
        ));

    // `/ready` needs AppState to inspect channel health, so it is mounted on
    // its own stateful router and converted to `Router<()>` before merging.
    let ready_router = Router::new()
        .route("/ready", get(ready_handler))
        .with_state(Arc::clone(&state));

    let base_router = Router::new()
        .merge(proxy_router)
        .merge(admin_router)
        .merge(portal_routes().with_state(Arc::clone(&state)))
        .merge(auth_routes(Arc::clone(&state)))
        .merge(ready_router)
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
        // Catch unmatched API paths before SPA fallback so they return JSON 404
        // instead of index.html with HTTP 200.
        router
            .route("/api/{*rest}", any(api_json_not_found))
            .route("/v1/api/{*rest}", any(api_json_not_found))
            .fallback_service(serve_dir)
    } else {
        router
    };

    router
}

/// Return JSON 404 for unmatched API paths.
/// Prevents the SPA static file fallback from serving index.html for
/// mistyped API endpoints (which would return HTML 200 instead of JSON 404).
async fn api_json_not_found() -> impl axum::response::IntoResponse {
    (
        axum::http::StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": { "message": "Not found", "code": "not_found" }
        })),
    )
}

/// Build a configurable CORS layer.
///
/// * When `allowed_origins` is configured (non-empty), only those origins are allowed.
/// * When `allowed_origins` is `None` or empty:
///   - **Debug builds**: `CorsLayer::permissive()` — convenient for local development.
///   - **Release builds**: localhost-only (127.0.0.1:8080, localhost:8080).
pub(super) fn build_cors_layer(allowed_origins: &Option<Vec<String>>) -> CorsLayer {
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

/// Readiness probe — returns 200 when the gateway can serve at least one
/// upstream request, 503 when all channels are unhealthy.
///
/// Unlike `/healthz` (liveness), this distinguishes "starting up" from
/// "ready to serve traffic." Zero channels configured is treated as ready
/// (fresh install / config-only mode). Unauthenticated.
async fn ready_handler(State(state): State<Arc<AppState>>) -> axum::response::Response {
    use axum::response::IntoResponse;

    let channels = state.channel_mgr.list().await;

    if channels.is_empty() {
        // No channels configured — fresh install or config-only mode.
        // Return 200: the gateway is "ready" (it just has nothing to proxy to yet).
        return (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({
                "status": "ready",
                "channels": 0,
                "healthy": 0,
            })),
        )
            .into_response();
    }

    let total = channels.len();
    let healthy = channels.iter().filter(|c| c.is_available()).count();

    if healthy == 0 {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "not_ready",
                "channels": total,
                "healthy": 0,
                "reason": "all channels are unhealthy",
            })),
        )
            .into_response();
    }

    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({
            "status": "ready",
            "channels": total,
            "healthy": healthy,
        })),
    )
        .into_response()
}
