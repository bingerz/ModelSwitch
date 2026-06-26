//! Integration tests for ModelSwitch API pipeline.
//!
//! These tests exercise the full HTTP path: router -> middleware -> handler -> response.
//! They use `tower::ServiceExt::oneshot` to send requests through the router
//! without needing a real TCP listener.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use model_switch_lib::proxy::AppState;
use model_switch_lib::server::build_router;
use model_switch_lib::test_helpers::{
    build_test_state, build_test_state_with_admin_token, build_test_state_with_rbac, Role,
};
use serde_json::Value;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Maximum bytes to read from a response body.
const BODY_LIMIT: usize = 1024 * 1024;

/// Read and parse the response body as JSON.
async fn body_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), BODY_LIMIT)
        .await
        .expect("failed to read response body");
    serde_json::from_slice(&bytes).expect("response body is not valid JSON")
}

// ---------------------------------------------------------------------------
// Test 1: healthz endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn healthz_returns_200() {
    let state = build_test_state(vec![]);
    let app = build_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["status"], "ok");
}

// ---------------------------------------------------------------------------
// Test 2: metrics endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_endpoint_returns_200() {
    let state = build_test_state(vec![]);
    let app = build_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // The metrics endpoint returns Prometheus text format. When no metrics
    // have been recorded (fresh test state), the body may be empty, so we
    // only verify the content type header is the Prometheus exposition format.
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/plain"),
        "metrics endpoint should return text/plain content type, got: {content_type}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: admin API requires auth token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_api_requires_auth_token() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/channels")
                // Unique IP to avoid interference with other rate-limited tests.
                .header("X-Real-IP", "198.51.100.10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Test 4: admin API accepts valid token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_api_accepts_valid_token() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/channels")
                .header("X-Real-IP", "198.51.100.20")
                .header("Authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["ok"], true);
    // With no channels configured, the data array should be empty.
    assert!(json["data"].is_array());
}

// ---------------------------------------------------------------------------
// Test 5: admin API rejects wrong token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_api_rejects_wrong_token() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/channels")
                .header("X-Real-IP", "198.51.100.30")
                .header("Authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Test 6: virtual key CRUD lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn virtual_key_crud_lifecycle() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    // Step 1: POST /api/virtual-keys -> create a key.
    let create_body = serde_json::json!({"name": "crud-test-key"}).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer test-secret")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.40")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["name"], "crud-test-key");
    let key_id = json["data"]["id"]
        .as_str()
        .expect("id should be a string")
        .to_string();
    let plaintext = json["data"]["key"]
        .as_str()
        .expect("key should be a string")
        .to_string();
    assert!(
        plaintext.starts_with("ms-vk-"),
        "plaintext key should start with ms-vk-"
    );

    // Step 2: GET /api/virtual-keys -> list should contain the key.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer test-secret")
                .header("X-Real-IP", "198.51.100.40")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["data"]["total"], 1);
    assert_eq!(json["data"]["data"].as_array().unwrap().len(), 1);
    assert_eq!(json["data"]["data"][0]["name"], "crud-test-key");

    // Step 3: DELETE /api/virtual-keys/{id} -> 204 No Content.
    let delete_uri = format!("/api/virtual-keys/{key_id}");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .header("Authorization", "Bearer test-secret")
                .header("X-Real-IP", "198.51.100.40")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Step 4: GET /api/virtual-keys -> list should be empty.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer test-secret")
                .header("X-Real-IP", "198.51.100.40")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["data"]["total"], 0);
    assert!(json["data"]["data"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Test 7: rate limiting blocks after three failures
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rate_limiting_blocks_after_three_failures() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    // Use a unique IP for this test so the global rate-limiter state does
    // not interfere with or get interfered by other tests.
    let ip = "198.51.100.50";

    // Three failed attempts -> three 401 responses.
    for i in 0..3 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/channels")
                    .header("Authorization", "Bearer wrong")
                    .header("X-Real-IP", ip)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {} should return 401",
            i + 1
        );
    }

    // Fourth attempt -> 429 Too Many Requests.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/channels")
                .header("Authorization", "Bearer wrong")
                .header("X-Real-IP", ip)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "fourth attempt after three failures should be rate-limited"
    );
}

