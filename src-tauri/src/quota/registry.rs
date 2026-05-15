use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo};

/// Registry that holds all available QuotaProvider implementations and
/// resolves the best strategy for a given channel.
pub struct QuotaProviderRegistry {
    providers: Vec<Box<dyn QuotaProvider>>,
}

impl QuotaProviderRegistry {
    pub fn new(providers: Vec<Box<dyn QuotaProvider>>) -> Self {
        Self { providers }
    }

    /// Resolve the best QuotaProvider for the given context.
    /// Priority:
    /// 0. User explicit strategy override (from quota config)
    /// 0.5. Auto-detect JSONPath collector when balance_url + balance_path are configured
    /// 1. Provider name exact match
    /// 2. Base URL pattern match
    /// 3. Fallback to openai_compat
    pub fn resolve(&self, ctx: &PollContext) -> Option<&dyn QuotaProvider> {
        // 0. User explicit strategy override
        if let Some(ref cfg) = ctx.quota_config {
            if let Some(ref strategy) = cfg.strategy {
                if strategy == "disabled" {
                    return None;
                }
                if let Some(p) = self.find_by_id(strategy) {
                    return Some(p);
                }
            }
        }
        // 0.5. Auto-detect JSONPath when quota config has balance_url + balance_path
        if let Some(p) = self.find_by_id("jsonpath") {
            if p.supports(ctx) {
                return Some(p);
            }
        }
        // 1. Exact match by provider name
        if let Some(p) = self.find_by_id(&ctx.provider) {
            return Some(p);
        }
        // 2. URL pattern match
        if let Some(p) = self.match_by_url(&ctx.base_url) {
            return Some(p);
        }
        // 3. Fallback: openai_compat (handles NewAPI/OneAPI stations)
        self.find_by_id("openai_compat")
    }

    /// Find a provider by its exact id.
    pub fn find_by_id(&self, id: &str) -> Option<&dyn QuotaProvider> {
        self.providers
            .iter()
            .find(|p| p.id().eq_ignore_ascii_case(id))
            .map(|p| p.as_ref())
    }

    /// Match a provider by base_url patterns for known hosting platforms.
    fn match_by_url(&self, url: &str) -> Option<&dyn QuotaProvider> {
        let lower = url.to_lowercase();
        match () {
            _ if lower.contains("siliconflow") => self.find_by_id("siliconflow"),
            _ if lower.contains("moonshot") || lower.contains("kimi") => {
                self.find_by_id("moonshot")
            }
            _ if lower.contains("stepfun") => self.find_by_id("stepfun"),
            _ if lower.contains("novita") => self.find_by_id("novita"),
            _ if lower.contains("shengsuanyun") => self.find_by_id("shengsuanyun"),
            _ if lower.contains("bigmodel") => self.find_by_id("zhipu"),
            _ if lower.contains("minimax") => self.find_by_id("minimax"),
            _ if lower.contains("deepseek") => self.find_by_id("deepseek"),
            _ => None,
        }
    }

    /// Poll using the best-matched provider, returning QuotaInfo or an error.
    pub async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        let provider = self.resolve(ctx).ok_or_else(|| {
            QuotaError::Unsupported(format!(
                "no quota provider for '{}'",
                ctx.provider
            ))
        })?;
        provider.poll(ctx).await
    }
}
