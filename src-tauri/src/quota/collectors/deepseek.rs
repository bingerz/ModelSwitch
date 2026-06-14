use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo, QuotaItem};

/// DeepSeek: GET /user/balance
pub struct DeepSeekCollector;

/// Parse the DeepSeek balance API response into QuotaInfo fields.
/// Extracted as a pure function for testability.
fn parse_balance_response(
    body: &serde_json::Value,
    channel_id: uuid::Uuid,
    channel_name: &str,
    provider: &str,
) -> Result<QuotaInfo, QuotaError> {
    let balance_infos = body
        .get("balance_infos")
        .and_then(|v| v.as_array())
        .ok_or_else(|| QuotaError::Parse("no balance_infos".into()))?;

    let balance: f64 = balance_infos
        .iter()
        .filter_map(|info| {
            info.get("total_balance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
        })
        .sum();

    let mut items = Vec::new();

    if let Some(available) = body.get("is_available").and_then(|v| v.as_bool()) {
        items.push(QuotaItem {
            label: "Account Status".into(),
            value: if available {
                "Available".into()
            } else {
                "Unavailable".into()
            },
        });
    }

    for info in balance_infos.iter() {
        let currency = info.get("currency").and_then(|v| v.as_str()).unwrap_or("?");
        let total = info
            .get("total_balance")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let granted = info
            .get("granted_balance")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        let topped = info
            .get("topped_up_balance")
            .and_then(|v| v.as_str())
            .unwrap_or("0");
        items.push(QuotaItem {
            label: format!("Balance ({})", currency),
            value: total.into(),
        });
        items.push(QuotaItem {
            label: format!("Granted ({})", currency),
            value: granted.into(),
        });
        items.push(QuotaItem {
            label: format!("Topped Up ({})", currency),
            value: topped.into(),
        });
    }

    Ok(QuotaInfo {
        channel_id,
        channel_name: channel_name.to_string(),
        provider: provider.to_string(),
        balance: Some(balance),
        source: "http_api".into(),
        items,
        ..QuotaInfo::new(channel_id, channel_name, provider, "http_api")
    })
}

#[async_trait::async_trait]
impl QuotaProvider for DeepSeekCollector {
    fn id(&self) -> &str {
        "deepseek"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        ctx.provider.eq_ignore_ascii_case("deepseek")
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        let url = "https://api.deepseek.com/user/balance".to_string();
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

        parse_balance_response(&body, ctx.channel_id, &ctx.channel_name, &ctx.provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_balance_with_positive_balance() {
        let body = serde_json::json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "110.00",
                "granted_balance": "10.00",
                "topped_up_balance": "100.00"
            }]
        });

        let id = uuid::Uuid::new_v4();
        let info = parse_balance_response(&body, id, "Test DeepSeek", "deepseek").unwrap();

        assert_eq!(info.balance, Some(110.0));
        assert_eq!(info.error, None);
        assert_eq!(info.items.len(), 4); // 1 account status + 3 currency items
        assert_eq!(info.items[0].label, "Account Status");
        assert_eq!(info.items[0].value, "Available");
        assert_eq!(info.items[1].label, "Balance (CNY)");
        assert_eq!(info.items[1].value, "110.00");
        assert_eq!(info.items[2].label, "Granted (CNY)");
        assert_eq!(info.items[2].value, "10.00");
        assert_eq!(info.items[3].label, "Topped Up (CNY)");
        assert_eq!(info.items[3].value, "100.00");
    }

    #[test]
    fn parse_balance_with_zero_balance() {
        let body = serde_json::json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "0.00",
                "granted_balance": "0.00",
                "topped_up_balance": "0.00"
            }]
        });

        let id = uuid::Uuid::new_v4();
        let info = parse_balance_response(&body, id, "Empty DeepSeek", "deepseek").unwrap();

        // Key assertion: balance 0.0 should return Some(0.0), not None
        assert_eq!(info.balance, Some(0.0));
        assert_eq!(info.items[0].value, "Available");
        assert_eq!(info.items[1].value, "0.00");
    }

    #[test]
    fn parse_balance_unavailable_account() {
        let body = serde_json::json!({
            "is_available": false,
            "balance_infos": [{
                "currency": "USD",
                "total_balance": "5.50",
                "granted_balance": "0.00",
                "topped_up_balance": "5.50"
            }]
        });

        let id = uuid::Uuid::new_v4();
        let info = parse_balance_response(&body, id, "Test", "deepseek").unwrap();

        assert_eq!(info.balance, Some(5.5));
        assert_eq!(info.items[0].value, "Unavailable");
    }

    #[test]
    fn parse_balance_missing_balance_infos() {
        let body = serde_json::json!({"is_available": true});
        let id = uuid::Uuid::new_v4();
        let result = parse_balance_response(&body, id, "Test", "deepseek");

        assert!(result.is_err());
    }
}
