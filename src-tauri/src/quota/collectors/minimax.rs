use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo, QuotaItem};

/// MiniMax: GET /v1/api/openplatform/coding_plan/remains
pub struct MiniMaxCollector;

#[async_trait::async_trait]
impl QuotaProvider for MiniMaxCollector {
    fn id(&self) -> &str {
        "minimax"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        let lower = ctx.provider.to_lowercase();
        lower == "minimax" || ctx.base_url.to_lowercase().contains("minimax")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        let url = "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains";
        let resp = ctx
            .http_client
            .get(url)
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

        // Check for API-level errors
        if let Some(status_code) = body
            .get("base_resp")
            .and_then(|r| r.get("status_code"))
            .and_then(|v| v.as_i64())
        {
            if status_code != 0 {
                let msg = body
                    .get("base_resp")
                    .and_then(|r| r.get("status_msg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return Err(QuotaError::Parse(format!(
                    "API error {}: {}",
                    status_code, msg
                )));
            }
        }

        let mut items = Vec::new();

        // Extract coding plan fields
        let remaining = body.get("remaining").and_then(|v| v.as_f64());
        let total = body.get("total").and_then(|v| v.as_f64());
        let used = body.get("used").and_then(|v| v.as_f64());
        let balance = body.get("balance").and_then(|v| v.as_f64());

        // Build detail items
        if let Some(v) = remaining {
            items.push(QuotaItem {
                label: "remaining".into(),
                value: format!("{:.0}", v),
            });
        }
        if let Some(v) = used {
            items.push(QuotaItem {
                label: "used".into(),
                value: format!("{:.0}", v),
            });
        }
        if let Some(v) = total {
            items.push(QuotaItem {
                label: "total".into(),
                value: format!("{:.0}", v),
            });
        }

        // balance is the usable dollar-equivalent amount
        // limit is total capacity, usage is consumed amount
        let quota_balance = balance.or_else(|| remaining);
        let quota_limit = total;
        let quota_usage = used;

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance: quota_balance,
            limit: quota_limit,
            usage: quota_usage,
            items,
            source: "http_api".into(),
            ..QuotaInfo::new(ctx.channel_id, &ctx.channel_name, &ctx.provider, "http_api")
        })
    }
}
