use axum::body::Body;
use axum::response::Response;
use bytes::Bytes;
use futures::stream::Stream;
use reqwest::StatusCode;
use std::sync::Arc;
use std::sync::Mutex;
use tokio_stream::StreamExt;

/// Create an SSE streaming response that accumulates chunks for telemetry.
/// Returns the response and a `StreamTelemetry` handle for post-stream cost tracking.
pub fn sse_stream_response_with_telemetry(
    upstream_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    translate_gemini: bool,
    model: String,
) -> (Response, Arc<Mutex<Vec<String>>>) {
    let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = Arc::clone(&chunks);

    let mapped = upstream_stream.map(move |result: Result<Bytes, reqwest::Error>| {
        let bytes = match result {
            Ok(b) => b,
            Err(e) => {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
            }
        };

        let text = String::from_utf8_lossy(&bytes);

        // Accumulate data lines for telemetry
        {
            let mut guard = chunks_clone.lock().unwrap_or_else(|e| e.into_inner());
            for line in text.split('\n') {
                let trimmed = line.trim();
                if trimmed.starts_with("data: ") {
                    let data = trimmed.strip_prefix("data: ").unwrap_or("");
                    if !data.is_empty() && data != "[DONE]" {
                        guard.push(data.to_string());
                    }
                }
            }
        }

        // If translating Gemini stream, transform chunks
        if translate_gemini {
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
            Ok(Bytes::from(output))
        } else {
            Ok(bytes)
        }
    });

    let body = Body::from_stream(mapped);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap();

    (response, chunks)
}

/// Create a non-streaming JSON response.
pub fn json_response(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
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
                    if !data_sent {
                        if tx.send(Ok(Bytes::from(": ping\n\n"))).await.is_err() {
                            break;
                        }
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
}
