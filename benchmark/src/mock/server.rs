use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
    routing::post,
    Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::net::TcpListener;
use uuid::Uuid;

/// Configuration for the mock LLM server.
#[derive(Clone)]
pub struct MockConfig {
    pub delay_ms: u64,
    pub fail_rate_pct: u8,
    pub tokens: usize,
    pub port: u16,
    /// Override chunk count for streaming. None = derive from tokens.
    pub stream_chunks: Option<usize>,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            delay_ms: 200,
            fail_rate_pct: 0,
            tokens: 100,
            port: 0,
            stream_chunks: None,
        }
    }
}

/// Handle to a running mock server.
pub struct MockServer {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl MockServer {
    pub fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Start the mock LLM server. Returns the server handle and the actual bound port.
pub async fn start(config: MockConfig) -> Result<(MockServer, u16)> {
    let addr = format!("127.0.0.1:{}", config.port);
    let listener = TcpListener::bind(&addr).await?;
    let bound_port = listener.local_addr()?.port();
    let state = Arc::new(config);

    let app = Router::new()
        .route("/v1/chat/completions", post(handle_openai))
        .route("/v1/messages", post(handle_anthropic))
        .route(
            "/v1beta/models/{model_action}",
            post(handle_gemini),
        )
        .with_state(state);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });

    tracing::info!("Mock LLM server started on port {}", bound_port);
    Ok((MockServer { shutdown_tx }, bound_port))
}

fn generate_content(tokens: usize) -> String {
    let words: Vec<&str> = vec![
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "do",
        "eiusmod",
        "tempor",
    ];
    words
        .iter()
        .cycle()
        .take(tokens)
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn should_fail(fail_rate_pct: u8) -> bool {
    if fail_rate_pct == 0 {
        return false;
    }
    let rand_val = Uuid::new_v4().as_u128() % 100;
    rand_val < fail_rate_pct as u128
}

/// Compute chunk count: explicit override or derive from tokens.
fn chunk_count(config: &MockConfig) -> usize {
    config
        .stream_chunks
        .unwrap_or_else(|| (config.tokens / 5).max(1))
}

// ── Shared logic ──

async fn maybe_delay(config: &MockConfig) {
    if config.delay_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(config.delay_ms)).await;
    }
}

// ── OpenAI handler ──

async fn handle_openai(
    State(config): State<Arc<MockConfig>>,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<Value>)> {
    let body_json: Value = serde_json::from_slice(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid body"})),
        )
    })?;

    let is_stream = body_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    maybe_delay(&config).await;

    if should_fail(config.fail_rate_pct) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": {"message": "Rate limit", "type": "rate_limit_error"}})),
        ));
    }

    if is_stream {
        Ok(stream_openai_response(config).into_response())
    } else {
        Ok(Json(json_openai_response(&config)).into_response())
    }
}

fn json_openai_response(config: &MockConfig) -> Value {
    let content = generate_content(config.tokens);
    json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": timestamp(),
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": config.tokens,
            "total_tokens": config.tokens + 10,
        }
    })
}

fn stream_openai_response(
    config: Arc<MockConfig>,
) -> Sse<tokio_stream::wrappers::ReceiverStream<Result<Event, std::convert::Infallible>>> {
    let chunks = chunk_count(&config);
    let (tx, rx) = tokio::sync::mpsc::channel(chunks + 2);
    let id = Uuid::new_v4().to_string();

    tokio::spawn(async move {
        let content = generate_content(config.tokens);
        let words: Vec<&str> = content.split_whitespace().collect();
        let per_chunk = (words.len() / chunks).max(1);

        for i in 0..chunks {
            let start = i * per_chunk;
            let end = std::cmp::min(start + per_chunk, words.len());
            let chunk_text: String = if start < end {
                words[start..end].join(" ")
            } else {
                String::new()
            };

            let is_last = i == chunks - 1;
            let chunk = json!({
                "id": &id,
                "object": "chat.completion.chunk",
                "created": timestamp(),
                "model": "mock-model",
                "choices": [{
                    "index": 0,
                    "delta": {"content": format!("{} ", chunk_text)},
                    "finish_reason": if is_last { json!("stop") } else { Value::Null },
                }]
            });

            let _ = tx.send(Ok(Event::default().data(chunk.to_string()))).await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

// ── Anthropic handler ──

async fn handle_anthropic(
    State(config): State<Arc<MockConfig>>,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<Value>)> {
    let body_json: Value = serde_json::from_slice(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"type": "error", "error": {"type": "invalid_request_error", "message": "invalid body"}})),
        )
    })?;

    let is_stream = body_json
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    maybe_delay(&config).await;

    if should_fail(config.fail_rate_pct) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(
                json!({"type": "error", "error": {"type": "rate_limit_error", "message": "Rate limit"}}),
            ),
        ));
    }

    if is_stream {
        Ok(stream_anthropic_response(config).into_response())
    } else {
        Ok(Json(json_anthropic_response(&config)).into_response())
    }
}

