use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo, QuotaItem};

/// Moonshot/Kimi: GET /v1/users/me/balance
pub struct MoonshotCollector;

#[async_trait::async_trait]
impl QuotaProvider for MoonshotCollector {
    fn id(&self) -> &str {
        "moonshot"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        let lower = ctx.provider.to_lowercase();
        lower == "moonshot"
            || lower == "kimi"
            || ctx.base_url.to_lowercase().contains("moonshot")
            || ctx.base_url.to_lowercase().contains("kimi")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        // Billing API has a fixed URL, not relative to proxy base_url
        let url = "https://api.moonshot.cn/v1/users/me/balance".to_string();
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

        let balance = data.get("available_balance").and_then(|v| v.as_f64());

        let mut items = Vec::new();
        if let Some(v) = data.get("voucher_balance").and_then(|v| v.as_f64()) {
            items.push(QuotaItem {
                label: "voucher".into(),
                value: format!("{:.2}", v),
            });
        }
        if let Some(v) = data.get("cash_balance").and_then(|v| v.as_f64()) {
            items.push(QuotaItem {
                label: "cash".into(),
                value: format!("{:.2}", v),
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
