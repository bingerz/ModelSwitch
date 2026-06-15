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
