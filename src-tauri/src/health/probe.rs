use crate::channel::{Channel, Provider};

/// Send a lightweight probe request to check if a channel is healthy.
/// Uses provider-appropriate endpoints with real auth:
/// - OpenAI/DeepSeek/OpenRouter/Custom: GET /v1/models with Bearer token
/// - Anthropic: lightweight POST to /v1/messages with x-api-key
/// - Gemini: GET with key in URL
///
/// 200/429 = healthy (endpoint reachable, key valid / rate-limited)
/// 401/403 = unhealthy (key invalid or unauthorized)
/// Connection error = unhealthy
pub async fn probe_channel(
    http_client: &reqwest::Client,
    channel: &Channel,
    api_key: Option<&str>,
) -> bool {
    let url = match channel.provider {
        Provider::Anthropic => {
            let base = channel.base_url.trim_end_matches('/');
            format!("{}/v1/messages", base)
        }
        Provider::Gemini => {
            let base = channel.base_url.trim_end_matches('/');
            let key = api_key.unwrap_or("");
            format!("{}/v1/models?key={}", base, key)
        }
        _ => {
            let base = channel.base_url.trim_end_matches('/');
            format!("{}/v1/models", base)
        }
    };

    let result = match channel.provider {
        Provider::Anthropic => {
            let mut req = http_client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .json(&serde_json::json!({
                    "model": "claude-3-5-haiku-20241022",
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "hi"}]
                }))
                .timeout(std::time::Duration::from_secs(10));
            if let Some(key) = api_key {
                req = req.header("x-api-key", key);
            }
            req.send().await
        }
        _ => {
            let mut req = http_client
                .get(&url)
                .timeout(std::time::Duration::from_secs(10));
            if let Some(key) = api_key {
                if !matches!(channel.provider, Provider::Gemini) {
                    req = req.header("Authorization", format!("Bearer {}", key));
                }
            }
            req.send().await
        }
    };

    match result {
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16();
            // 200 = healthy, 429 = rate limited but key is valid
            // 401/403 = key invalid — treat as unhealthy
            status.is_success() || code == 429
        }
        Err(_) => false,
    }
}
