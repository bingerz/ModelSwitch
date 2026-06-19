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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // -- Test helpers ---------------------------------------------------------

    fn make_channel(provider: Provider, base_url: &str) -> Channel {
        Channel {
            id: Uuid::new_v4(),
            name: "probe-test".to_string(),
            provider,
            priority: 1,
            weight: 1,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential: Credential {
                cred_type: CredentialType::ApiKey,
                key_ref: "test".to_string(),
                api_key: Some("sk-test-key".to_string()),
                expires_at: None,
            },
            enabled: true,
            status: ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: base_url.to_string(),
            model_mapping: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            avg_latency_ms: 0,
            consecutive_failures: 0,
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
            api_keys: vec![],
        }
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    }

    // -- Healthy response tests -----------------------------------------------

    #[tokio::test]
    async fn probe_openai_200_returns_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-test")).await;
        assert!(result, "200 response should indicate healthy channel");
    }

    #[tokio::test]
    async fn probe_openai_429_returns_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-test")).await;
        assert!(
            result,
            "429 should indicate healthy (rate limited but reachable)"
        );
    }

    #[tokio::test]
    async fn probe_without_api_key_still_works() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, None).await;
        assert!(result, "probe without API key should still succeed on 200");
    }

    // -- Unhealthy response tests ---------------------------------------------

    #[tokio::test]
    async fn probe_openai_500_returns_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-test")).await;
        assert!(!result, "500 response should indicate unhealthy channel");
    }

    #[tokio::test]
    async fn probe_openai_401_returns_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-test")).await;
        assert!(!result, "401 should indicate unhealthy (invalid key)");
    }

    #[tokio::test]
    async fn probe_openai_403_returns_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-test")).await;
        assert!(!result, "403 should indicate unhealthy (forbidden)");
    }

    #[tokio::test]
    async fn probe_connection_error_returns_false() {
        // Port 1 is reserved and will refuse connections
        let channel = make_channel(Provider::OpenAI, "http://127.0.0.1:1");
        let result = probe_channel(&http_client(), &channel, Some("sk-test")).await;
        assert!(!result, "connection error should return false");
    }

    // -- Provider-specific endpoint tests -------------------------------------

    #[tokio::test]
    async fn probe_anthropic_uses_post_messages_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-anthropic"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Anthropic, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-anthropic")).await;
        assert!(result, "Anthropic probe should POST to /v1/messages");
    }

    #[tokio::test]
    async fn probe_anthropic_429_returns_true() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Anthropic, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("sk-anthropic")).await;
        assert!(result, "Anthropic 429 should be healthy (rate limited)");
    }

    #[tokio::test]
    async fn probe_gemini_passes_key_in_url_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(query_param("key", "gemini-secret"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Gemini, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("gemini-secret")).await;
        assert!(result, "Gemini probe should pass key as query parameter");
    }

    #[tokio::test]
    async fn probe_openai_sends_bearer_token_in_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer my-secret-key"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::OpenAI, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("my-secret-key")).await;
        assert!(result, "OpenAI probe should send Bearer token");
    }

    #[tokio::test]
    async fn probe_gemini_does_not_send_authorization_header() {
        let server = MockServer::start().await;
        // Mock without Authorization header matcher — if Gemini sends it,
        // the default mock (no header requirement) would still match.
        // We verify via received_requests instead.
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Gemini, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("gemini-key")).await;
        assert!(result);

        // Verify no Authorization header was sent
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].headers.contains_key("authorization"),
            "Gemini probe should not send Authorization header (key is in URL)"
        );
    }

    // -- DeepSeek / OpenRouter / Custom provider tests ------------------------

    #[tokio::test]
    async fn probe_deepseek_uses_get_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer ds-key"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::DeepSeek, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("ds-key")).await;
        assert!(
            result,
            "DeepSeek should use GET /v1/models with Bearer token"
        );
    }

    #[tokio::test]
    async fn probe_custom_provider_uses_get_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Custom("my-provider".to_string()), &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("key")).await;
        assert!(result, "Custom provider should use GET /v1/models");
    }

    // -- Ollama provider tests -------------------------------------------------

    #[tokio::test]
    async fn probe_ollama_uses_get_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Ollama, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("ollama")).await;
        assert!(result, "Ollama probe should use GET /v1/models");
    }

    #[tokio::test]
    async fn probe_ollama_401_returns_false() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let channel = make_channel(Provider::Ollama, &server.uri());
        let result = probe_channel(&http_client(), &channel, Some("ollama")).await;
        assert!(!result, "Ollama 401 should indicate unhealthy");
    }
}
