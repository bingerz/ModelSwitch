use crate::admin::ApiResponse;
use crate::channel::ChannelStatus;
use crate::proxy::AppState;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Request body for batch operations that only need channel IDs.
#[derive(Debug, Deserialize)]
pub struct BatchChannelRequest {
    pub ids: Vec<Uuid>,
}

/// Request body for batch tag updates.
#[derive(Debug, Deserialize)]
pub struct BatchTagUpdate {
    pub ids: Vec<Uuid>,
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

/// Summary of a batch operation result.
#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub errors: Vec<BatchErrorEntry>,
}

/// A single error within a batch result.
#[derive(Debug, Serialize)]
pub struct BatchErrorEntry {
    pub id: Uuid,
    pub error: String,
}

impl BatchResult {
    pub(super) fn new(total: usize) -> Self {
        Self {
            total,
            success: 0,
            failed: 0,
            errors: Vec::new(),
        }
    }
}

/// POST /api/channels/batch/enable — enable multiple channels at once.
pub async fn batch_enable_channels(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchChannelRequest>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        match state.channel_mgr.get(id).await {
            Some(mut channel) => {
                channel.enabled = true;
                if channel.status == ChannelStatus::Disabled {
                    channel.status = ChannelStatus::Healthy;
                }
                channel.updated_at = chrono::Utc::now();
                if state
                    .channel_mgr
                    .update(id, channel.clone())
                    .await
                    .is_some()
                {
                    result.success += 1;
                    state
                        .audit_log
                        .record(
                            "channel.batch_enable",
                            "admin-api",
                            &id.to_string(),
                            serde_json::json!({
                                "name": channel.name,
                            }),
                        )
                        .await;
                } else {
                    result.failed += 1;
                    result.errors.push(BatchErrorEntry {
                        id,
                        error: "Update failed".to_string(),
                    });
                }
            }
            None => {
                result.failed += 1;
                result.errors.push(BatchErrorEntry {
                    id,
                    error: "Channel not found".to_string(),
                });
            }
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}

/// POST /api/channels/batch/disable — disable multiple channels at once.
pub async fn batch_disable_channels(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchChannelRequest>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        match state.channel_mgr.get(id).await {
            Some(mut channel) => {
                channel.enabled = false;
                channel.status = ChannelStatus::Disabled;
                channel.updated_at = chrono::Utc::now();
                if state
                    .channel_mgr
                    .update(id, channel.clone())
                    .await
                    .is_some()
                {
                    result.success += 1;
                    state
                        .audit_log
                        .record(
                            "channel.batch_disable",
                            "admin-api",
                            &id.to_string(),
                            serde_json::json!({
                                "name": channel.name,
                            }),
                        )
                        .await;
                } else {
                    result.failed += 1;
                    result.errors.push(BatchErrorEntry {
                        id,
                        error: "Update failed".to_string(),
                    });
                }
            }
            None => {
                result.failed += 1;
                result.errors.push(BatchErrorEntry {
                    id,
                    error: "Channel not found".to_string(),
                });
            }
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}

/// POST /api/channels/batch/delete — delete multiple channels at once.
pub async fn batch_delete_channels(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchChannelRequest>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        // Capture channel name for audit before deletion
        let channel_name = state
            .channel_mgr
            .get(id)
            .await
            .map(|ch| ch.name)
            .unwrap_or_default();

        // Clean up credential before deleting
        if let Some(channel) = state.channel_mgr.get(id).await {
            let username = &channel.credential.key_ref;
            if let Err(e) = state.credential_store.delete("modelswitch", username) {
                tracing::warn!("Failed to delete credential for {}: {}", username, e);
            }
        }

        if state.channel_mgr.delete(id).await {
            state.billing.quota_store.delete(id).await;
            state.router.cooldown_tracker.remove(id);
            result.success += 1;
            state
                .audit_log
                .record(
                    "channel.batch_delete",
                    "admin-api",
                    &id.to_string(),
                    serde_json::json!({
                        "name": channel_name,
                    }),
                )
                .await;
        } else {
            result.failed += 1;
            result.errors.push(BatchErrorEntry {
                id,
                error: "Channel not found".to_string(),
            });
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}

/// PUT /api/channels/batch/tags — add and/or remove tags on multiple channels.
pub async fn batch_update_tags(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchTagUpdate>,
) -> Json<ApiResponse<BatchResult>> {
    let total = req.ids.len();
    let mut result = BatchResult::new(total);

    for id in req.ids {
        match state.channel_mgr.get(id).await {
            Some(mut channel) => {
                // Remove requested tags
                channel.tags.retain(|t| !req.remove_tags.contains(t));
                // Add requested tags (avoid duplicates)
                for tag in &req.add_tags {
                    if !channel.tags.contains(tag) {
                        channel.tags.push(tag.clone());
                    }
                }
                channel.updated_at = chrono::Utc::now();
                let tags_snapshot = channel.tags.clone();
                let name_snapshot = channel.name.clone();
                if state.channel_mgr.update(id, channel).await.is_some() {
                    result.success += 1;
                    state
                        .audit_log
                        .record(
                            "channel.batch_update_tags",
                            "admin-api",
                            &id.to_string(),
                            serde_json::json!({
                                "name": name_snapshot,
                                "tags": tags_snapshot,
                                "added": req.add_tags,
                                "removed": req.remove_tags,
                            }),
                        )
                        .await;
                } else {
                    result.failed += 1;
                    result.errors.push(BatchErrorEntry {
                        id,
                        error: "Update failed".to_string(),
                    });
                }
            }
            None => {
                result.failed += 1;
                result.errors.push(BatchErrorEntry {
                    id,
                    error: "Channel not found".to_string(),
                });
            }
        }
    }

    if result.success > 0 {
        state.channel_mgr.persist().await;
    }

    Json(ApiResponse::ok(result))
}