// ---------------------------------------------------------------------------
// Test 8: virtual keys pagination
// ---------------------------------------------------------------------------

#[tokio::test]
async fn virtual_keys_pagination() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    // Create 5 keys.
    for i in 0..5 {
        let body = serde_json::json!({"name": format!("page-test-{i}")}).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/virtual-keys")
                    .header("Authorization", "Bearer test-secret")
                    .header("Content-Type", "application/json")
                    .header("X-Real-IP", "198.51.100.60")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "key {} should be created",
            i
        );
    }

    // Page 1, limit 3 -> 3 keys, total 5.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys?page=1&limit=3")
                .header("Authorization", "Bearer test-secret")
                .header("X-Real-IP", "198.51.100.60")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["data"]["total"], 5);
    assert_eq!(json["data"]["page"], 1);
    assert_eq!(json["data"]["limit"], 3);
    assert_eq!(json["data"]["data"].as_array().unwrap().len(), 3);

    // Page 2, limit 3 -> 2 keys (5 - 3).
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys?page=2&limit=3")
                .header("Authorization", "Bearer test-secret")
                .header("X-Real-IP", "198.51.100.60")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["data"]["total"], 5);
    assert_eq!(json["data"]["data"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// Test 9: portal endpoint requires valid key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn portal_endpoint_requires_valid_key() {
    let state: Arc<AppState> = build_test_state(vec![]);

    // Create a virtual key directly through the store so we have the plaintext.
    let (_vk, plaintext_key) = state
        .billing
        .virtual_key_store
        .create(
            "portal-user".into(),
            Some(100),
            Some(3000),
            None,
            vec![],
            vec![],
            None,
            None,
            None,
            None,
        )
        .await;

    let app = build_router(Arc::clone(&state), None);

    // Without Authorization header -> 401.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/portal/usage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With a garbage key -> 401.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/portal/usage")
                .header("Authorization", "Bearer ms-vk-garbage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // With a valid key -> 200.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/portal/usage")
                .header("Authorization", format!("Bearer {plaintext_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["key_name"], "portal-user");
    assert_eq!(json["data"]["daily_budget_cents"], 100);
    assert_eq!(json["data"]["is_active"], true);
}

// ---------------------------------------------------------------------------
// Test 10: batch create keys
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_create_keys() {
    let state = build_test_state_with_admin_token(vec![], "test-secret");
    let app = build_router(state, None);

    let batch_body = serde_json::json!({
        "name_prefix": "batchtest",
        "count": 5,
        "daily_budget_cents": 100
    })
    .to_string();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/virtual-keys/batch")
                .header("Authorization", "Bearer test-secret")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.80")
                .body(Body::from(batch_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["ok"], true);
    let keys = json["data"].as_array().expect("data should be an array");
    assert_eq!(keys.len(), 5, "batch create should return 5 keys");

    // Each key should have a plaintext and zero-padded name.
    for (i, key) in keys.iter().enumerate() {
        let name = key["name"].as_str().expect("name should be present");
        assert!(
            name.starts_with("batchtest-"),
            "key {} name should start with batchtest-",
            i
        );
        let plaintext = key["key"].as_str().expect("key should be present");
        assert!(
            plaintext.starts_with("ms-vk-"),
            "key {} plaintext should start with ms-vk-",
            i
        );
        assert_eq!(key["daily_budget_cents"], 100);
    }

    // Verify names are numbered correctly.
    assert_eq!(keys[0]["name"], "batchtest-001");
    assert_eq!(keys[4]["name"], "batchtest-005");

    // List endpoint should show all 5 keys.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer test-secret")
                .header("X-Real-IP", "198.51.100.80")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["data"]["total"], 5);
}

// ---------------------------------------------------------------------------
// Test 11: virtual key budget exceeded blocks request
// ---------------------------------------------------------------------------

#[tokio::test]
async fn virtual_key_budget_exceeded_blocks_request() {
    let state: Arc<AppState> = build_test_state(vec![]);

    // Create a virtual key with a 1-cent daily budget.
    let (vk, plaintext_key) = state
        .billing
        .virtual_key_store
        .create(
            "budget-limited".into(),
            Some(1),
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

    // Record enough spend to exceed the 1-cent daily budget.
    state
        .billing
        .virtual_key_store
        .accumulate_spend(vk.id, 5)
        .await;

    let app = build_router(Arc::clone(&state), None);

    // A proxy request with the exhausted key must be rejected.
    // validate() returns None for over-budget keys, and the middleware
    // maps that to 401 Unauthorized ("Invalid or exhausted virtual key").
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("Authorization", format!("Bearer {plaintext_key}"))
                .header("X-Real-IP", "198.51.100.90")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "over-budget key must be rejected at the middleware"
    );
}

// ---------------------------------------------------------------------------
// Test 12: virtual key expired blocks request
// ---------------------------------------------------------------------------

#[tokio::test]
async fn virtual_key_expired_blocks_request() {
    let state: Arc<AppState> = build_test_state(vec![]);

    // Create a virtual key that expired 5 minutes ago.
    let past = chrono::Utc::now() - chrono::Duration::minutes(5);
    let (_vk, plaintext_key) = state
        .billing
        .virtual_key_store
        .create(
            "expired-key".into(),
            None,
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            Some(past),
            None,
        )
        .await;

    let app = build_router(Arc::clone(&state), None);

    // validate() returns None for expired keys, so the middleware returns
    // 401 Unauthorized ("Invalid or exhausted virtual key").
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("Authorization", format!("Bearer {plaintext_key}"))
                .header("X-Real-IP", "198.51.100.91")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "expired key must be rejected at the middleware"
    );
}

// ---------------------------------------------------------------------------
// Test 13: virtual key IP restriction enforced
// ---------------------------------------------------------------------------

#[tokio::test]
async fn virtual_key_ip_restriction_enforced() {
    let state: Arc<AppState> = build_test_state(vec![]);

    // Create a virtual key restricted to a specific IP.
    let (_vk, plaintext_key) = state
        .billing
        .virtual_key_store
        .create(
            "ip-restricted".into(),
            None,
            None,
            None,
            vec![],
            vec!["10.0.0.5".to_string()],
            None,
            None,
            None,
            None,
        )
        .await;

    let app = build_router(Arc::clone(&state), None);

    // A request from a non-allowed IP must be rejected with 403 Forbidden.
    // The key itself is valid (validate() returns Some), but the IP check
    // in the middleware returns FORBIDDEN.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("Authorization", format!("Bearer {plaintext_key}"))
                .header("X-Real-IP", "192.168.1.99")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "request from non-allowed IP must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Test 14: RBAC — auditor role can read but cannot write
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auditor_role_can_read_but_cannot_write() {
    let state = build_test_state_with_rbac(
        vec![],
        None,
        vec![("auditor-tok".to_string(), Role::Auditor)],
    );
    let app = build_router(state, None);

    // GET /api/virtual-keys with auditor token -> 200 (read permitted).
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer auditor-tok")
                .header("X-Real-IP", "198.51.100.100")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "auditor should be able to read virtual keys"
    );

    // POST /api/virtual-keys with auditor token -> 403 (write denied by RBAC).
    let create_body = serde_json::json!({"name": "auditor-should-fail"}).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer auditor-tok")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.101")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "auditor must not be able to create virtual keys"
    );

    // GET /api/channels with auditor token -> 200 (read permitted).
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/channels")
                .header("Authorization", "Bearer auditor-tok")
                .header("X-Real-IP", "198.51.100.102")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "auditor should be able to read channels"
    );
}

