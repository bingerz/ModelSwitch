use anyhow::Result;
use reqwest::{Client, StatusCode};
use serde_json::json;
use std::time::{Duration, Instant};

use crate::client::Protocol;
use crate::metrics::summary::ErrorCategory;

/// Result of a streaming request, including TTFB and chunk timing.
pub struct StreamTiming {
    pub status: StatusCode,
    pub success: bool,
    pub latency: Duration,
    pub ttfb: Option<Duration>,
    pub chunk_intervals: Vec<Duration>,
    pub total_chunks: usize,
    pub error_category: ErrorCategory,
}

/// Send a streaming request and measure SSE timing.
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat(
    client: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    message_content: &str,
    max_tokens: usize,
    timeout_secs: u64,
    protocol: Protocol,
) -> Result<StreamTiming> {
    let (url, body) = match protocol {
        Protocol::OpenAI => (
            format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            json!({
                "model": model,
                "messages": [{"role": "user", "content": message_content}],
                "max_tokens": max_tokens,
                "stream": true,
            }),
        ),
        Protocol::Anthropic => (
            format!("{}/v1/messages", base_url.trim_end_matches('/')),
            json!({
                "model": model,
                "messages": [{"role": "user", "content": message_content}],
                "max_tokens": max_tokens,
                "stream": true,
            }),
        ),
        Protocol::Gemini => (
            format!(
                "{}/v1beta/models/{}:streamGenerateContent",
                base_url.trim_end_matches('/'),
                model
            ),
            json!({
                "contents": [{"role": "user", "parts": [{"text": message_content}]}],
                "generationConfig": {"maxOutputTokens": max_tokens},
            }),
        ),
    };

    let req = match protocol {
        Protocol::OpenAI => client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key)),
        Protocol::Anthropic => client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        Protocol::Gemini => client.post(&url).header("x-goog-api-key", api_key),
    };

    let start = Instant::now();

    let resp_result = req
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(timeout_secs))
        .send()
        .await;

    match resp_result {
        Ok(resp) => {
            let status = resp.status();
            let success = status.is_success();

            if !success {
                let latency = start.elapsed();
                let error_category = if status == StatusCode::TOO_MANY_REQUESTS {
                    ErrorCategory::RateLimited429
                } else if status.is_client_error() {
                    ErrorCategory::Client4xx
                } else {
                    ErrorCategory::Server5xx
                };
                return Ok(StreamTiming {
                    status,
                    success,
                    latency,
                    ttfb: None,
                    chunk_intervals: vec![],
                    total_chunks: 0,
                    error_category,
                });
            }

            // Read SSE stream — track errors from truncated/broken streams
            let mut ttfb: Option<Duration> = None;
            let mut chunk_intervals: Vec<Duration> = Vec::new();
            let mut last_chunk_time: Option<Instant> = None;
            let mut total_chunks = 0usize;
            let mut stream_error = false;
            let mut final_error_category = ErrorCategory::None;

            use futures::StreamExt;
            let mut stream = resp.bytes_stream();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        let has_data = text.contains("data:");

                        if has_data {
                            let now = Instant::now();
                            if ttfb.is_none() {
                                ttfb = Some(now.duration_since(start));
                            }
                            if let Some(last) = last_chunk_time {
                                chunk_intervals.push(now.duration_since(last));
                            }
                            last_chunk_time = Some(now);
                            total_chunks += 1;
                        }
                    }
                    Err(e) => {
                        // Stream broken mid-flight (TCP reset, TLS error, premature close)
                        stream_error = true;
                        final_error_category = if e.is_timeout() {
                            ErrorCategory::Timeout
                        } else {
                            ErrorCategory::Network
                        };
                        break;
                    }
                }
            }

            let latency = start.elapsed();

            Ok(StreamTiming {
                status,
                success: !stream_error,
                latency,
                ttfb,
                chunk_intervals,
                total_chunks,
                error_category: final_error_category,
            })
        }
        Err(e) => {
            let latency = start.elapsed();
            let error_category = if e.is_timeout() {
                ErrorCategory::Timeout
            } else {
                ErrorCategory::Network
            };
            Ok(StreamTiming {
                status: StatusCode::default(),
                success: false,
                latency,
                ttfb: None,
                chunk_intervals: vec![],
                total_chunks: 0,
                error_category,
            })
        }
    }
}
