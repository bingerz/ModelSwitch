use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo};
use chrono::Datelike;

/// Generic NewAPI/OneAPI compatible collector.
/// Tries /dashboard/billing/subscription and /dashboard/billing/usage endpoints.
/// Covers: AiHubMix, DMXAPI, Compshare, ModelScope, and all NewAPI/OneAPI stations.
pub struct OpenAiCompatCollector;

#[async_trait::async_trait]
impl QuotaProvider for OpenAiCompatCollector {
    fn id(&self) -> &str {
        "openai_compat"
    }

    fn supports(&self, _ctx: &PollContext) -> bool {
        // This is the fallback — always returns true for supports()
        true
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        let base = ctx.base_url.trim_end_matches('/');
        let auth = format!("Bearer {}", ctx.credential);
        let timeout = std::time::Duration::from_secs(15);

        // Try multiple path variants for subscription
        let subscription_paths = [
            format!("{}/dashboard/billing/subscription", base),
            format!("{}/v1/dashboard/billing/subscription", base),
        ];

        let mut hard_limit_usd: Option<f64> = None;
        let mut access_until: Option<String> = None;

        for url in &subscription_paths {
            if let Ok(resp) = ctx
                .http_client
                .get(url)
                .header("Authorization", &auth)
                .timeout(timeout)
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        hard_limit_usd = body.get("hard_limit_usd").and_then(|v| v.as_f64());
                        if let Some(until) = body.get("access_until").and_then(|v| v.as_i64()) {
                            access_until = Some(
                                chrono::DateTime::from_timestamp(until, 0)
                                    .map(|dt| dt.to_rfc3339())
                                    .unwrap_or_else(|| until.to_string()),
                            );
                        }
                        break;
                    }
                }
            }
        }

        // If subscription endpoint didn't work, this isn't a NewAPI station
        let hard_limit = match hard_limit_usd {
            Some(l) => l,
            None => {
                return Err(QuotaError::Unsupported(
                    "no /dashboard/billing/subscription endpoint".into(),
                ));
            }
        };

        // Fetch usage for current month
        let now = chrono::Utc::now();
        let start_date = format!("{}-{:02}-01", now.format("%Y"), now.month());
        let end_date = now.format("%Y-%m-%d").to_string();

        let usage_paths = [
            format!(
                "{}/dashboard/billing/usage?start_date={}&end_date={}",
                base, start_date, end_date
            ),
            format!(
                "{}/v1/dashboard/billing/usage?start_date={}&end_date={}",
                base, start_date, end_date
            ),
        ];

        let mut total_usage_usd: f64 = 0.0;

        for url in &usage_paths {
            if let Ok(resp) = ctx
                .http_client
                .get(url)
                .header("Authorization", &auth)
                .timeout(timeout)
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        // total_usage is in cents
                        total_usage_usd = body
                            .get("total_usage")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                            / 100.0;
                        break;
                    }
                }
            }
        }

        let balance = Some(hard_limit - total_usage_usd);

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance,
            limit: Some(hard_limit),
            usage: Some(total_usage_usd),
            expires_at: access_until,
            source: "http_api".into(),
            ..QuotaInfo::new(ctx.channel_id, &ctx.channel_name, &ctx.provider, "http_api")
        })
    }
}
