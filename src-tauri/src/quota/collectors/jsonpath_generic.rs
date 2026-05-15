use crate::quota::provider::{PollContext, QuotaProvider};
use crate::quota::{QuotaError, QuotaInfo};

/// Generic JSONPath collector: uses per-channel `QuotaConfig` to poll any billing endpoint.
///
/// Supports simple dot-path JSON extraction (e.g. `$.data.balance`, `data.totalBalance`).
/// No external dependencies required.
pub struct JsonPathCollector;

#[async_trait::async_trait]
impl QuotaProvider for JsonPathCollector {
    fn id(&self) -> &str {
        "jsonpath"
    }

    fn supports(&self, ctx: &PollContext) -> bool {
        // Only activate when the user has explicitly configured balance_url + balance_path
        if let Some(ref cfg) = ctx.quota_config {
            cfg.balance_url.is_some() && cfg.balance_path.is_some()
        } else {
            false
        }
    }

    async fn poll(&self, ctx: &PollContext) -> Result<QuotaInfo, QuotaError> {
        let cfg = ctx
            .quota_config
            .as_ref()
            .ok_or_else(|| QuotaError::Unsupported("no quota config".into()))?;

        let balance_url = cfg
            .balance_url
            .as_ref()
            .ok_or_else(|| QuotaError::Unsupported("no balance_url configured".into()))?;
        let balance_path = cfg
            .balance_path
            .as_ref()
            .ok_or_else(|| QuotaError::Unsupported("no balance_path configured".into()))?;

        // Build URL: if balance_url is relative, prepend base_url
        let url = if balance_url.starts_with("http://") || balance_url.starts_with("https://") {
            balance_url.clone()
        } else {
            let base = ctx.base_url.trim_end_matches('/');
            format!("{}{}", base, if balance_url.starts_with('/') { "" } else { "/" })
                + balance_url
        };

        // Build auth header
        let auth_prefix = cfg
            .auth_prefix
            .as_deref()
            .unwrap_or("Bearer");
        let auth_header = format!("{} {}", auth_prefix, ctx.credential);

        let resp = ctx
            .http_client
            .get(&url)
            .header("Authorization", auth_header)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| QuotaError::Network(e.to_string()))?;

        if resp.status().as_u16() == 401 {
            return Err(QuotaError::AuthFailed("invalid credentials".into()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| QuotaError::Parse(e.to_string()))?;

        // Extract balance using dot-path
        let balance = extract_json_path(&body, balance_path)?;

        // Optionally extract limit and usage
        let limit = cfg
            .limit_path
            .as_ref()
            .and_then(|p| extract_json_path(&body, p).ok())
            .flatten();
        let usage = cfg
            .usage_path
            .as_ref()
            .and_then(|p| extract_json_path(&body, p).ok())
            .flatten();

        Ok(QuotaInfo {
            channel_id: ctx.channel_id,
            channel_name: ctx.channel_name.clone(),
            provider: ctx.provider.clone(),
            balance,
            limit,
            usage,
            source: "jsonpath".into(),
            ..QuotaInfo::new(ctx.channel_id, &ctx.channel_name, &ctx.provider, "jsonpath")
        })
    }
}

/// Extract a numeric value from a JSON value using a simple dot-path notation.
///
/// Supports formats:
/// - `$.data.balance` (JSONPath-like, leading $. is stripped)
/// - `data.balance` (plain dot-path)
/// - `data.0.name` (array index access)
fn extract_json_path(value: &serde_json::Value, path: &str) -> Result<Option<f64>, QuotaError> {
    // Strip leading $. if present
    let path = path.strip_prefix("$.").unwrap_or(path);

    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }
        // Try numeric index first (for array access)
        if let Ok(idx) = segment.parse::<usize>() {
            current = current
                .get(idx)
                .ok_or_else(|| QuotaError::Parse(format!("path '{}' not found at index {}", path, idx)))?;
        } else {
            current = current
                .get(segment)
                .ok_or_else(|| {
                    QuotaError::Parse(format!("path '{}' not found at key '{}'", path, segment))
                })?;
        }
    }

    // Extract as f64, handling both numeric and string values
    let num = current
        .as_f64()
        .or_else(|| current.as_str().and_then(|s| s.parse::<f64>().ok()));

    Ok(if num.is_some() { num } else { None })
}