// ---------------------------------------------------------------------------
// Test 15: RBAC — key manager can manage virtual keys but not channels
// ---------------------------------------------------------------------------

#[tokio::test]
async fn key_manager_can_manage_virtual_keys_but_not_channels() {
    let state =
        build_test_state_with_rbac(vec![], None, vec![("km-tok".to_string(), Role::KeyManager)]);
    let app = build_router(state, None);

    // GET /api/virtual-keys -> 200 (read permitted).
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer km-tok")
                .header("X-Real-IP", "198.51.100.110")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "key_manager should be able to read virtual keys"
    );

    // POST /api/virtual-keys -> 200 (write to virtual-keys permitted).
    let create_body = serde_json::json!({"name": "km-created-key"}).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer km-tok")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.111")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "key_manager should be able to create virtual keys"
    );

    // POST /api/channels -> 403 (write outside virtual-keys denied).
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/channels")
                .header("Authorization", "Bearer km-tok")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.112")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "key_manager must not be able to create channels"
    );

    // PUT /api/guardrails -> 403 (write outside virtual-keys denied).
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/guardrails")
                .header("Authorization", "Bearer km-tok")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.113")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "key_manager must not be able to update guardrails"
    );
}

// ---------------------------------------------------------------------------
// Test 16: RBAC — super admin token has full access
// ---------------------------------------------------------------------------

