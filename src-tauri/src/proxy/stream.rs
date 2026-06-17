use axum::body::Body;
use axum::response::Response;
use bytes::{Bytes, BytesMut};
use futures::stream::Stream;
use reqwest::StatusCode;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::Notify;
use tokio_stream::StreamExt;

/// Per-read timeout for upstream SSE streams to prevent stalled connections
/// from being held open indefinitely. Generous enough for slow LLM providers.
const STREAM_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Create an SSE streaming response that accumulates output bytes for
/// post-stream telemetry and cache insertion.
///
/// Uses an mpsc channel so that a background task owns the upstream stream.
/// If the client disconnects mid-stream, the task continues draining the
/// upstream to ensure telemetry (and therefore budget reconciliation) completes.
///
/// Returns the response, a unified output buffer that captures the exact bytes
/// delivered to the client (after any Gemini translation), and a `Notify` that
/// is signaled once the upstream stream has been fully consumed (allowing the
/// caller to react immediately without polling).
///
/// All SSE line parsing (`data:` extraction, usage detection) is deferred to
/// the post-stream telemetry task. The per-chunk hot path performs only a
/// single zero-copy `extend_from_slice` into the output buffer.
#[allow(clippy::type_complexity)]
pub fn sse_stream_response_with_telemetry(
    upstream_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    translate_gemini: bool,
    model: String,
) -> (
    Response,
    Arc<Mutex<BytesMut>>,
    Arc<Notify>,
) {
    use tokio::sync::mpsc;

    let output_buffer: Arc<Mutex<BytesMut>> = Arc::new(Mutex::new(BytesMut::new()));
    let output_buffer_clone = Arc::clone(&output_buffer);

    let stream_done: Arc<Notify> = Arc::new(Notify::new());

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    {
        let stream_done = Arc::clone(&stream_done);
        tokio::spawn(async move {
            let mut upstream = Box::pin(upstream_stream);

            loop {
                let next_result =
                    match tokio::time::timeout(STREAM_READ_TIMEOUT, upstream.next()).await {
                        Ok(Some(result)) => result,
                        Ok(None) => break, // stream ended normally
                        Err(_elapsed) => {
                            tracing::warn!(
                                "Stream read timeout ({}s) — upstream stalled, aborting stream",
                                STREAM_READ_TIMEOUT.as_secs()
                            );
                            let _ = tx
                                .send(Err(std::io::Error::new(
                                    std::io::ErrorKind::TimedOut,
                                    "upstream stream read timeout",
                                )))
                                .await;
                            break;
                        }
                    };
                let bytes = match next_result {
                    Ok(b) => b,
                    Err(e) => {
                        // Forward error to client if still connected
                        let _ = tx.send(Err(std::io::Error::other(e))).await;
                        break;
                    }
                };

                // Translate if needed (must happen per-chunk for real-time delivery)
                let output_bytes = if translate_gemini {
                    let text = String::from_utf8_lossy(&bytes);
                    use crate::proxy::translate::gemini_stream_to_openai;
                    let mut output = String::new();
                    for line in text.split('\n') {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with(':') {
                            output.push_str(line);
                            output.push('\n');
                            continue;
                        }
                        let json_str = trimmed.strip_prefix("data: ").unwrap_or(trimmed);
                        if json_str == "[DONE]" {
                            output.push_str("data: [DONE]\n\n");
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(translated) = gemini_stream_to_openai(&v, &model) {
                                output.push_str(&translated);
                            } else {
                                output.push_str(line);
                                output.push_str("\n\n");
                            }
                        } else {
                            output.push_str(line);
                            output.push('\n');
                        }
                    }
                    Bytes::from(output)
                } else {
                    bytes
                };

                // Accumulate output bytes for telemetry + cache (zero-parse hot path).
                // Single mutex lock + byte append, no SSE parsing on the hot path.
                {
                    let mut buf =
                        output_buffer_clone.lock().unwrap_or_else(|e| e.into_inner());
                    buf.extend_from_slice(&output_bytes);
                }

                // Forward to client — if disconnected, continue draining upstream
                // to ensure telemetry completes. Ignore SendError.
                if tx.send(Ok(output_bytes)).await.is_err() {
                    // Client disconnected — keep draining upstream for telemetry
                    // but don't attempt to forward.
                    loop {
                        let next_result = match tokio::time::timeout(
                            STREAM_READ_TIMEOUT,
                            upstream.next(),
                        )
                        .await
                        {
                            Ok(Some(result)) => result,
                            Ok(None) => break,
                            Err(_elapsed) => {
                                tracing::warn!(
                                    "Stream read timeout ({}s) — upstream stalled, aborting stream (drain path)",
                                    STREAM_READ_TIMEOUT.as_secs()
                                );
                                break;
                            }
                        };
                        if let Ok(bytes) = next_result {
                            // For Gemini, translate before accumulating so the
                            // output buffer mirrors what the client would have
                            // received. Fixes a pre-existing bug where the drain
                            // path accumulated raw upstream bytes instead of
                            // translated output for Gemini streams.
                            let drain_output = if translate_gemini {
                                let text = String::from_utf8_lossy(&bytes);
                                use crate::proxy::translate::gemini_stream_to_openai;
                                let mut output = String::new();
                                for line in text.split('\n') {
                                    let trimmed = line.trim();
                                    if trimmed.is_empty() || trimmed.starts_with(':') {
                                        output.push_str(line);
                                        output.push('\n');
                                        continue;
                                    }
                                    let json_str =
                                        trimmed.strip_prefix("data: ").unwrap_or(trimmed);
                                    if json_str == "[DONE]" {
                                        output.push_str("data: [DONE]\n\n");
                                        continue;
                                    }
                                    if let Ok(v) =
                                        serde_json::from_str::<serde_json::Value>(json_str)
                                    {
                                        if let Some(translated) =
                                            gemini_stream_to_openai(&v, &model)
                                        {
                                            output.push_str(&translated);
                                        } else {
                                            output.push_str(line);
                                            output.push_str("\n\n");
                                        }
                                    } else {
                                        output.push_str(line);
                                        output.push('\n');
                                    }
                                }
                                Bytes::from(output)
                            } else {
                                bytes
                            };
                            {
                                let mut buf = output_buffer_clone
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                buf.extend_from_slice(&drain_output);
                            }
                        }
                    }
                    break;
                }
            }

            // Signal that the upstream stream has been fully consumed so the
            // telemetry/budget reconciliation task can proceed immediately
            // without polling. `notify_one` stores a permit if no waiter is
            // registered yet, so ordering is not significant.
            stream_done.notify_one();
        });
    }

    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap();

    (response, output_buffer, stream_done)
}

