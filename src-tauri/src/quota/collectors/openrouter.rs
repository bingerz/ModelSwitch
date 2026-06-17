use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo, QuotaItem};

/// OpenRouter: GET /api/v1/key
pub struct OpenRouterCollector;

#[async_trait::async_trait]
impl QuotaProvider for OpenRouterCollector {
    fn id(&self) -> &str {
        "openrouter"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case("openrouter")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        // Billing API has a fixed URL
        let url = "https://openrouter.ai/api/v1/key".to_string();
        let resp = ctx
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", ctx.credential))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| QuotaError::Network(e.to_string()))?;

        if resp.status().as_u16() == 401 {
            return Err(QuotaError::AuthFailed("invalid API key".into()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| QuotaError::Parse(e.to_string()))?;

        let data = body
            .get("data")
            .ok_or_else(|| QuotaError::Parse("no data field".into()))?;

        let usage = data.get("usage").and_then(|v| v.as_f64());
        let limit = data
            .get("limit")
            .and_then(|v| v.as_f64())
            .filter(|&v| v > 0.0);
        let balance = limit.and_then(|l| usage.map(|u| l - u));

        let mut items = Vec::new();
        if let Some(d) = data.get("usage_daily").and_then(|v| v.as_f64()) {
            items.push(QuotaItem {
                label: "daily".into(),
                value: format!("{:.2}", d),
            });
        }
        if let Some(w) = data.get("usage_weekly").and_then(|v| v.as_f64()) {
            items.push(QuotaItem {
                label: "weekly".into(),
                value: format!("{:.2}", w),
            });
        }
        if let Some(m) = data.get("usage_monthly").and_then(|v| v.as_f64()) {
            items.push(QuotaItem {
                label: "monthly".into(),
                value: format!("{:.2}", m),
            });
        }

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance,
            limit,
            usage,
            items,
            source: "http_api".into(),
            ..QuotaInfo::new(ctx.channel_id, &ctx.channel_name, &ctx.provider, "http_api")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(provider: &str) -> PollContext {
        PollContext {
            channel_id: uuid::Uuid::new_v4(),
            channel_name: "test".into(),
            provider: provider.into(),
            base_url: "https://openrouter.ai".into(),
            credential: "key".into(),
            http_client: reqwest::Client::new(),
            quota_config: None,
        }
    }

    #[test]
    fn id_is_openrouter() {
        assert_eq!(OpenRouterCollector.id(), "openrouter");
    }

    #[test]
    fn supports_openrouter_case_insensitive() {
        assert!(OpenRouterCollector.supports(&make_ctx("openrouter")));
        assert!(OpenRouterCollector.supports(&make_ctx("OpenRouter")));
        assert!(OpenRouterCollector.supports(&make_ctx("OPENROUTER")));
    }

    #[test]
    fn supports_rejects_non_openrouter() {
        assert!(!OpenRouterCollector.supports(&make_ctx("deepseek")));
        assert!(!OpenRouterCollector.supports(&make_ctx("zhipu")));
        assert!(!OpenRouterCollector.supports(&make_ctx("")));
    }
}
