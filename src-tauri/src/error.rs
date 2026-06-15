use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Unified error type for the ModelSwitch gateway.
#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("channel not found: {0}")]
    ChannelNotFound(String),

    #[error("no healthy channel available for model '{0}'")]
    NoHealthyChannel(String),

    #[error("all channels exhausted for model '{0}'")]
    AllChannelsExhausted(String),

    #[error("credential error: {0}")]
    Credential(String),

    #[error("rate limit exceeded")]
    RateLimited,

    #[error("payload rejected: {0}")]
    PayloadRejected(String),

    #[error("upstream error: {status} {body}")]
    Upstream { status: u16, body: String },

    #[error("connection error: {0}")]
    Connection(String),

    #[error("timeout after {0}s")]
    Timeout(u64),

    #[error("quota error: {0}")]
    Quota(#[from] crate::quota::QuotaError),

    #[error("virtual key error: {0}")]
    VirtualKey(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("mcp error: {0}")]
    Mcp(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

/// Convenience type alias.
pub type GatewayResult<T> = Result<T, GatewayError>;

#[derive(Serialize)]
struct GatewayErrorBody {
    error: GatewayErrorDetail,
}

#[derive(Serialize)]
struct GatewayErrorDetail {
    message: String,
    code: String,
}

impl GatewayError {
    /// Returns a client-safe message that omits internal details.
    ///
    /// Only client-actionable variants (channel not found, rate limited, payload
    /// rejected, etc.) include the full error text. Internal variants (upstream
    /// body, config, io, internal) return a generic description to prevent
    /// information disclosure.
    fn safe_message(&self) -> String {
        match self {
            // Client-actionable — safe to expose details
            GatewayError::ChannelNotFound(_)
            | GatewayError::NoHealthyChannel(_)
            | GatewayError::AllChannelsExhausted(_)
            | GatewayError::RateLimited
            | GatewayError::PayloadRejected(_)
            | GatewayError::Timeout(_) => self.to_string(),

            // Credential/key errors — omit the detail string
            GatewayError::Credential(_) => "authentication or credential error".into(),
            GatewayError::VirtualKey(_) => "virtual key error".into(),

            // Upstream — expose status but not the body
            GatewayError::Upstream { status, .. } => {
                format!("upstream returned status {status}")
            }

            // Internal system errors — generic message only
            GatewayError::Connection(_) => "upstream connection error".into(),
            GatewayError::Config(_) => "configuration error".into(),
            GatewayError::Mcp(_) => "MCP server error".into(),
            GatewayError::Io(_) => "internal I/O error".into(),
            GatewayError::Internal(_) => "internal server error".into(),
            GatewayError::Quota(_) => "quota or billing limit exceeded".into(),
        }
    }
}

/// Maps each `GatewayError` variant to an HTTP status code and error code string.
impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            GatewayError::ChannelNotFound(_) => (StatusCode::NOT_FOUND, "channel_not_found"),
            GatewayError::NoHealthyChannel(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "no_healthy_channel")
            }
            GatewayError::AllChannelsExhausted(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "all_channels_exhausted")
            }
            GatewayError::Credential(_) => (StatusCode::UNAUTHORIZED, "credential_error"),
            GatewayError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_exceeded"),
            GatewayError::PayloadRejected(_) => (StatusCode::BAD_REQUEST, "payload_rejected"),
            GatewayError::Upstream { status, .. } => {
                let code = if *status >= 500 {
                    "upstream_server_error"
                } else {
                    "upstream_client_error"
                };
                return (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(GatewayErrorBody {
                        error: GatewayErrorDetail {
                            message: self.safe_message(),
                            code: code.to_string(),
                        },
                    }),
                )
                    .into_response();
            }
            GatewayError::Connection(_) => (StatusCode::BAD_GATEWAY, "upstream_connection_error"),
            GatewayError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, "upstream_timeout"),
            GatewayError::Quota(_) => (StatusCode::FORBIDDEN, "quota_exceeded"),
            GatewayError::VirtualKey(_) => (StatusCode::FORBIDDEN, "virtual_key_error"),
            GatewayError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "config_error"),
            GatewayError::Mcp(_) => (StatusCode::INTERNAL_SERVER_ERROR, "mcp_error"),
            GatewayError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io_error"),
            GatewayError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };

        (
            status,
            axum::Json(GatewayErrorBody {
                error: GatewayErrorDetail {
                    message: self.safe_message(),
                    code: code.to_string(),
                },
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for GatewayError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

impl From<serde_json::Error> for GatewayError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(format!("json: {e}"))
    }
}