/// Create a non-streaming JSON response.
pub fn json_response(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

/// Create an SSE response containing a single JSON chunk followed by [DONE].
/// Used when the MCP tool loop completes and the client originally requested streaming.
pub fn sse_single_chunk_response(status: StatusCode, body_json: &str) -> Response {
    let sse_body = format!("data: {}\n\ndata: [DONE]\n\n", body_json);
    Response::builder()
        .status(status)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from(sse_body))
        .unwrap()
}

/// Create the "all channels rate limited" error response.
pub fn all_channels_exhausted_response() -> Response {
    let body = serde_json::json!({
        "error": {
            "message": "All channels are rate-limited or unavailable. Please retry after a brief wait.",
            "type": "rate_limit_exhausted",
            "code": "all_channels_rate_limited"
        }
    });
    json_response(StatusCode::TOO_MANY_REQUESTS, body.to_string())
}

/// Create an SSE response from cached SSE text.
/// The cached text is the full SSE event stream including `data:` prefixes,
/// newlines, etc. The client receives it as a single body but the SSE format
/// is preserved, so the client parses it correctly.
pub fn sse_stream_response_with_cached(cached_body: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(Body::from(cached_body.to_string()))
        .unwrap()
}

/// Wrap a byte stream with keepalive SSE comments.
/// Spawns a background task that merges upstream data with periodic `: ping\n\n`
/// when no data has flowed for `interval_secs`.
pub fn keepalive_stream(
    upstream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    interval_secs: u64,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    use tokio::sync::mpsc;

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    tokio::spawn(async move {
        let mut upstream = Box::pin(upstream);
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // skip first immediate tick

        let mut data_sent = false;

        loop {
            tokio::select! {
                item = upstream.next() => {
                    match item {
                        Some(Ok(bytes)) => {
                            data_sent = true;
                            if tx.send(Ok(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Some(Err(e)) => {
                            let _ = tx.send(Err(e)).await;
                            break;
                        }
                        None => break,
                    }
                }
                _ = ticker.tick() => {
                    if !data_sent
                        && tx.send(Ok(Bytes::from(": ping\n\n"))).await.is_err()
                    {
                        break;
                    }
                    data_sent = false;
                }
            }
        }
    });

    tokio_stream::wrappers::ReceiverStream::new(rx)
}

/// Check if an SSE data line contains an error indicator.
/// Returns Some(reason) if an error is detected, None otherwise.
#[allow(dead_code)]
pub fn detect_sse_error(data: &str) -> Option<String> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return None;
    }

    // Strip "data: " prefix if present
    let json_str = trimmed.strip_prefix("data: ").unwrap_or(trimmed);

    // Quick check for "error" key before full JSON parse
    if !json_str.contains("\"error\"") {
        return None;
    }

    // Try to parse and extract error info
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(error) = v.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Some(msg.to_string());
        }
    }

    None
}

