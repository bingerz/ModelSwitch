use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo, QuotaItem};

/// StepFun: GET /v1/accounts
pub struct StepFunCollector;

#[async_trait::async_trait]
impl QuotaProvider for StepFunCollector {
    fn id(&self) -> &str {
        "stepfun"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case("stepfun")
            || ctx.base_url.to_lowercase().contains("stepfun")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        // Billing API has a fixed URL, not relative to proxy base_url
        let url = "https://api.stepfun.com/v1/accounts".to_string();
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

        let cash = body
            .get("total_cash_balance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let voucher = body
            .get("total_voucher_balance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let balance = Some(cash + voucher);

        let mut items = Vec::new();
        if cash > 0.0 {
            items.push(QuotaItem {
                label: "cash".into(),
                value: format!("{:.2}", cash),
            });
        }
        if voucher > 0.0 {
            items.push(QuotaItem {
                label: "voucher".into(),
                value: format!("{:.2}", voucher),
            });
        }

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance,
            items,
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
    fn id_is_stepfun() {
        assert_eq!(StepFunCollector.id(), "stepfun");
    }

    #[test]
    fn supports_stepfun_by_name() {
        assert!(StepFunCollector.supports(&make_ctx("stepfun", "")));
        assert!(StepFunCollector.supports(&make_ctx("StepFun", "")));
        assert!(StepFunCollector.supports(&make_ctx("STEPFUN", "")));
    }

    #[test]
    fn supports_stepfun_by_url() {
        assert!(StepFunCollector.supports(&make_ctx("custom", "https://api.stepfun.com")));
    }

    #[test]
    fn supports_rejects_unrelated() {
        assert!(!StepFunCollector.supports(&make_ctx("deepseek", "")));
        assert!(!StepFunCollector.supports(&make_ctx("", "https://api.openai.com")));
    }
}
