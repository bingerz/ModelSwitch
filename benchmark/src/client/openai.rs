use anyhow::Result;
use reqwest::{Client, StatusCode};
use serde_json::json;
use std::time::{Duration, Instant};

use crate::client::stream;
use crate::client::Protocol;
use crate::metrics::summary::ErrorCategory;

/// HTTP client for benchmark requests. Supports OpenAI and Anthropic protocols.
pub struct OpenAIClient {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    timeout_secs: u64,
    protocol: Protocol,
}

/// Timing data from a single non-streaming request.
pub struct RequestTiming {
    pub status: StatusCode,
    pub success: bool,
    pub latency: Duration,
    pub conn_latency: Duration,
    pub error_category: ErrorCategory,
}

impl OpenAIClient {
    pub fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        timeout_secs: u64,
        pool_max_idle: usize,
        tls_skip_verify: bool,
        protocol: Protocol,
    ) -> Result<Self> {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .pool_max_idle_per_host(pool_max_idle)
            .user_agent("gateway-bench/0.1");

        if tls_skip_verify {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let client = builder.build()?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            timeout_secs,
            protocol,
        })
    }

    /// Expose params for streaming.
    fn endpoint(&self) -> String {
        match self.protocol {
            Protocol::OpenAI => format!("{}/v1/chat/completions", self.base_url),
            Protocol::Anthropic => format!("{}/v1/messages", self.base_url),
            Protocol::Gemini => format!(
                "{}/v1beta/models/{}:generateContent",
                self.base_url, self.model
            ),
        }
    }

    /// Send a non-streaming request.
    pub async fn chat(&self, message_content: &str, max_tokens: usize) -> Result<RequestTiming> {
        let url = self.endpoint();
        let start = Instant::now();

        let body = match self.protocol {
            Protocol::OpenAI => json!({
                "model": &self.model,
                "messages": [{"role": "user", "content": message_content}],
                "max_tokens": max_tokens,
                "stream": false,
            }),
            Protocol::Anthropic => json!({
                "model": &self.model,
                "messages": [{"role": "user", "content": message_content}],
                "max_tokens": max_tokens,
            }),
            Protocol::Gemini => json!({
                "contents": [{"role": "user", "parts": [{"text": message_content}]}],
                "generationConfig": {"maxOutputTokens": max_tokens},
            }),
        };

        let req = match self.protocol {
            Protocol::OpenAI => self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", &self.api_key)),
            Protocol::Anthropic => self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01"),
            Protocol::Gemini => self
                .client
                .post(&url)
                .header("x-goog-api-key", &self.api_key),
        };

        let resp_result = req
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        let latency = start.elapsed();

        match resp_result {
            Ok(resp) => {
                let status = resp.status();
                let success = status.is_success();
                let error_category = if success {
                    ErrorCategory::None
                } else if status == StatusCode::TOO_MANY_REQUESTS {
                    ErrorCategory::RateLimited429
                } else if status.is_client_error() {
                    ErrorCategory::Client4xx
                } else {
                    ErrorCategory::Server5xx
                };

                Ok(RequestTiming {
                    status,
                    success,
                    latency,
                    conn_latency: Duration::ZERO,
                    error_category,
                })
            }
            Err(e) => {
                let error_category = if e.is_timeout() {
                    ErrorCategory::Timeout
                } else {
                    ErrorCategory::Network
                };
                Ok(RequestTiming {
                    status: StatusCode::default(),
                    success: false,
                    latency,
                    conn_latency: Duration::ZERO,
                    error_category,
                })
            }
        }
    }

    /// Send a streaming request and measure SSE timing.
    pub async fn chat_stream(
        &self,
        message_content: &str,
        max_tokens: usize,
    ) -> Result<stream::StreamTiming> {
        stream::stream_chat(
            &self.client,
            &self.base_url,
            &self.api_key,
            &self.model,
            message_content,
            max_tokens,
            self.timeout_secs,
            self.protocol,
        )
        .await
    }
}