/// Build an SSE error event payload for mid-stream errors.
#[allow(dead_code)]
pub fn sse_error_event(message: &str) -> Bytes {
    let escaped = message.replace('"', "\\\"");
    Bytes::from(format!(
        "event: modelswitch_error\ndata: {{\"error\": \"{}\"}}\n\n",
        escaped
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_openai_error() {
        let data = r#"data: {"id":"chatcmpl-123","error":{"message":"Rate limit exceeded","type":"rate_limit_error","code":"rate_limit_exceeded"}}"#;
        assert_eq!(
            detect_sse_error(data),
            Some("Rate limit exceeded".to_string())
        );
    }

    #[test]
    fn detect_anthropic_error() {
        let data =
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        assert_eq!(detect_sse_error(data), Some("Overloaded".to_string()));
    }

    #[test]
    fn no_error_in_normal_chunk() {
        let data = r#"data: {"id":"chatcmpl-123","choices":[{"delta":{"content":"Hello"}}]}"#;
        assert_eq!(detect_sse_error(data), None);
    }

    #[test]
    fn no_error_in_done() {
        assert_eq!(detect_sse_error("[DONE]"), None);
        assert_eq!(detect_sse_error(""), None);
        assert_eq!(detect_sse_error("data: "), None);
    }

    #[tokio::test]
    async fn raw_sse_accumulates_all_chunks() {
        use bytes::Bytes;
        use futures::stream;

        let chunks = vec![
            Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n"),
            Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n"),
            Bytes::from("data: [DONE]\n\n"),
        ];
        let upstream = stream::iter(chunks.into_iter().map(Ok::<_, reqwest::Error>));

        let (_response, output_buffer, stream_done) =
            sse_stream_response_with_telemetry(upstream, false, "gpt-4".to_string());

        // Consume the response body so the background task completes
        let body = _response.into_body();
        let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        assert!(!body_bytes.is_empty());

        // Wait for the stream task to finish writing all chunks
        stream_done.notified().await;

        let buf = output_buffer.lock().unwrap_or_else(|e| e.into_inner());
        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("Hello"),
            "output_buffer should contain 'Hello': {:?}",
            raw
        );
        assert!(
            raw.contains("world"),
            "output_buffer should contain 'world': {:?}",
            raw
        );
        assert!(
            raw.contains("[DONE]"),
            "output_buffer should contain '[DONE]': {:?}",
            raw
        );
        assert!(
            raw.contains("data: "),
            "output_buffer should contain 'data: ': {:?}",
            raw
        );
    }
}
