//! Security headers middleware.
//!
//! Adds standard security headers to all HTTP responses to harden
//! the gateway against common web attacks (clickjacking, MIME sniffing,
//! referrer leaks).

use axum::middleware::Next;
use axum::response::Response;

/// Add security headers to all responses.
///
/// Applied globally via `axum::middleware::from_fn`. The headers set are:
///
/// - `X-Content-Type-Options: nosniff` — prevents MIME-type sniffing.
/// - `X-Frame-Options: DENY` — blocks clickjacking by disallowing framing.
/// - `Referrer-Policy: strict-origin-when-cross-origin` — limits referrer leakage.
pub async fn security_headers_middleware(req: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        "X-Content-Type-Options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "X-Frame-Options",
        axum::http::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        "Referrer-Policy",
        axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn adds_security_headers() {
        let app = axum::Router::new()
            .route("/test", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/test")
                    .body(Body::default())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("X-Content-Type-Options")
                .unwrap()
                .to_str()
                .unwrap(),
            "nosniff"
        );
        assert_eq!(
            response
                .headers()
                .get("X-Frame-Options")
                .unwrap()
                .to_str()
                .unwrap(),
            "DENY"
        );
        assert_eq!(
            response
                .headers()
                .get("Referrer-Policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "strict-origin-when-cross-origin"
        );
    }
}
