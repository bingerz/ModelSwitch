use axum::body::Body;
use axum::http::{HeaderName, Request, Response};
use axum::middleware::Next;
use uuid::Uuid;

const REQUEST_ID_HEADER: &str = "x-request-id";

/// Extract or generate a request ID from the incoming request.
/// Propagates it as both a response header and an upstream request header.
pub async fn request_id_middleware(request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Set the request ID header for downstream handlers and upstream forwarding
    let mut request = request;
    let header_name = HeaderName::from_static(REQUEST_ID_HEADER);
    request
        .headers_mut()
        .insert(header_name.clone(), request_id.parse().unwrap());

    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header_name, request_id.parse().unwrap());
    response
}