#[tokio::test]
async fn super_admin_token_has_full_access() {
    // Configure both a legacy admin_token (SuperAdmin) and a role-based
    // SuperAdmin token to verify neither is blocked by RBAC.
    let state = build_test_state_with_rbac(
        vec![],
        Some("legacy-admin"),
        vec![("sa-tok".to_string(), Role::SuperAdmin)],
    );
    let app = build_router(state, None);

    // GET /api/channels with legacy admin_token -> 200.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/channels")
                .header("Authorization", "Bearer legacy-admin")
                .header("X-Real-IP", "198.51.100.120")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // POST /api/virtual-keys with role-based super_admin token -> 200.
    let create_body = serde_json::json!({"name": "sa-created-key"}).to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/virtual-keys")
                .header("Authorization", "Bearer sa-tok")
                .header("Content-Type", "application/json")
                .header("X-Real-IP", "198.51.100.121")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "super_admin should be able to create virtual keys"
    );
    let json = body_json(response).await;
    let key_id = json["data"]["id"].as_str().unwrap_or("").to_string();

    // DELETE /api/virtual-keys/{id} with super_admin token -> 204.
    let delete_uri = format!("/api/virtual-keys/{key_id}");
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .header("Authorization", "Bearer sa-tok")
                .header("X-Real-IP", "198.51.100.122")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "super_admin should be able to delete virtual keys"
    );
}

// ---------------------------------------------------------------------------
// Test 17: RBAC — /api/auth/me returns the correct role
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_me_returns_correct_role() {
    let state = build_test_state_with_rbac(
        vec![],
        None,
        vec![
            ("auditor-tok".to_string(), Role::Auditor),
            ("km-tok".to_string(), Role::KeyManager),
        ],
    );
    let app = build_router(state, None);

    // Auditor token -> role "auditor", authenticated true.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("Authorization", "Bearer auditor-tok")
                .header("X-Real-IP", "198.51.100.130")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["data"]["role"], "auditor");
    assert_eq!(json["data"]["authenticated"], true);

    // Key manager token -> role "key_manager", authenticated true.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("Authorization", "Bearer km-tok")
                .header("X-Real-IP", "198.51.100.131")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["data"]["role"], "key_manager");
    assert_eq!(json["data"]["authenticated"], true);

    // No auth header -> 401 (auth is configured, so it is enforced).
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("X-Real-IP", "198.51.100.132")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "missing auth header should return 401 when auth is configured"
    );
}

// ---------------------------------------------------------------------------
// Test 18: RBAC — invalid role in config is ignored (token not authenticated)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_role_in_config_is_ignored() {
    // Simulate the effect of config loading filtering out an invalid role
    // string: the token is simply absent from admin_roles. A valid admin
    // token ensures auth is enforced, so the "filtered" token gets 401.
    let state = build_test_state_with_rbac(
        vec![],
        Some("real-admin"),
        // "invalid-role-tok" would have been filtered out by Role::from_str
        // returning None during config loading, so it is absent here.
        vec![],
    );
    let app = build_router(state, None);

    // The token that would have had an invalid role is rejected.
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("Authorization", "Bearer invalid-role-tok")
                .header("X-Real-IP", "198.51.100.140")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "token with invalid role string should not authenticate"
    );
}
