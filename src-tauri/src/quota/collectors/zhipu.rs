use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo, QuotaItem};

/// Zhipu GLM (BigModel): GET /api/paas/api/biz/tokenAccounts/list
pub struct ZhipuCollector;

#[async_trait::async_trait]
impl QuotaProvider for ZhipuCollector {
    fn id(&self) -> &str {
        "zhipu"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        let lower = ctx.provider.to_lowercase();
        lower == "zhipu"
            || lower == "glm"
            || lower == "bigmodel"
            || ctx.base_url.to_lowercase().contains("bigmodel")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        let url = "https://open.bigmodel.cn/api/paas/api/biz/tokenAccounts/list?filterEnabled=true";
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

        // Response shape: { "data": [ { "id", "name", "totalBalance", "usedBalance", "balance", "status" } ] }
        let accounts = body
            .get("data")
            .and_then(|v| v.as_array())
            .ok_or_else(|| QuotaError::Parse("no data array in response".into()))?;

        let mut total_balance = 0.0_f64;
        let mut total_limit = 0.0_f64;
        let mut total_usage = 0.0_f64;
        let mut items = Vec::new();

        for account in accounts {
            // Sum across all enabled token accounts
            let balance = account
                .get("balance")
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                })
                .unwrap_or(0.0);
            let limit = account
                .get("totalBalance")
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                })
                .unwrap_or(0.0);
            let used = account
                .get("usedBalance")
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                })
                .unwrap_or(0.0);

            total_balance += balance;
            total_limit += limit;
            total_usage += used;

            if let Some(name) = account.get("name").and_then(|v| v.as_str()) {
                items.push(QuotaItem {
                    label: name.to_string(),
                    value: format!("{:.2} / {:.2}", balance, limit),
                });
            }
        }

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance: if total_balance > 0.0 {
                Some(total_balance)
            } else {
                None
            },
            limit: if total_limit > 0.0 {
                Some(total_limit)
            } else {
                None
            },
            usage: if total_usage > 0.0 {
                Some(total_usage)
            } else {
                None
            },
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
    fn id_is_zhipu() {
        assert_eq!(ZhipuCollector.id(), "zhipu");
    }

    #[test]
    fn supports_zhipu_name_variants() {
        assert!(ZhipuCollector.supports(&make_ctx("zhipu", "")));
        assert!(ZhipuCollector.supports(&make_ctx("Zhipu", "")));
        assert!(ZhipuCollector.supports(&make_ctx("glm", "")));
        assert!(ZhipuCollector.supports(&make_ctx("GLM", "")));
        assert!(ZhipuCollector.supports(&make_ctx("bigmodel", "")));
    }

    #[test]
    fn supports_bigmodel_url() {
        assert!(ZhipuCollector.supports(&make_ctx("custom", "https://open.bigmodel.cn")));
        assert!(ZhipuCollector.supports(&make_ctx("custom", "https://BIGMODEL.com")));
    }

    #[test]
    fn supports_rejects_unrelated() {
        assert!(!ZhipuCollector.supports(&make_ctx("deepseek", "https://api.deepseek.com")));
        assert!(!ZhipuCollector.supports(&make_ctx("openai", "https://api.openai.com")));
    }
}
