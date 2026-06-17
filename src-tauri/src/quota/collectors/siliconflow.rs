use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo};

/// SiliconFlow: GET /v1/user/info
pub struct SiliconFlowCollector;

#[async_trait::async_trait]
impl QuotaProvider for SiliconFlowCollector {
    fn id(&self) -> &str {
        "siliconflow"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case("siliconflow")
            || ctx.base_url.to_lowercase().contains("siliconflow")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        // Billing API has a fixed URL
        let url = "https://api.siliconflow.cn/v1/user/info".to_string();
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

        // Try totalBalance first, then balance
        let balance = body
            .get("totalBalance")
            .and_then(|v| v.as_f64())
            .or_else(|| body.get("balance").and_then(|v| v.as_f64()));

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance,
            source: "http_api".into(),
            ..QuotaInfo::new(ctx.channel_id, &ctx.channel_name, &ctx.provider, "http_api")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(provider: &str, base_url: &str) -> PollContext {
        PollContext {
            channel_id: uuid::Uuid::new_v4(),
            channel_name: "test".into(),
            provider: provider.into(),
            base_url: base_url.into(),
            credential: "key".into(),
            http_client: reqwest::Client::new(),
            quota_config: None,
        }
    }

    #[test]
    fn id_is_siliconflow() {
        assert_eq!(SiliconFlowCollector.id(), "siliconflow");
    }

    #[test]
    fn supports_siliconflow_by_name() {
        assert!(SiliconFlowCollector.supports(&make_ctx("siliconflow", "")));
        assert!(SiliconFlowCollector.supports(&make_ctx("SiliconFlow", "")));
        assert!(SiliconFlowCollector.supports(&make_ctx("SILICONFLOW", "")));
    }

    #[test]
    fn supports_siliconflow_by_url() {
        assert!(SiliconFlowCollector.supports(&make_ctx("custom", "https://api.siliconflow.cn")));
    }

    #[test]
    fn supports_rejects_unrelated() {
        assert!(!SiliconFlowCollector.supports(&make_ctx("deepseek", "")));
        assert!(!SiliconFlowCollector.supports(&make_ctx("", "https://api.openai.com")));
    }
}
