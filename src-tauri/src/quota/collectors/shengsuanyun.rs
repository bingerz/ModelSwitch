use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo};

/// 胜算云 (ShengSuanYun): GET /api/v1/key
pub struct ShengSuanYunCollector;

#[async_trait::async_trait]
impl QuotaProvider for ShengSuanYunCollector {
    fn id(&self) -> &str {
        "shengsuanyun"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case("shengsuanyun")
            || ctx.base_url.to_lowercase().contains("shengsuanyun")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        // Billing API: base_url is .../api, billing is .../api/v1/key
        let url = "https://router.shengsuanyun.com/api/v1/key".to_string();
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

        let max_quota = data
            .get("max_quota")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let consumed = data
            .get("consumed_amount")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Convert from internal units (divide by 500_000 for USD-like value)
        let unit_divisor = 500_000.0;
        let balance = Some((max_quota - consumed) / unit_divisor);
        let limit = Some(max_quota / unit_divisor);
        let usage = Some(consumed / unit_divisor);

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance,
            limit,
            usage,
            source: "http_api".into(),
            ..QuotaInfo::new(ctx.channel_id, &ctx.channel_name, &ctx.provider, "http_api")
        })
    }
}
