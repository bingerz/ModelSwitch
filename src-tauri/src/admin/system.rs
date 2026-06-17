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

// ─── Cache Statistics ────────────────────────────────────

/// Cache statistics: size, capacity, mode, hit/miss counts.
pub async fn cache_stats(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<serde_json::Value>> {
    let cache_len = state.cache.request_cache.len();
    let cache_mode = format!("{:?}", state.cache.request_cache.mode());

    // Read Prometheus counters for hit/miss data
    // We gather all metrics and extract the values
    let metrics_output = crate::metrics::render();

    // Parse hit/miss counts from Prometheus text format
    let hits = parse_counter_value(&metrics_output, "modelswitch_cache_hits_total");
    let misses = parse_counter_value(&metrics_output, "modelswitch_cache_misses_total");
    let evictions = parse_counter_value(&metrics_output, "modelswitch_cache_evictions_total");

    let total_requests = hits + misses;
    let hit_rate = if total_requests > 0 {
        (hits as f64 / total_requests as f64 * 100.0).round() as u64
    } else {
        0
    };

    Json(ApiResponse::ok(serde_json::json!({
        "entries": cache_len,
        "mode": cache_mode,
        "hits": hits,
        "misses": misses,
        "evictions": evictions,
        "hit_rate_percent": hit_rate,
        "total_requests": total_requests,
    })))
}

/// Parse a counter value from Prometheus text format output.
fn parse_counter_value(metrics_text: &str, metric_name: &str) -> u64 {
    for line in metrics_text.lines() {
        if line.starts_with(metric_name) && !line.contains("_bucket") && !line.contains("_sum") && !line.contains("_count") {
            // Format: "metric_name 123" or "metric_name{labels} 123"
            if let Some(value_str) = line.split_whitespace().last() {
                if let Ok(value) = value_str.parse::<u64>() {
                    return value;
                }
            }
        }
    }
    0
}

// ─── Gateway Info ──────────────────────────────────────────

/// Gateway runtime info: version, uptime, configuration summary.
pub async fn gateway_info(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<serde_json::Value>> {
    let uptime_secs = state.started_at.elapsed().as_secs();
    let channels = state.channel_mgr.list().await;
    let total_channels = channels.len();
    let healthy_channels = channels
        .iter()
        .filter(|c| c.enabled && c.status == crate::channel::ChannelStatus::Healthy)
        .count();
    let active_requests = state.router.active_requests.total();

    // Format uptime as human-readable
    let uptime_formatted = format_uptime(uptime_secs);

    Json(ApiResponse::ok(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime_secs,
        "uptime_formatted": uptime_formatted,
        "total_channels": total_channels,
        "healthy_channels": healthy_channels,
        "active_requests": active_requests,
        "cache_entries": state.cache.request_cache.len(),
        "routing_strategy": state.gateway.routing_strategy,
        "max_retries": state.gateway.max_retries,
    })))
}

/// Format seconds into a human-readable uptime string.
fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    if days > 0 {
        format!("{}d {}h {}m", days, hours, minutes)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_counter() {
        let metrics = "modelswitch_cache_hits_total 42\n";
        assert_eq!(parse_counter_value(metrics, "modelswitch_cache_hits_total"), 42);
    }

    #[test]
    fn parse_counter_not_found() {
        let metrics = "some_other_metric 10\n";
        assert_eq!(parse_counter_value(metrics, "modelswitch_cache_hits_total"), 0);
    }

    #[test]
    fn parse_skips_histogram_lines() {
        let metrics = "modelswitch_cache_hits_total 5\nmodelswitch_cache_hits_total_sum 10.5\nmodelswitch_cache_hits_total_count 5\n";
        assert_eq!(parse_counter_value(metrics, "modelswitch_cache_hits_total"), 5);
    }

    #[test]
    fn format_seconds() {
        assert_eq!(format_uptime(45), "45s");
    }

    #[test]
    fn format_minutes() {
        assert_eq!(format_uptime(125), "2m 5s");
    }

    #[test]
    fn format_hours() {
        assert_eq!(format_uptime(3661), "1h 1m 1s");
    }

    #[test]
    fn format_days() {
        assert_eq!(format_uptime(90061), "1d 1h 1m");
    }
}
