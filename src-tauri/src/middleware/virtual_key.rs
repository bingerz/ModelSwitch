//! Virtual key authentication middleware for proxy routes.
//!
//! Behavior:
//! - If no virtual keys are configured, the middleware is a pass-through
//!   (the gateway behaves as an open proxy keyed only on upstream channel creds).
//! - If at least one virtual key exists, every proxy request must carry a
//!   valid `Authorization: Bearer ms-vk-<key>` header that resolves to an
//!   enabled key within budget.
//! - On success, the matched key's id is injected as `x-virtual-key-id` so
//!   downstream handlers can attribute spend.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

use crate::proxy::AppState;

pub async fn virtual_key_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    // Open-proxy mode: no virtual keys configured.
    if !state.billing.virtual_key_store.has_keys().await {
        // Strip any client-provided virtual key header to prevent spoofing
        req.headers_mut().remove("x-virtual-key-id");
        return Ok(next.run(req).await);
    }

    // Extract bearer token from Authorization header.
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => return Err((StatusCode::UNAUTHORIZED, "Virtual key required")),
    };

    // Validate against virtual key store.
    match state.billing.virtual_key_store.validate(token).await {
        Some(vk) => {
            // Enforce IP allowlist if configured for this key.
            if !vk.allowed_ips.is_empty() {
                let client_ip =
                    super::auth::extract_client_ip(&req, state.security.trust_forwarded_headers);
                if !vk.check_ip_allowed(&client_ip) {
                    return Err((
                        StatusCode::FORBIDDEN,
                        "Client IP not allowed for this virtual key",
                    ));
                }
            }

            // Enforce per-key RPM rate limit.
            if let Some(rpm_limit) = vk.rpm_limit {
                if !state.billing.key_rate_limiter.check(vk.id, rpm_limit) {
                    return Err((
                        StatusCode::TOO_MANY_REQUESTS,
                        "Virtual key RPM limit exceeded",
                    ));
                }
            }

            // Pre-check TPM (block if already at/over limit — prevents runaway
            // usage). Actual token consumption is recorded post-response in the
            // dispatch path once the upstream returns real usage counts.
            if let Some(tpm_limit) = vk.tpm_limit {
                if !state.billing.key_rate_limiter.check_tpm(vk.id, tpm_limit) {
                    return Err((
                        StatusCode::TOO_MANY_REQUESTS,
                        "Virtual key TPM limit exceeded",
                    ));
                }
            }

            // Record this request against the key's RPM window.
            state.billing.key_rate_limiter.record(vk.id);

            // Inject virtual key ID for downstream spend tracking.
            // Handler extractors only see HeaderMap, not request extensions,
            // so we use a synthetic header to thread the id through.
            // Remove any client-provided value before setting our validated one.
            req.headers_mut().remove("x-virtual-key-id");
            req.headers_mut().insert(
                "x-virtual-key-id",
                vk.id.to_string().parse().unwrap_or_else(|_| {
                    // Uuid::to_string always parses back as a valid header value.
                    unreachable!("uuid string is always a valid header value")
                }),
            );
            Ok(next.run(req).await)
        }
        None => Err((StatusCode::UNAUTHORIZED, "Invalid or exhausted virtual key")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::build_test_state;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[test]
    fn extract_bearer_token_from_valid_header() {
        let header = "Bearer ms-vk-abc";
        let token = match header {
            h if h.starts_with("Bearer ") => &h[7..],
            _ => "",
        };
        assert_eq!(token, "ms-vk-abc");
    }

    #[test]
    fn extract_bearer_token_rejects_non_bearer() {
        let header = "Basic abc";
        let token = match header {
            h if h.starts_with("Bearer ") => &h[7..],
            _ => "",
        };
        assert_eq!(token, "");
    }

    /// A request from an IP that is not in the key's `allowed_ips` list
    /// must be rejected with 403 Forbidden.
    #[tokio::test]
    async fn rejects_request_from_non_allowed_ip() {
        let state = build_test_state(vec![]);

        // Create a virtual key restricted to a specific IP.
        let (_, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "ip-restricted".to_string(),
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

        let app = axum::Router::new()
            .route("/v1/test", axum::routing::any(|| async { "ok" }))
            .layer(from_fn_with_state(
                Arc::clone(&state),
                virtual_key_middleware,
            ));

        // Send a request from a different IP via x-forwarded-for.
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .header("x-forwarded-for", "192.168.1.99")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// A request from an allowed IP must succeed (positive control).
    #[tokio::test]
    async fn allows_request_from_allowed_ip() {
        let state = build_test_state(vec![]);

        let (_, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "ip-restricted".to_string(),
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

        let app = axum::Router::new()
            .route("/v1/test", axum::routing::any(|| async { "ok" }))
            .layer(from_fn_with_state(
                Arc::clone(&state),
                virtual_key_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .header("x-forwarded-for", "10.0.0.5")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// A key with no IP restrictions must allow any IP (backward compat).
    #[tokio::test]
    async fn allows_any_ip_when_no_restriction() {
        let state = build_test_state(vec![]);

        let (_, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "open".to_string(),
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

        let app = axum::Router::new()
            .route("/v1/test", axum::routing::any(|| async { "ok" }))
            .layer(from_fn_with_state(
                Arc::clone(&state),
                virtual_key_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .header("x-forwarded-for", "99.99.99.99")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// A key with `rpm_limit: Some(1)` must reject the second rapid request
    /// with 429 Too Many Requests.
    #[tokio::test]
    async fn rejects_request_when_rpm_exceeded() {
        let state = build_test_state(vec![]);

        let (_, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "rpm-limited".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                Some(1),
                None,
                None,
                None,
            )
            .await;

        let build_app = || {
            let state = Arc::clone(&state);
            axum::Router::new()
                .route("/v1/test", axum::routing::any(|| async { "ok" }))
                .layer(from_fn_with_state(state, virtual_key_middleware))
        };

        // First request should succeed.
        let response = build_app()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Second request within the same window should be rejected.
        let response = build_app()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// A key with `tpm_limit: Some(100)` must reject a request once the
    /// rolling TPM window has already reached or exceeded the limit.
    #[tokio::test]
    async fn rejects_request_when_tpm_exceeded() {
        let state = build_test_state(vec![]);

        let (vk, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "tpm-limited".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                Some(100),
                None,
                None,
            )
            .await;

        // Record enough token usage to exceed the TPM limit.
        state.billing.key_rate_limiter.record_tokens(vk.id, 101);

        let app = axum::Router::new()
            .route("/v1/test", axum::routing::any(|| async { "ok" }))
            .layer(from_fn_with_state(
                Arc::clone(&state),
                virtual_key_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// A key with `rpm_limit: Some(10)` must allow a request that stays
    /// within the limit.
    #[tokio::test]
    async fn allows_request_under_rpm_limit() {
        let state = build_test_state(vec![]);

        let (_, plaintext) = state
            .billing
            .virtual_key_store
            .create(
                "rpm-ok".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                Some(10),
                None,
                None,
                None,
            )
            .await;

        let app = axum::Router::new()
            .route("/v1/test", axum::routing::any(|| async { "ok" }))
            .layer(from_fn_with_state(
                Arc::clone(&state),
                virtual_key_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/test")
                    .header("Authorization", format!("Bearer {plaintext}"))
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
