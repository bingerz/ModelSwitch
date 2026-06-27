mod actions;
mod batch;
mod crud;

pub use actions::*;
pub use batch::*;
pub use crud::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelStatus;
    use crate::proxy::AppState;
    use crate::test_helpers::build_test_state;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::Json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn list_channels_returns_empty() {
        let state = build_test_state(vec![]);
        let result = list_channels(State(state)).await;
        assert!(result.ok);
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn create_channel_adds_to_list() {
        let state = build_test_state(vec![]);
        let req = CreateChannelRequest {
            name: "test-channel".to_string(),
            provider: "openai".to_string(),
            priority: 1,
            weight: 100,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential_type: "api_key".to_string(),
            credential_value: "sk-test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            model_mapping: HashMap::new(),
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
        };
        let created = create_channel(State(state.clone()), Json(req))
            .await
            .expect("create_channel should succeed");
        let channel_id = created.0.data.id;

        let list_result = list_channels(State(state)).await;
        assert!(list_result.ok);
        assert_eq!(list_result.data.len(), 1);
        assert_eq!(list_result.data[0].id, channel_id);
    }

    #[tokio::test]
    async fn delete_channel_removes_from_list() {
        let state = build_test_state(vec![]);
        let req = CreateChannelRequest {
            name: "delete-me".to_string(),
            provider: "openai".to_string(),
            priority: 1,
            weight: 100,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential_type: "api_key".to_string(),
            credential_value: "sk-test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            model_mapping: HashMap::new(),
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
        };
        let created = create_channel(State(state.clone()), Json(req))
            .await
            .expect("create_channel should succeed");
        let channel_id = created.0.data.id;

        let delete_response = delete_channel(State(state.clone()), Path(channel_id)).await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_result = list_channels(State(state)).await;
        assert!(list_result.ok);
        assert!(list_result.data.is_empty());
    }

    // ─── Batch operation helpers ───────────────────────────

    /// Helper: create N test channels and return their IDs.
    async fn create_test_channels(state: &Arc<AppState>, count: usize) -> Vec<Uuid> {
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            let req = CreateChannelRequest {
                name: format!("batch-ch-{i}"),
                provider: "openai".to_string(),
                priority: 1,
                weight: 100,
                cost_per_token: None,
                input_cost_per_mtok: None,
                output_cost_per_mtok: None,
                credential_type: "api_key".to_string(),
                credential_value: "sk-test".to_string(),
                base_url: "https://api.openai.com".to_string(),
                model_mapping: HashMap::new(),
                cooldown_minutes: None,
                rpm_limit: None,
                tpm_limit: None,
                account_group: None,
                max_concurrent: None,
            };
            let created = create_channel(State(state.clone()), Json(req))
                .await
                .expect("create_channel should succeed");
            ids.push(created.0.data.id);
        }
        ids
    }

    // ─── Batch enable tests ────────────────────────────────

    #[tokio::test]
    async fn batch_enable_enables_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 3).await;

        // Disable them first
        for &id in &ids {
            let mut ch = state.channel_mgr.get(id).await.unwrap();
            ch.enabled = false;
            ch.status = ChannelStatus::Disabled;
            state.channel_mgr.update(id, ch).await;
        }

        // Batch enable
        let result = batch_enable_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: ids.clone() }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 3);
        assert_eq!(result.data.success, 3);
        assert_eq!(result.data.failed, 0);
        assert!(result.data.errors.is_empty());

        // Verify all channels are enabled
        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(ch.enabled);
            assert_ne!(ch.status, ChannelStatus::Disabled);
        }
    }

    #[tokio::test]
    async fn batch_enable_reports_not_found_errors() {
        let state = build_test_state(vec![]);
        let fake_id = Uuid::new_v4();

        let result = batch_enable_channels(
            State(state),
            Json(BatchChannelRequest { ids: vec![fake_id] }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 1);
        assert_eq!(result.data.success, 0);
        assert_eq!(result.data.failed, 1);
        assert_eq!(result.data.errors[0].id, fake_id);
    }

    // ─── Batch disable tests ───────────────────────────────

    #[tokio::test]
    async fn batch_disable_disables_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;

        let result = batch_disable_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: ids.clone() }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 2);
        assert_eq!(result.data.success, 2);
        assert_eq!(result.data.failed, 0);

        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(!ch.enabled);
            assert_eq!(ch.status, ChannelStatus::Disabled);
        }
    }

    // ─── Batch delete tests ────────────────────────────────

    #[tokio::test]
    async fn batch_delete_removes_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 3).await;

        let result = batch_delete_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: ids.clone() }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 3);
        assert_eq!(result.data.success, 3);
        assert_eq!(result.data.failed, 0);

        let list = list_channels(State(state)).await;
        assert!(list.data.is_empty());
    }

    #[tokio::test]
    async fn batch_delete_with_partial_failures() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;
        let fake_id = Uuid::new_v4();

        let mut all_ids = ids.clone();
        all_ids.push(fake_id);

        let result = batch_delete_channels(
            State(state.clone()),
            Json(BatchChannelRequest { ids: all_ids }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 3);
        assert_eq!(result.data.success, 2);
        assert_eq!(result.data.failed, 1);
        assert_eq!(result.data.errors.len(), 1);
        assert_eq!(result.data.errors[0].id, fake_id);

        let list = list_channels(State(state)).await;
        assert!(list.data.is_empty());
    }

    // ─── Batch update tags tests ───────────────────────────

    #[tokio::test]
    async fn batch_update_tags_adds_tags_to_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;

        let result = batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["production".to_string(), "fast".to_string()],
                remove_tags: vec![],
            }),
        )
        .await
        .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 2);
        assert_eq!(result.data.success, 2);
        assert_eq!(result.data.failed, 0);

        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(ch.tags.contains(&"production".to_string()));
            assert!(ch.tags.contains(&"fast".to_string()));
        }
    }

    #[tokio::test]
    async fn batch_update_tags_removes_tags_from_multiple_channels() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 2).await;

        // First add tags
        batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["production".to_string(), "staging".to_string()],
                remove_tags: vec![],
            }),
        )
        .await;

        // Now remove "production"
        let result = batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec![],
                remove_tags: vec!["production".to_string()],
            }),
        )
        .await
        .0;

        assert_eq!(result.data.success, 2);

        for &id in &ids {
            let ch = state.channel_mgr.get(id).await.unwrap();
            assert!(!ch.tags.contains(&"production".to_string()));
            assert!(ch.tags.contains(&"staging".to_string()));
        }
    }

    #[tokio::test]
    async fn batch_update_tags_does_not_create_duplicates() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;

        // Add tag
        batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["alpha".to_string()],
                remove_tags: vec![],
            }),
        )
        .await;

        // Add same tag again
        batch_update_tags(
            State(state.clone()),
            Json(BatchTagUpdate {
                ids: ids.clone(),
                add_tags: vec!["alpha".to_string()],
                remove_tags: vec![],
            }),
        )
        .await;

        let ch = state.channel_mgr.get(ids[0]).await.unwrap();
        let count = ch.tags.iter().filter(|t| *t == "alpha").count();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn batch_enable_empty_ids_is_noop() {
        let state = build_test_state(vec![]);

        let result = batch_enable_channels(State(state), Json(BatchChannelRequest { ids: vec![] }))
            .await
            .0;

        assert!(result.ok);
        assert_eq!(result.data.total, 0);
        assert_eq!(result.data.success, 0);
        assert_eq!(result.data.failed, 0);
    }

    // ─── update_channel tests ──────────────────────────────

    /// Build a minimal `UpdateChannelRequest` matching the values used by
    /// `create_test_channels`. Tests mutate specific fields as needed.
    fn make_update_req() -> UpdateChannelRequest {
        UpdateChannelRequest {
            name: "test-channel".to_string(),
            provider: "openai".to_string(),
            priority: 1,
            weight: 100,
            cost_per_token: None,
            base_url: "https://api.openai.com".to_string(),
            enabled: true,
            model_mapping: HashMap::new(),
            cooldown_minutes: None,
            credential_value: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
            excluded_models: vec![],
            api_keys: vec![],
            proxy_url: None,
            headers: HashMap::new(),
            max_retries: None,
            models_endpoint: None,
            models_refresh_interval_secs: None,
            tags: vec![],
        }
    }

    #[tokio::test]
    async fn update_channel_changes_name() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        let mut req = make_update_req();
        req.name = "renamed-channel".to_string();

        let result = update_channel(State(state.clone()), Path(channel_id), Json(req))
            .await
            .expect("update_channel should succeed");
        assert_eq!(result.0.data.name, "renamed-channel");

        // Verify persistence via channel_mgr
        let stored = state.channel_mgr.get(channel_id).await.unwrap();
        assert_eq!(stored.name, "renamed-channel");
    }

    #[tokio::test]
    async fn update_channel_disables_channel() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        let mut req = make_update_req();
        req.enabled = false;

        let result = update_channel(State(state.clone()), Path(channel_id), Json(req))
            .await
            .expect("update_channel should succeed");
        assert!(!result.0.data.enabled);
        assert_eq!(result.0.data.status, ChannelStatus::Disabled);

        let stored = state.channel_mgr.get(channel_id).await.unwrap();
        assert!(!stored.enabled);
        assert_eq!(stored.status, ChannelStatus::Disabled);
    }

    #[tokio::test]
    async fn update_channel_reenables_channel() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        // First disable the channel
        let mut disable_req = make_update_req();
        disable_req.enabled = false;
        update_channel(State(state.clone()), Path(channel_id), Json(disable_req))
            .await
            .expect("disable should succeed");

        // Now re-enable it
        let mut enable_req = make_update_req();
        enable_req.enabled = true;
        let result = update_channel(State(state.clone()), Path(channel_id), Json(enable_req))
            .await
            .expect("reenable should succeed");

        assert!(result.0.data.enabled);
        assert_eq!(result.0.data.status, ChannelStatus::Healthy);
    }

    #[tokio::test]
    async fn update_channel_returns_404_for_nonexistent_id() {
        let state = build_test_state(vec![]);
        let req = make_update_req();
        let result = update_channel(State(state), Path(Uuid::new_v4()), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_channel_updates_model_mapping() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        let mut req = make_update_req();
        req.model_mapping
            .insert("gpt-4".to_string(), "gpt-4".to_string());

        let result = update_channel(State(state.clone()), Path(channel_id), Json(req))
            .await
            .expect("update_channel should succeed");
        assert_eq!(
            result.0.data.model_mapping.get("gpt-4"),
            Some(&"gpt-4".to_string())
        );

        let stored = state.channel_mgr.get(channel_id).await.unwrap();
        assert_eq!(
            stored.model_mapping.get("gpt-4"),
            Some(&"gpt-4".to_string())
        );
    }

    // ─── channel_status tests ──────────────────────────────

    #[tokio::test]
    async fn channel_status_returns_status_for_existing() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        let result = channel_status(State(state.clone()), Path(channel_id))
            .await
            .expect("channel_status should succeed for existing channel");

        let data = &result.0.data;
        assert_eq!(data["id"], serde_json::json!(channel_id));
        assert!(data["name"].is_string());
        assert!(data["status"].is_string());
        assert_eq!(data["enabled"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn channel_status_returns_404_for_nonexistent() {
        let state = build_test_state(vec![]);
        let result = channel_status(State(state), Path(Uuid::new_v4())).await;
        assert!(result.is_err());
    }

    // ─── payload rules tests ───────────────────────────────

    #[tokio::test]
    async fn set_payload_rules_stores_and_get_retrieves() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        let mut defaults = HashMap::new();
        defaults.insert("temperature".to_string(), serde_json::json!(0.7));
        let mut overrides = HashMap::new();
        overrides.insert("max_tokens".to_string(), serde_json::json!(1024));

        let rules = crate::config::PayloadRulesConfig {
            defaults,
            overrides,
            strip: vec!["user.metadata".to_string()],
            model_rules: vec![],
        };

        let set_result = set_payload_rules(State(state.clone()), Path(channel_id), Json(rules))
            .await
            .expect("set_payload_rules should succeed");
        assert_eq!(
            set_result.0.data["channel_id"],
            serde_json::json!(channel_id)
        );
        assert_eq!(set_result.0.data["updated"], serde_json::json!(true));

        let get_result = get_payload_rules(State(state), Path(channel_id))
            .await
            .expect("get_payload_rules should succeed");
        let fetched = get_result.0.data;
        assert_eq!(
            fetched.defaults.get("temperature"),
            Some(&serde_json::json!(0.7))
        );
        assert_eq!(
            fetched.overrides.get("max_tokens"),
            Some(&serde_json::json!(1024))
        );
        assert!(fetched.strip.contains(&"user.metadata".to_string()));
    }

    #[tokio::test]
    async fn get_payload_rules_returns_empty_for_no_rules() {
        let state = build_test_state(vec![]);
        let ids = create_test_channels(&state, 1).await;
        let channel_id = ids[0];

        let result = get_payload_rules(State(state), Path(channel_id))
            .await
            .expect("get_payload_rules should succeed for channel without rules");

        let rules = result.0.data;
        assert!(rules.defaults.is_empty());
        assert!(rules.overrides.is_empty());
        assert!(rules.strip.is_empty());
        assert!(rules.model_rules.is_empty());
    }
}