fn json_anthropic_response(config: &MockConfig) -> Value {
    let content = generate_content(config.tokens);
    json!({
        "id": format!("msg_{}", Uuid::new_v4()),
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": content}],
        "model": "mock-model",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": config.tokens,
        }
    })
}

fn stream_anthropic_response(
    config: Arc<MockConfig>,
) -> Sse<tokio_stream::wrappers::ReceiverStream<Result<Event, std::convert::Infallible>>> {
    let chunks = chunk_count(&config);
    let (tx, rx) = tokio::sync::mpsc::channel(chunks + 4);
    let msg_id = format!("msg_{}", Uuid::new_v4());

    tokio::spawn(async move {
        let content = generate_content(config.tokens);
        let words: Vec<&str> = content.split_whitespace().collect();
        let per_chunk = (words.len() / chunks).max(1);

        // message_start event
        let start_event = json!({
            "type": "message_start",
            "message": {
                "id": &msg_id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "mock-model",
                "stop_reason": null,
                "usage": {"input_tokens": 10, "output_tokens": 0}
            }
        });
        let _ = tx
            .send(Ok(Event::default()
                .event("message_start")
                .data(start_event.to_string())))
            .await;

        for i in 0..chunks {
            let start = i * per_chunk;
            let end = std::cmp::min(start + per_chunk, words.len());
            let chunk_text: String = if start < end {
                words[start..end].join(" ")
            } else {
                String::new()
            };

            let delta_event = json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": format!("{} ", chunk_text)}
            });
            let _ = tx
                .send(Ok(Event::default()
                    .event("content_block_delta")
                    .data(delta_event.to_string())))
                .await;

            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // message_stop event
        let _ = tx
            .send(Ok(Event::default()
                .event("message_stop")
                .data(json!({"type": "message_stop"}).to_string())))
            .await;
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

// ── Gemini handler ──
//
// Gemini URLs look like `/v1beta/models/{model}:generateContent` or
// `/v1beta/models/{model}:streamGenerateContent`. The model name and action
// share a single path segment separated by `:`, so we capture them together
// and split on the first `:`.

async fn handle_gemini(
    State(config): State<Arc<MockConfig>>,
    Path(model_action): Path<String>,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (StatusCode, Json<Value>)> {
    // Body must be valid JSON for generateContent; streamGenerateContent may
    // also POST JSON. Reject malformed bodies.
    let _body_json: Value = serde_json::from_slice(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": 400, "message": "invalid body"}})),
        )
    })?;

    let is_stream = model_action.ends_with(":streamGenerateContent")
        || model_action.ends_with("%3AstreamGenerateContent");

    maybe_delay(&config).await;

    if should_fail(config.fail_rate_pct) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": {"code": 429, "message": "Rate limit", "status": "RESOURCE_EXHAUSTED"}})),
        ));
    }

    if is_stream {
        Ok(stream_gemini_response(config).into_response())
    } else {
        Ok(Json(json_gemini_response(&config)).into_response())
    }
}

fn json_gemini_response(config: &MockConfig) -> Value {
    let content = generate_content(config.tokens);
    json!({
        "candidates": [{
            "content": {"parts": [{"text": content}], "role": "model"},
            "finishReason": "STOP",
            "index": 0,
        }],
        "usageMetadata": {
            "promptTokenCount": 10,
            "candidatesTokenCount": config.tokens,
            "totalTokenCount": config.tokens + 10,
        }
    })
}

fn stream_gemini_response(
    config: Arc<MockConfig>,
) -> Sse<tokio_stream::wrappers::ReceiverStream<Result<Event, std::convert::Infallible>>> {
    let chunks = chunk_count(&config);
    let (tx, rx) = tokio::sync::mpsc::channel(chunks + 2);

    tokio::spawn(async move {
        let content = generate_content(config.tokens);
        let words: Vec<&str> = content.split_whitespace().collect();
        let per_chunk = (words.len() / chunks).max(1);

        for i in 0..chunks {
            let start = i * per_chunk;
            let end = std::cmp::min(start + per_chunk, words.len());
            let chunk_text: String = if start < end {
                words[start..end].join(" ")
            } else {
                String::new()
            };

            let is_last = i == chunks - 1;
            let chunk = json!({
                "candidates": [{
                    "content": {
                        "parts": [{"text": format!("{} ", chunk_text)}],
                        "role": "model"
                    },
                    "finishReason": if is_last { json!("STOP") } else { Value::Null },
                    "index": 0,
                }]
            });

            let _ = tx.send(Ok(Event::default().data(chunk.to_string()))).await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}
