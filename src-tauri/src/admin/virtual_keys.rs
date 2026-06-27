mod batch;
mod crud;
mod groups;

use crate::proxy::AppState;
use std::sync::Arc;

/// Persist virtual keys to disk. Logs a warning on failure so that one bad
/// write does not crash an otherwise-successful CRUD call.
async fn persist_virtual_keys(state: &Arc<AppState>) {
    if let Err(e) = state.billing.virtual_key_store.persist().await {
        tracing::warn!(error = %e, "Failed to persist virtual keys");
    }
}

pub use batch::*;
pub use crud::*;
pub use groups::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_state;
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use axum::Json;
    use uuid::Uuid;

    fn list_params() -> ListVirtualKeysParams {
        ListVirtualKeysParams {
            page: 1,
            limit: DEFAULT_PAGE_LIMIT,
            search: None,
            group: None,
        }
    }

    fn make_create_req(name: &str) -> CreateVirtualKeyRequest {
        CreateVirtualKeyRequest {
            name: name.to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        }
    }

    #[tokio::test]
    async fn list_virtual_keys_returns_empty() {
        let state = build_test_state(vec![]);
        let result = list_virtual_keys(State(state), Query(list_params())).await;
        assert!(result.data.data.is_empty());
        assert_eq!(result.data.total, 0);
    }

    #[tokio::test]
    async fn create_virtual_key_succeeds() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "test-key".to_string(),
            daily_budget_cents: Some(1000),
            monthly_budget_cents: Some(30000),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = create_virtual_key(State(state), Json(req)).await;
        assert!(result.is_ok());
        let data = result.unwrap().0.data;
        assert!(data["key"].as_str().is_some());
        assert!(data["key"].as_str().unwrap().starts_with("ms-vk-"));
        assert_eq!(data["name"].as_str().unwrap(), "test-key");
    }

    #[tokio::test]
    async fn create_then_delete_virtual_key() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "delete-me".to_string(),
            daily_budget_cents: Some(500),
            monthly_budget_cents: Some(10000),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let create_result = create_virtual_key(State(state.clone()), Json(req))
            .await
            .unwrap();
        let id_str = create_result.0.data["id"].as_str().unwrap().to_string();
        let id: Uuid = id_str.parse().unwrap();

        let delete_response = delete_virtual_key(State(state.clone()), Path(id)).await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let list_result = list_virtual_keys(State(state), Query(list_params())).await;
        assert!(list_result.data.data.is_empty());
    }

    #[tokio::test]
    async fn list_virtual_keys_paginates() {
        let state = build_test_state(vec![]);
        for i in 0..12 {
            let req = CreateVirtualKeyRequest {
                name: format!("key-{i:02}"),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: None,
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        // Page 1 with limit 5 → 5 keys, total 12.
        let params = ListVirtualKeysParams {
            page: 1,
            limit: 5,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state.clone()), Query(params)).await;
        assert_eq!(result.data.data.len(), 5);
        assert_eq!(result.data.total, 12);
        assert_eq!(result.data.page, 1);
        assert_eq!(result.data.limit, 5);

        // Page 3 with limit 5 → only 2 keys (12 - 10).
        let params = ListVirtualKeysParams {
            page: 3,
            limit: 5,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state.clone()), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
        assert_eq!(result.data.total, 12);
    }

    #[tokio::test]
    async fn list_virtual_keys_search_filters_by_name_prefix() {
        let state = build_test_state(vec![]);
        for name in &["alpha-1", "alpha-2", "beta-1"] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: None,
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: Some("alpha".to_string()),
            group: None,
        };
        let result = list_virtual_keys(State(state.clone()), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
        assert_eq!(result.data.total, 2);

        // Case-insensitive match.
        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: Some("ALPHA".to_string()),
            group: None,
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
    }

    #[tokio::test]
    async fn list_virtual_keys_limit_capped_at_max() {
        let state = build_test_state(vec![]);
        let params = ListVirtualKeysParams {
            page: 1,
            limit: 10_000,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.limit, MAX_PAGE_LIMIT);
    }

    #[tokio::test]
    async fn batch_create_generates_named_keys() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 5,
            name_prefix: "team".to_string(),
            daily_budget_cents: Some(100),
            monthly_budget_cents: Some(3000),
            allowed_models: Some(vec!["gpt-4".to_string()]),
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state.clone()), Json(req))
            .await
            .unwrap();
        let created = result.0.data;
        assert_eq!(created.len(), 5);
        assert_eq!(created[0]["name"].as_str().unwrap(), "team-001");
        assert_eq!(created[1]["name"].as_str().unwrap(), "team-002");
        assert_eq!(created[4]["name"].as_str().unwrap(), "team-005");
        // Each key has a plaintext.
        for entry in &created {
            assert!(entry["key"].as_str().unwrap().starts_with("ms-vk-"));
        }

        // Total key count in store should be 5.
        let list = list_virtual_keys(State(state), Query(list_params())).await;
        assert_eq!(list.data.total, 5);
    }

    #[tokio::test]
    async fn batch_create_rejects_zero_count() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 0,
            name_prefix: "team".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn batch_create_rejects_empty_prefix() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 3,
            name_prefix: "   ".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn batch_create_rejects_count_above_max() {
        let state = build_test_state(vec![]);
        let req = BatchCreateVirtualKeyRequest {
            count: 501,
            name_prefix: "team".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        let result = batch_create_virtual_keys(State(state), Json(req)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn groups_summary_returns_empty_when_no_keys() {
        let state = build_test_state(vec![]);
        let result = list_virtual_key_groups(State(state)).await;
        assert!(result.0.data.is_empty());
    }

    #[tokio::test]
    async fn groups_summary_aggregates_by_group() {
        let state = build_test_state(vec![]);
        // Two keys in "engineering", one in "sales", one without a group.
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("eng-2", Some("engineering")),
            ("sales-1", Some("sales")),
            ("ungrouped-1", None),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let result = list_virtual_key_groups(State(state)).await;
        let groups = result.0.data;
        // Only two groups should appear (ungrouped excluded).
        assert_eq!(groups.len(), 2);
        let eng = groups.iter().find(|g| g.group == "engineering").unwrap();
        assert_eq!(eng.key_count, 2);
        let sales = groups.iter().find(|g| g.group == "sales").unwrap();
        assert_eq!(sales.key_count, 1);
    }

    #[tokio::test]
    async fn create_virtual_key_with_group() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "grouped-key".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: Some("marketing".to_string()),
        };
        let result = create_virtual_key(State(state), Json(req)).await;
        assert!(result.is_ok());
        let data = result.unwrap().0.data;
        assert_eq!(data["group"].as_str().unwrap(), "marketing");
    }

    #[tokio::test]
    async fn list_with_group_filter_returns_only_matching_keys() {
        let state = build_test_state(vec![]);
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("eng-2", Some("engineering")),
            ("sales-1", Some("sales")),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: Some("engineering".to_string()),
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 2);
        assert_eq!(result.data.total, 2);
        for key in &result.data.data {
            assert_eq!(key.group.as_deref(), Some("engineering"));
        }
    }

    #[tokio::test]
    async fn list_with_group_filter_returns_empty_when_no_match() {
        let state = build_test_state(vec![]);
        let req = CreateVirtualKeyRequest {
            name: "eng-1".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: None,
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: Some("engineering".to_string()),
        };
        create_virtual_key(State(state.clone()), Json(req))
            .await
            .unwrap();

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: Some("nonexistent".to_string()),
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert!(result.data.data.is_empty());
        assert_eq!(result.data.total, 0);
    }

    #[tokio::test]
    async fn list_with_group_filter_ignores_keys_with_no_group() {
        let state = build_test_state(vec![]);
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("ungrouped-1", None),
            ("ungrouped-2", None),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: Some("engineering".to_string()),
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 1);
        assert_eq!(result.data.total, 1);
        assert_eq!(result.data.data[0].group.as_deref(), Some("engineering"));
    }

    #[tokio::test]
    async fn list_without_group_filter_returns_all() {
        let state = build_test_state(vec![]);
        for (name, group) in &[
            ("eng-1", Some("engineering")),
            ("sales-1", Some("sales")),
            ("ungrouped-1", None),
        ] {
            let req = CreateVirtualKeyRequest {
                name: name.to_string(),
                daily_budget_cents: None,
                monthly_budget_cents: None,
                allowed_models: None,
                denied_models: vec![],
                allowed_ips: None,
                rpm_limit: None,
                tpm_limit: None,
                expires_at: None,
                group: group.map(|s| s.to_string()),
            };
            create_virtual_key(State(state.clone()), Json(req))
                .await
                .unwrap();
        }

        let params = ListVirtualKeysParams {
            page: 1,
            limit: 50,
            search: None,
            group: None,
        };
        let result = list_virtual_keys(State(state), Query(params)).await;
        assert_eq!(result.data.data.len(), 3);
        assert_eq!(result.data.total, 3);
    }
}
