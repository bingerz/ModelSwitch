//! Admin route handlers for feature modules: guardrails, redemption codes,
//! notifications, channel auto-test, MCP health, completion ratios, and
//! channel cooldown status.

use super::ApiResponse;
use crate::channel::ChannelTestResult;
use crate::guardrails::GuardrailsConfig;
use crate::mcp::McpServerHealth;
use crate::middleware::error::ApiError;
use crate::notification::NotificationConfig;
use crate::proxy::AppState;
use crate::quota::RedemptionCode;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ─── Guardrails ───────────────────────────────────────

/// `GET /api/guardrails` — return current guardrails config.
pub async fn get_guardrails_config(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<GuardrailsConfig>> {
    let config = state.guardrails.get_config();
    Json(ApiResponse::ok(config))
}

/// `PUT /api/guardrails` — update guardrails config.
///
/// Accepts a full [`GuardrailsConfig`] body. All fields use `#[serde(default)]`
/// so omitted fields fall back to their defaults, enabling partial updates.
pub async fn update_guardrails_config(
    State(state): State<Arc<AppState>>,
    Json(config): Json<GuardrailsConfig>,
) -> Json<ApiResponse<GuardrailsConfig>> {
    state.guardrails.update_config(config.clone());
    Json(ApiResponse::ok(config))
}

// ─── Redemption Codes ─────────────────────────────────

/// Request body for creating a redemption code.
#[derive(Debug, Deserialize)]
pub struct CreateRedemptionCodeRequest {
    pub credits_cents: u64,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Request body for redeeming a code.
#[derive(Debug, Deserialize)]
pub struct RedeemCodeRequest {
    pub code: String,
    #[serde(default)]
    pub user_id: Option<String>,
}

/// `GET /api/redemption-codes` — list all redemption codes.
pub async fn list_redemption_codes(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<RedemptionCode>>> {
    let codes = state.redemption_codes.list();
    Json(ApiResponse::ok(codes))
}

/// `POST /api/redemption-codes` — create a new redemption code.
pub async fn create_redemption_code(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateRedemptionCodeRequest>,
) -> Json<ApiResponse<RedemptionCode>> {
    let code = state
        .redemption_codes
        .generate(req.credits_cents, req.expires_at);
    state
        .audit_log
        .record(
            "redemption_code.create",
            "admin-api",
            &code.code,
            serde_json::json!({
                "credits_cents": code.credits_cents,
            }),
        )
        .await;
    Json(ApiResponse::ok(code))
}

/// `POST /api/redemption-codes/redeem` — redeem a code.
pub async fn redeem_code(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RedeemCodeRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let redeemed_by = req.user_id.unwrap_or_else(|| "admin".to_string());
    match state.redemption_codes.redeem(&req.code, &redeemed_by) {
        Ok(credits_cents) => {
            state
                .audit_log
                .record(
                    "redemption_code.redeem",
                    "admin-api",
                    &req.code,
                    serde_json::json!({
                        "credits_cents": credits_cents,
                        "redeemed_by": redeemed_by,
                    }),
                )
                .await;
            Ok(Json(ApiResponse::ok(serde_json::json!({
                "code": req.code,
                "credits_cents": credits_cents,
                "redeemed_by": redeemed_by,
            }))))
        }
        Err(e) => Err(ApiError::new(StatusCode::BAD_REQUEST, &e.to_string())),
    }
}

/// `DELETE /api/redemption-codes/{code}` — delete a code.
pub async fn delete_redemption_code(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> axum::response::Response {
    if state.redemption_codes.delete(&code) {
        state
            .audit_log
            .record(
                "redemption_code.delete",
                "admin-api",
                &code,
                serde_json::json!({}),
            )
            .await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Redemption code not found")
    }
}

// ─── Notifications ────────────────────────────────────

/// `GET /api/notifications` — return current notification config.
pub async fn get_notification_config(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<NotificationConfig>> {
    let config = state.notifications.get_config().await;
    Json(ApiResponse::ok(config))
}

/// `PUT /api/notifications` — update notification config.
pub async fn update_notification_config(
    State(state): State<Arc<AppState>>,
    Json(config): Json<NotificationConfig>,
) -> Json<ApiResponse<NotificationConfig>> {
    state.notifications.update_config(config.clone()).await;
    Json(ApiResponse::ok(config))
}

// ─── Channel Auto-Test ────────────────────────────────

/// `POST /api/channels/{id}/test` — test single channel connectivity.
pub async fn test_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Json<ApiResponse<ChannelTestResult>> {
    let result = state.channel_mgr.run_channel_test(id).await;
    Json(ApiResponse::ok(result))
}

/// `POST /api/channels/test-all` — test all enabled channels.
pub async fn test_all_channels(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<ChannelTestResult>>> {
    let results = state.channel_mgr.run_all_tests().await;
    Json(ApiResponse::ok(results))
}

// ─── MCP Health ───────────────────────────────────────

/// `GET /api/mcp/health` — return cached health status for all MCP servers.
pub async fn get_mcp_health(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<McpServerHealth>>> {
    let health = state.mcp.mcp_manager.get_health_status().await;
    Json(ApiResponse::ok(health))
}

// ─── Completion Ratios ────────────────────────────────

/// Request body for updating completion ratios.
#[derive(Debug, Deserialize)]
pub struct UpdateCompletionRatiosRequest {
    pub ratios: HashMap<String, f64>,
}

/// `GET /api/completion-ratios` — return current completion ratios.
pub async fn get_completion_ratios(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<HashMap<String, f64>>> {
    let ratios = state.completion_ratios.read().clone();
    Json(ApiResponse::ok(ratios))
}

/// `PUT /api/completion-ratios` — update completion ratios.
pub async fn update_completion_ratios(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateCompletionRatiosRequest>,
) -> Json<ApiResponse<HashMap<String, f64>>> {
    {
        let mut ratios = state.completion_ratios.write();
        *ratios = req.ratios.clone();
    }
    Json(ApiResponse::ok(req.ratios))
}

// ─── Channel Cooldown Status ──────────────────────────

/// `GET /api/channels/{id}/cooldown` — return cooldown info for a channel.
///
/// Combines the router-level cooldown tracker state with the channel's
/// per-model cooldowns and circuit-breaker status.
pub async fn get_channel_cooldown(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    let in_cooldown = state.router.cooldown_tracker.is_in_cooldown(id);
    let cooldown_remaining_secs = state.router.cooldown_tracker.cooldown_remaining_secs(id);

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "channel_id": id,
        "channel_name": channel.name,
        "in_cooldown": in_cooldown,
        "cooldown_remaining_secs": cooldown_remaining_secs,
        "circuit_open_until": channel.circuit_open_until,
        "model_cooldowns": channel.model_cooldowns,
    }))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_state;

    #[tokio::test]
    async fn get_guardrails_returns_default_config() {
        let state = build_test_state(vec![]);
        let result = get_guardrails_config(State(state)).await;
        assert!(result.ok);
        assert!(!result.data.enabled);
    }

    #[tokio::test]
    async fn update_guardrails_persists_new_config() {
        let state = build_test_state(vec![]);
        let new_config = GuardrailsConfig {
            enabled: true,
            blocked_patterns: vec!["spam".to_string()],
            ..Default::default()
        };
        let result = update_guardrails_config(State(state.clone()), Json(new_config)).await;
        assert!(result.ok);
        assert!(result.data.enabled);

        // Verify it persisted
        let current = get_guardrails_config(State(state)).await;
        assert!(current.data.enabled);
        assert!(current.data.blocked_patterns.contains(&"spam".to_string()));
    }

    #[tokio::test]
    async fn list_redemption_codes_returns_empty_initially() {
        let state = build_test_state(vec![]);
        let result = list_redemption_codes(State(state)).await;
        assert!(result.ok);
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn create_then_list_redemption_code() {
        let state = build_test_state(vec![]);
        let created = create_redemption_code(
            State(state.clone()),
            Json(CreateRedemptionCodeRequest {
                credits_cents: 500,
                expires_at: None,
            }),
        )
        .await;
        assert_eq!(created.data.credits_cents, 500);

        let list = list_redemption_codes(State(state)).await;
        assert_eq!(list.data.len(), 1);
        assert_eq!(list.data[0].code, created.data.code);
    }

    #[tokio::test]
    async fn redeem_code_succeeds_for_valid_code() {
        let state = build_test_state(vec![]);
        let created = create_redemption_code(
            State(state.clone()),
            Json(CreateRedemptionCodeRequest {
                credits_cents: 1000,
                expires_at: None,
            }),
        )
        .await;

        let result = redeem_code(
            State(state),
            Json(RedeemCodeRequest {
                code: created.data.code.clone(),
                user_id: Some("user-1".to_string()),
            }),
        )
        .await
        .expect("redeem should succeed");
        assert!(result.ok);
        assert_eq!(result.data["credits_cents"], 1000);
    }

    #[tokio::test]
    async fn redeem_unknown_code_returns_error() {
        let state = build_test_state(vec![]);
        let response = redeem_code(
            State(state),
            Json(RedeemCodeRequest {
                code: "nonexistent".to_string(),
                user_id: None,
            }),
        )
        .await;
        assert!(response.is_err());
    }

    #[tokio::test]
    async fn delete_redemption_code_returns_no_content() {
        let state = build_test_state(vec![]);
        let created = create_redemption_code(
            State(state.clone()),
            Json(CreateRedemptionCodeRequest {
                credits_cents: 100,
                expires_at: None,
            }),
        )
        .await;

        let response =
            delete_redemption_code(State(state.clone()), Path(created.data.code.clone())).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Deleting again should 404
        let response =
            delete_redemption_code(State(state), Path(created.data.code.clone())).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_notification_config_returns_default() {
        let state = build_test_state(vec![]);
        let result = get_notification_config(State(state)).await;
        assert!(result.ok);
        assert_eq!(result.data.budget_threshold_pct, 80);
    }

    #[tokio::test]
    async fn update_notification_config_persists() {
        let state = build_test_state(vec![]);
        let new_config = NotificationConfig {
            webhook_url: Some("https://example.com/hook".to_string()),
            budget_threshold_pct: 90,
            ..Default::default()
        };
        let result =
            update_notification_config(State(state.clone()), Json(new_config)).await;
        assert!(result.ok);
        assert_eq!(result.data.budget_threshold_pct, 90);

        let current = get_notification_config(State(state)).await;
        assert_eq!(current.data.budget_threshold_pct, 90);
        assert!(current.data.webhook_url.is_some());
    }

    #[tokio::test]
    async fn get_completion_ratios_returns_empty_default() {
        let state = build_test_state(vec![]);
        let result = get_completion_ratios(State(state)).await;
        assert!(result.ok);
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn update_completion_ratios_persists() {
        let state = build_test_state(vec![]);
        let mut ratios = HashMap::new();
        ratios.insert("gpt-4".to_string(), 2.0);
        ratios.insert("claude-3".to_string(), 1.5);

        let result = update_completion_ratios(
            State(state.clone()),
            Json(UpdateCompletionRatiosRequest { ratios }),
        )
        .await;
        assert!(result.ok);
        assert_eq!(result.data.len(), 2);

        let current = get_completion_ratios(State(state)).await;
        assert_eq!(current.data.get("gpt-4"), Some(&2.0));
        assert_eq!(current.data.get("claude-3"), Some(&1.5));
    }

    #[tokio::test]
    async fn get_mcp_health_returns_empty_initially() {
        let state = build_test_state(vec![]);
        let result = get_mcp_health(State(state)).await;
        assert!(result.ok);
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn get_channel_cooldown_returns_not_found_for_missing() {
        let state = build_test_state(vec![]);
        let response =
            get_channel_cooldown(State(state), Path(Uuid::new_v4())).await;
        assert!(response.is_err());
    }
}
