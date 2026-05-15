use crate::config::QuotaConfig;
use crate::quota::{QuotaError, QuotaInfo};

/// Context passed to each QuotaProvider for polling.
pub struct PollContext {
    pub channel_id: uuid::Uuid,
    pub channel_name: String,
    pub provider: String,
    pub base_url: String,
    pub credential: String,
    pub http_client: reqwest::Client,
    /// Per-channel quota config from TOML (if set).
    pub quota_config: Option<QuotaConfig>,
}

/// A strategy for fetching quota/billing info from a specific provider.
#[async_trait::async_trait]
pub trait QuotaProvider: Send + Sync {
    /// Unique identifier for this strategy (e.g. "openrouter", "openai_compat").
    fn id(&self) -> &str;

    /// Poll the provider's billing API and return normalized QuotaInfo.
    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError>;

    /// Check whether this provider can handle the given context.
    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case(self.id())
    }
}
