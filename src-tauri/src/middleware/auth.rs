use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

use crate::proxy::openai::AppState;

/// Constant-time Bearer token verification to prevent timing attacks.
fn verify_bearer_token(header_value: &str, expected: &str) -> bool {
    if !header_value.starts_with("Bearer ") {
        return false;
    }
    use subtle::ConstantTimeEq;
    header_value[7..]
        .as_bytes()
        .ct_eq(expected.as_bytes())
        .into()
}

/// Bearer token auth middleware for /api/* routes.
/// If admin_token is configured, validates Authorization header.
/// If not configured, passes through (backward compatible).
pub async fn admin_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, &'static str)> {
    let token = match &state.admin_token {
        Some(t) => t,
        None => return Ok(next.run(req).await),
    };

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(h) if verify_bearer_token(h, token) => Ok(next.run(req).await),
        _ => Err((StatusCode::UNAUTHORIZED, "Unauthorized")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_correct_token() {
        assert!(verify_bearer_token("Bearer my-secret", "my-secret"));
    }

    #[test]
    fn rejects_wrong_token() {
        assert!(!verify_bearer_token("Bearer wrong", "my-secret"));
    }

    #[test]
    fn rejects_missing_prefix() {
        assert!(!verify_bearer_token("my-secret", "my-secret"));
    }

    #[test]
    fn rejects_empty_header() {
        assert!(!verify_bearer_token("", "my-secret"));
    }
}
