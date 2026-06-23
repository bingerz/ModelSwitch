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
}
