use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Serialize)]
pub struct ApiError {
    pub error: ApiErrorDetail,
}

#[derive(Serialize)]
pub struct ApiErrorDetail {
    pub message: String,
    pub code: String,
}

impl ApiError {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Response {
        let code = match status {
            StatusCode::UNAUTHORIZED => "invalid_api_key",
            StatusCode::FORBIDDEN => "permission_denied",
            StatusCode::NOT_FOUND => "not_found",
            StatusCode::BAD_REQUEST => "bad_request",
            StatusCode::TOO_MANY_REQUESTS => "rate_limit_exceeded",
            StatusCode::CONFLICT => "conflict",
            StatusCode::INTERNAL_SERVER_ERROR => "internal_error",
            StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
            _ => "unknown_error",
        };
        (
            status,
            axum::Json(ApiError {
                error: ApiErrorDetail {
                    message: message.into(),
                    code: code.to_string(),
                },
            }),
        )
            .into_response()
    }
}
