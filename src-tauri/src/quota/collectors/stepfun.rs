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
