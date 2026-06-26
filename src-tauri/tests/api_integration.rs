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
use model_switch_lib::test_helpers::{build_test_state, build_test_state_with_admin_token};
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
