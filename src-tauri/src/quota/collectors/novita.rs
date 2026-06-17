use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo};

/// Novita AI: GET /openapi/v1/user/balance (unit: 0.0001 USD)
pub struct NovitaCollector;

#[async_trait::async_trait]
impl QuotaProvider for NovitaCollector {
    fn id(&self) -> &str {
        "novita"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case("novita")
            || ctx.base_url.to_lowercase().contains("novita")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        // Billing API has a fixed URL
        let url = "https://api.novita.ai/openapi/v1/user/balance".to_string();
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

        let raw = resp
            .text()
            .await
            .map_err(|e| QuotaError::Parse(e.to_string()))?;

        // Response may be a plain number (unit: 0.0001 USD)
        let balance = if let Ok(v) = raw.trim().parse::<f64>() {
            Some(v / 10_000.0)
        } else if let Ok(body) = serde_json::from_str::<serde_json::Value>(&raw) {
            // Or wrapped in JSON
            body.as_f64()
                .or_else(|| body.get("balance").and_then(|v| v.as_f64()))
                .map(|v| v / 10_000.0)
        } else {
            None
        };

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
    fn id_is_novita() {
        assert_eq!(NovitaCollector.id(), "novita");
    }

    #[test]
    fn supports_novita_by_name() {
        assert!(NovitaCollector.supports(&make_ctx("novita", "")));
        assert!(NovitaCollector.supports(&make_ctx("Novita", "")));
        assert!(NovitaCollector.supports(&make_ctx("NOVITA", "")));
    }

    #[test]
    fn supports_novita_by_url() {
        assert!(NovitaCollector.supports(&make_ctx("custom", "https://api.novita.ai")));
    }

    #[test]
    fn supports_rejects_unrelated() {
        assert!(!NovitaCollector.supports(&make_ctx("deepseek", "")));
        assert!(!NovitaCollector.supports(&make_ctx("", "https://api.openai.com")));
    }
}
