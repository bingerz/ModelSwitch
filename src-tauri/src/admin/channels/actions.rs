use crate::admin::ApiResponse;
use crate::channel::Provider;
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use std::sync::Arc;
use uuid::Uuid;

pub async fn ping_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    let api_key = state.channel_mgr.get_credential(id).await.ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to retrieve credential",
        )
    })?;

    // Use provider-appropriate ping endpoint and auth
    let (url, auth_headers) = match &channel.provider {
        Provider::Anthropic => (
            format!("{}/v1/messages", channel.base_url.trim_end_matches('/')),
            vec![
                ("x-api-key", api_key.clone()),
                ("anthropic-version", "2023-06-01".to_string()),
            ],
        ),
        _ => (
            format!("{}/v1/models", channel.base_url.trim_end_matches('/')),
            vec![("Authorization", format!("Bearer {}", api_key))],
        ),
    };

    let pool_guard = state.http_pool.get();
    let mut req_builder = pool_guard
        .get(&url)
        .timeout(std::time::Duration::from_secs(10));

    for (key, value) in &auth_headers {
        req_builder = req_builder.header(*key, value.as_str());
    }

    let start = std::time::Instant::now();
    let resp = req_builder.send().await;

    match resp {
        Ok(r) => {
            let latency = start.elapsed().as_millis() as u64;
            let success = r.status().is_success();
            Ok(Json(ApiResponse::ok(serde_json::json!({
                "success": success,
                "status": r.status().as_u16(),
                "latency_ms": latency,
            }))))
        }
        Err(e) => Ok(Json(ApiResponse::ok(serde_json::json!({
            "success": false,
            "error": e.to_string(),
        })))),
    }
}

pub async fn channel_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "id": channel.id,
        "name": channel.name,
        "status": channel.status,
        "enabled": channel.enabled,
        "circuit_open_until": channel.circuit_open_until,
    }))))
}

/// Set payload rules for a channel at runtime.
pub async fn set_payload_rules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(rules): Json<crate::config::PayloadRulesConfig>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    // Verify channel exists
    state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    use crate::proxy::payload_rules::PayloadRules;
    let model_rules = rules.model_rules.clone();
    state.limits.payload_rules.add(
        id,
        PayloadRules {
            defaults: rules.defaults,
            overrides: rules.overrides,
            strip: rules.strip,
        },
    );
    if !model_rules.is_empty() {
        state.limits.payload_rules.set_model_rules(id, model_rules);
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "channel_id": id,
        "updated": true
    }))))
}

/// Get the current payload rules for a channel.
///
/// Returns the stored rules, or an empty default if none have been
/// configured. Returns 404 if the channel itself does not exist.
pub async fn get_payload_rules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<crate::config::PayloadRulesConfig>>, axum::response::Response> {
    // Verify channel exists
    state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    let rules = state
        .limits
        .payload_rules
        .get_full(id)
        .map(|full| crate::config::PayloadRulesConfig {
            defaults: full.channel.defaults,
            overrides: full.channel.overrides,
            strip: full.channel.strip,
            model_rules: full.model_rules,
        })
        .unwrap_or_default();

    Ok(Json(ApiResponse::ok(rules)))
}
