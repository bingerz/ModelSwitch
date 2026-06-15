use super::{ApiResponse, PaginationParams};
use crate::channel::{Channel, Provider};
use crate::log::DispatchLog;
use crate::middleware::error::ApiError;
use crate::proxy::openai::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

// ─── Types ─────────────────────────────────────────────

fn default_hours() -> u64 {
    24
}

#[derive(Debug, Deserialize)]
pub struct UsageParams {
    #[serde(default = "default_hours")]
    pub hours: u64,
}

// ─── Logs & Stats ─────────────────────────────────────

pub async fn get_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PaginationParams>,
) -> Json<super::PaginatedResponse<Vec<DispatchLog>>> {
    let logs = state.logger.list(params.offset, params.limit).await;
    let total = state.logger.total().await;
    Json(super::PaginatedResponse {
        data: logs,
        total,
        offset: params.offset,
        limit: params.limit,
    })
}

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<crate::log::DispatchStats>> {
    let stats = state.logger.stats().await;
    Json(ApiResponse::ok(stats))
}

pub async fn get_cost_stats(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<crate::log::CostStats>> {
    let stats = state.logger.cost_stats().await;
    Json(ApiResponse::ok(stats))
}

pub async fn get_quota(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<crate::quota::QuotaInfo>>> {
    let quotas = state.billing.quota_store.list().await;
    Json(ApiResponse::ok(quotas))
}

pub async fn get_usage_history(
    Query(params): Query<UsageParams>,
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<crate::log::UsageHistory>> {
    let history = state.logger.usage_history(params.hours).await;
    Json(ApiResponse::ok(history))
}

// ─── Operational Endpoints ────────────────────────────

/// Reset a channel's circuit breaker, forcing it back to healthy state.
pub async fn reset_circuit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let channel = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    state.channel_mgr.force_recover(id).await;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "id": channel.id,
        "name": channel.name,
        "status": "healthy",
        "message": "Circuit breaker reset"
    }))))
}

/// Flush all cached responses.
pub async fn flush_cache(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<serde_json::Value>> {
    state.cache.request_cache.flush();
    let count = state.cache.request_cache.len();
    Json(ApiResponse::ok(serde_json::json!({
        "flushed": true,
        "remaining": count
    })))
}

/// Reload configuration from disk and update channels.
pub async fn reload_config(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    match crate::config::AppConfig::load() {
        Ok(new_config) => {
            let channels = state.channel_mgr.list().await;
            let mut created = 0u32;
            let mut updated = 0u32;
            let mut removed = 0u32;

            // Update or create channels from new config
            for cc in &new_config.channels {
                let id = match Uuid::parse_str(&cc.id) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                if let Some(existing) = state.channel_mgr.get(id).await {
                    let mut ch = existing;
                    ch.name = cc.name.clone();
                    ch.provider = Provider::from_str(&cc.provider);
                    ch.priority = cc.priority;
                    ch.weight = cc.weight;
                    ch.cost_per_token = cc.cost_per_token;
                    ch.input_cost_per_mtok = cc.input_cost_per_mtok;
                    ch.output_cost_per_mtok = cc.output_cost_per_mtok;
                    ch.base_url = cc.base_url.clone();
                    ch.model_mapping = cc.model_mapping.clone();
                    ch.enabled = cc.enabled;
                    ch.cooldown_minutes = cc.cooldown_minutes;
                    ch.rpm_limit = cc.rpm_limit;
                    ch.tpm_limit = cc.tpm_limit;
                    ch.updated_at = chrono::Utc::now();
                    let _ = state.channel_mgr.update(id, ch).await;
                    updated += 1;
                } else {
                    let new_channel = Channel::from_config(cc);
                    let _ = state.channel_mgr.create(new_channel).await;
                    created += 1;
                }
            }

            // Remove channels no longer in config
            let config_ids: Vec<Uuid> = new_config
                .channels
                .iter()
                .filter_map(|c| Uuid::parse_str(&c.id).ok())
                .collect();
            for ch in &channels {
                if !config_ids.contains(&ch.id) {
                    state.channel_mgr.delete(ch.id).await;
                    state.billing.quota_store.delete(ch.id).await;
                    removed += 1;
                }
            }

            // Update rate limits
            for cc in &new_config.channels {
                let id = match Uuid::parse_str(&cc.id) {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                if let Some(rpm) = cc.rpm_limit {
                    state.limits.rate_limiter.set_channel_rpm_limit(id, rpm);
                }
                if let Some(tpm) = cc.tpm_limit {
                    state.limits.rate_limiter.set_channel_tpm_limit(id, tpm);
                }
            }

            tracing::info!(
                "Config reload: {updated} updated, {created} created, {removed} removed"
            );
            Ok(Json(ApiResponse::ok(serde_json::json!({
                "reloaded": true,
                "updated": updated,
                "created": created,
                "removed": removed
            }))))
        }
        Err(e) => {
            tracing::error!("Config reload failed: {}", e);
            Err(
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to reload config")
                    .into_response(),
            )
        }
    }
}
