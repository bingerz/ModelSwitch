//! Employee self-service portal endpoints.
//!
//! These routes are NOT protected by `admin_auth_middleware`. They authenticate
//! via the employee's virtual key (`Authorization: Bearer ms-vk-xxx`).

use crate::log::DispatchLog;
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::ApiResponse;

// ─── Types ─────────────────────────────────────────────

/// Usage info returned by `GET /api/portal/usage`.
/// Never exposes `key_hash` — only display-safe fields.
#[derive(Debug, Serialize)]
pub struct PortalUsage {
    pub key_name: String,
    pub key_prefix: String,
    pub daily_budget_cents: Option<u64>,
    pub monthly_budget_cents: Option<u64>,
    pub daily_spent_cents: u64,
    pub monthly_spent_cents: u64,
    pub total_spent_cents: u64,
    pub rpm_limit: Option<u32>,
    pub tpm_limit: Option<u32>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub allowed_models: Option<Vec<String>>,
    pub is_active: bool,
    pub is_expired: bool,
}

/// Query params for `GET /api/portal/logs`.
#[derive(Debug, Deserialize)]
pub struct PortalLogsParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

// ─── Auth helper ───────────────────────────────────────

/// Extract the bearer token from the `Authorization` header and validate it
/// against the virtual key store. Returns the matching `VirtualKey` or an
/// error response.
///
/// Uses `validate_any` so that disabled/expired keys can still view their
/// own status in the portal.
async fn authenticate_portal(
    headers: &HeaderMap,
    state: &Arc<AppState>,
) -> Result<crate::virtual_key::VirtualKey, axum::response::Response> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid Authorization scheme"))?;

    if token.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Empty bearer token",
        ));
    }

    let vk = state
        .billing
        .virtual_key_store
        .validate_any(token.trim())
        .await
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid virtual key"))?;

    Ok(vk)
}

// ─── Endpoints ─────────────────────────────────────────

/// GET /api/portal/usage — returns the authenticated key's usage and budget info.
pub async fn portal_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PortalUsage>>, axum::response::Response> {
    let vk = authenticate_portal(&headers, &state).await?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let this_month = chrono::Local::now().format("%Y-%m").to_string();

    let daily_spent = if vk.spend.today.date == today {
        vk.spend.today.cents
    } else {
        0
    };
    let monthly_spent = if vk.spend.this_month.month == this_month {
        vk.spend.this_month.cents
    } else {
        0
    };

    let is_expired = vk.is_expired();
    Ok(Json(ApiResponse::ok(PortalUsage {
        key_name: vk.name,
        key_prefix: vk.key_prefix,
        daily_budget_cents: vk.daily_budget_cents,
        monthly_budget_cents: vk.monthly_budget_cents,
        daily_spent_cents: daily_spent,
        monthly_spent_cents: monthly_spent,
        total_spent_cents: vk.spend.total_cents,
        rpm_limit: vk.rpm_limit,
        tpm_limit: vk.tpm_limit,
        expires_at: vk.expires_at,
        allowed_models: vk.allowed_models,
        is_active: vk.enabled,
        is_expired,
    })))
}

/// GET /api/portal/logs — returns the authenticated key's recent dispatch logs.
pub async fn portal_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<PortalLogsParams>,
) -> Result<Json<ApiResponse<Vec<DispatchLog>>>, axum::response::Response> {
    let vk = authenticate_portal(&headers, &state).await?;

    let limit = params.limit.clamp(1, 100);
    let key_id = vk.id.to_string();
    let logs = state.logger.list_by_key(&key_id, 0, limit).await;

    Ok(Json(ApiResponse::ok(logs)))
}

/// GET /api/portal/test — simple connectivity test.
pub async fn portal_test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, axum::response::Response> {
    let _vk = authenticate_portal(&headers, &state).await?;
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "message": "Your key is valid"
    }))))
}

// ─── Tests ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_state;

    fn auth_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {token}"))
                .expect("valid header value"),
        );
        headers
    }

    #[tokio::test]
    async fn portal_usage_returns_key_info() {
        let state = build_test_state(vec![]);
        let (_vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "employee-test".into(),
                Some(100),
                Some(2000),
                Some(vec!["deepseek-chat".into()]),
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;

        let headers = auth_headers(&plaintext);
        let result = portal_usage(State(state), headers).await;
        assert!(result.is_ok());
        let json_response = result.unwrap();
        let usage = &json_response.0.data;
        assert_eq!(usage.key_name, "employee-test");
        assert_eq!(usage.daily_budget_cents, Some(100));
        assert_eq!(usage.monthly_budget_cents, Some(2000));
        assert!(usage.is_active);
        assert!(!usage.is_expired);
        assert_eq!(
            usage.allowed_models.as_deref(),
            Some(&["deepseek-chat".to_string()][..])
        );
    }

    #[tokio::test]
    async fn portal_usage_rejects_invalid_token() {
        let state = build_test_state(vec![]);
        let headers = auth_headers("ms-vk-nonexistentkey");
        let result = portal_usage(State(state), headers).await;
        match result {
            Err(response) => assert_eq!(response.status(), StatusCode::UNAUTHORIZED),
            Ok(_) => panic!("expected error for invalid token"),
        }
    }

    #[tokio::test]
    async fn portal_usage_rejects_missing_header() {
        let state = build_test_state(vec![]);
        let headers = HeaderMap::new();
        let result = portal_usage(State(state), headers).await;
        match result {
            Err(response) => assert_eq!(response.status(), StatusCode::UNAUTHORIZED),
            Ok(_) => panic!("expected error for missing header"),
        }
    }

    #[tokio::test]
    async fn portal_usage_shows_disabled_key_status() {
        let state = build_test_state(vec![]);
        let (vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "disabled-emp".into(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        // Disable the key
        state
            .billing
            .virtual_key_store
            .update(
                vk.id,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await;

        let headers = auth_headers(&plaintext);
        let result = portal_usage(State(state), headers).await;
        // validate_any still returns the key even when disabled
        assert!(result.is_ok());
        let usage = &result.unwrap().0.data;
        assert!(!usage.is_active);
    }

    #[tokio::test]
    async fn portal_logs_returns_empty_for_new_key() {
        let state = build_test_state(vec![]);
        let (_vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "no-logs".into(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;

        let headers = auth_headers(&plaintext);
        let params = PortalLogsParams { limit: 10 };
        let result = portal_logs(State(state), headers, Query(params)).await;
        assert!(result.is_ok());
        let logs = &result.unwrap().0.data;
        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn portal_test_succeeds_with_valid_key() {
        let state = build_test_state(vec![]);
        let (_vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "test-conn".into(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;

        let headers = auth_headers(&plaintext);
        let result = portal_test(State(state), headers).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn portal_logs_limit_clamped_to_max() {
        let state = build_test_state(vec![]);
        let (_vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "clamp-test".into(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;

        let headers = auth_headers(&plaintext);
        // Request way more than the max — should not panic
        let params = PortalLogsParams { limit: 9999 };
        let result = portal_logs(State(state), headers, Query(params)).await;
        assert!(result.is_ok());
    }
}
