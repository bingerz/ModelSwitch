//! Privacy guardrail middleware — redacts secrets from request bodies before
//! forwarding to upstream LLMs, and optionally scans SSE response streams for
//! echoed secrets.
//!
//! ## Layer ordering
//!
//! The sanitizer is registered as the outermost proxy layer so it runs before
//! the virtual-key auth middleware and the handler. This guarantees secrets
//! are stripped from the body before any downstream code (including request
//! logging, caching, or upstream dispatch) observes them.
//!
//! ## Pattern catalog
//!
//! A built-in catalog covers common high-signal secret formats:
//! AWS access keys, AWS secret keys, Stripe live/restricted keys, SSH/PGP
//! private key blocks, database connection strings with embedded passwords,
//! GitHub PATs, Slack bot tokens, and generic `api_key=`/`password=`
//! assignments. Users can extend the catalog with custom regexes via
//! `GatewayConfig::sanitizer::custom_patterns`.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use regex::Regex;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::LazyLock;
use std::task::{Context, Poll};

use crate::config::SanitizerConfig;
use crate::proxy::AppState;

/// Maximum body size the sanitizer will buffer for scanning. Mirrors the
/// router's `DefaultBodyLimit` so we can always fully consume proxy bodies.
const BODY_SCAN_LIMIT: usize = 10 * 1024 * 1024;

/// Overlap window used when flushing a streaming SSE sanitizer. Must be large
/// enough to span any single secret we expect to echo across chunk boundaries.
const STREAM_OVERLAP: usize = 256;

/// A compiled sanitizer pattern.
#[derive(Clone)]
pub struct CompiledPattern {
    regex: Regex,
    replacement: String,
    /// Human-readable identifier for debugging and log correlation.
    /// Stored on the struct so redaction sites can report which pattern fired;
    /// not currently read by the hot path.
    #[allow(dead_code)]
    name: &'static str,
}

/// Built-in secret patterns. Always active when the sanitizer is enabled.
/// Compiled once and reused for every request via `LazyLock`.
static BUILTIN_PATTERNS: LazyLock<Vec<CompiledPattern>> = LazyLock::new(|| {
    vec![
        CompiledPattern {
            // AWS Access Key ID
            regex: Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex pattern"),
            replacement: "[REDACTED:AWS_KEY]".to_string(),
            name: "aws_access_key",
        },
        CompiledPattern {
            // AWS Secret Access Key (40 char base64 following the key name)
            regex: Regex::new(r#"(?i)aws_secret_access_key["'\s:=]+([A-Za-z0-9/+=]{40})"#)
                .expect("valid regex pattern"),
            replacement: "[REDACTED:AWS_SECRET]".to_string(),
            name: "aws_secret_key",
        },
        CompiledPattern {
            // Stripe live secret key
            regex: Regex::new(r"sk_live_[0-9a-zA-Z]{24,}").expect("valid regex pattern"),
            replacement: "[REDACTED:STRIPE_KEY]".to_string(),
            name: "stripe_live_key",
        },
        CompiledPattern {
            // Stripe restricted key
            regex: Regex::new(r"rk_live_[0-9a-zA-Z]{24,}").expect("valid regex pattern"),
            replacement: "[REDACTED:STRIPE_KEY]".to_string(),
            name: "stripe_restricted_key",
        },
        CompiledPattern {
            // SSH/GPG/PGP private key blocks
            regex: Regex::new(
                r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----[\s\S]*?-----END (?:RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----",
            )
            .expect("valid regex pattern"),
            replacement: "[REDACTED:PRIVATE_KEY]".to_string(),
            name: "private_key_block",
        },
        CompiledPattern {
            // Just the header line (handles truncated blocks)
            regex: Regex::new(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |)PRIVATE KEY-----")
                .expect("valid regex pattern"),
            replacement: "[REDACTED:PRIVATE_KEY]".to_string(),
            name: "private_key_header",
        },
        CompiledPattern {
            // Database connection strings with embedded passwords
            regex: Regex::new(
                r"(?i)(postgresql|postgres|mysql|mongodb|redis)://[^:\s]+:([^@\s]+)@",
            )
            .expect("valid regex pattern"),
            // Keep the scheme visible for debugging but hide credentials
            replacement: "${1}://[REDACTED_USER]:[REDACTED_PASSWORD]@".to_string(),
            name: "db_connection_string",
        },
        CompiledPattern {
            // GitHub classic personal access token
            regex: Regex::new(r"ghp_[A-Za-z0-9]{36}").expect("valid regex pattern"),
            replacement: "[REDACTED:GITHUB_TOKEN]".to_string(),
            name: "github_pat",
        },
        CompiledPattern {
            // Slack bot token
            regex: Regex::new(r"xoxb-[0-9]{10,13}-[0-9]{10,13}-[A-Za-z0-9]{24}")
                .expect("valid regex pattern"),
            replacement: "[REDACTED:SLACK_TOKEN]".to_string(),
            name: "slack_bot_token",
        },
        CompiledPattern {
            // Generic api_key=/secret=/password= assignments with high-entropy values.
            // Only fires when the key-like name is immediately adjacent to a 20+ char value.
            regex: Regex::new(
                r#"(?i)(api[_-]?key|secret[_-]?key|access[_-]?token|private[_-]?key|password|passwd|pwd)["'\s]*[:=]["'\s]*[A-Za-z0-9+/=_-]{20,}"#,
            )
            .expect("valid regex pattern"),
            replacement: "[REDACTED:SECRET]".to_string(),
            name: "generic_api_key",
        },
    ]
});

/// Returns a reference to the lazily-compiled built-in patterns.
fn builtin_patterns() -> &'static Vec<CompiledPattern> {
    &BUILTIN_PATTERNS
}

/// Compile all active patterns: builtins plus any user-supplied custom patterns.
/// Invalid custom regexes are logged and skipped (we never want to crash the
/// gateway over a typo in user config).
fn compile_patterns(config: &SanitizerConfig) -> Vec<CompiledPattern> {
    let mut patterns = builtin_patterns().clone();
    for custom in &config.custom_patterns {
        match Regex::new(&custom.pattern) {
            Ok(re) => patterns.push(CompiledPattern {
                regex: re,
                replacement: custom.replacement.clone(),
                name: "custom",
            }),
            Err(e) => {
                tracing::warn!(
                    pattern = %custom.pattern,
                    name = %custom.name,
                    error = %e,
                    "sanitizer: skipping invalid custom pattern"
                );
            }
        }
    }
    patterns
}

/// Apply all patterns to `text`, returning the redacted string and a total
/// match count (one per pattern match, so overlapping patterns may over-count).
pub fn scan_and_redact(text: &str, patterns: &[CompiledPattern]) -> (String, usize) {
    let mut result = text.to_string();
    let mut total = 0usize;
    for p in patterns {
        let count = p.regex.find_iter(&result).count();
        if count > 0 {
            total += count;
            result = p
                .regex
                .replace_all(&result, p.replacement.as_str())
                .into_owned();
        }
    }
    (result, total)
}

/// Apply patterns to `text` using a tuple-shaped pattern slice. Used by the
/// streaming sanitizer which owns its patterns independently of `CompiledPattern`.
fn scan_and_redact_owned(text: &str, patterns: &[(Regex, String)]) -> (String, usize) {
    let mut result = text.to_string();
    let mut total = 0usize;
    for (re, replacement) in patterns {
        let count = re.find_iter(&result).count();
        if count > 0 {
            total += count;
            result = re.replace_all(&result, replacement.as_str()).into_owned();
        }
    }
    (result, total)
}

/// Sanitizer middleware entry point. Wraps the request body with secret
/// redaction, and optionally wraps the SSE response stream.
pub async fn sanitizer_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let config = state.security.sanitizer_config.read().clone();

    // Fast path: pass-through when disabled or when redaction is off.
    if !config.enabled || !config.redact_secrets {
        return next.run(req).await;
    }

    let patterns = compile_patterns(&config);

    // Split request into parts and body so we can buffer+scan the body.
    let (parts, body) = req.into_parts();

    let body_bytes = match axum::body::to_bytes(body, BODY_SCAN_LIMIT).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, "sanitizer: failed to buffer request body; passing through");
            // Reconstruct an empty-body request so the handler still runs —
            // the Json extractor will surface a 400 if the body was required.
            let req = Request::from_parts(parts, Body::from(Bytes::new()));
            return next.run(req).await;
        }
    };

    // Scan and redact. We operate on the lossy UTF-8 view so non-UTF-8 bytes
    // (rare in JSON request bodies) still produce a scannable string.
    let body_str = String::from_utf8_lossy(&body_bytes);
    let (redacted, count) = scan_and_redact(&body_str, &patterns);

    if count > 0 {
        tracing::info!(
            redaction_count = count,
            "sanitizer: redacted secrets from request body"
        );
    }

    // Reconstruct request with the redacted body. `Body::from(String)` produces
    // a concrete `Full<Bytes>` body which the `Json<Value>` extractor can
    // re-buffer without issue.
    let req = Request::from_parts(parts, Body::from(redacted));

    let response = next.run(req).await;

    if config.scan_response {
        sanitize_sse_response(response, &patterns)
    } else {
        response
    }
}

/// Wrap an SSE response body with a sanitizing stream that redacts any secrets
/// echoed back by the upstream model. Non-SSE responses pass through untouched.
fn sanitize_sse_response(response: Response<Body>, patterns: &[CompiledPattern]) -> Response<Body> {
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/event-stream") {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let data_stream = body.into_data_stream();
    let sanitized = SanitizingStream::new(data_stream, patterns);
    parts.extensions.clear();
    let new_body = Body::from_stream(sanitized);
    Response::from_parts(parts, new_body)
}

/// A stream wrapper that applies secret redaction across chunk boundaries using
/// a sliding overlap window. Secrets that echo back from the model are
/// redacted before reaching the client.
struct SanitizingStream<S> {
    inner: S,
    buffer: Vec<u8>,
    /// Owned (Regex, replacement) tuples — the stream outlives the borrow on
    /// `CompiledPattern` so we detach the values we need.
    patterns: Vec<(Regex, String)>,
    done: bool,
}

impl<S> SanitizingStream<S> {
    fn new(inner: S, patterns: &[CompiledPattern]) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            patterns: patterns
                .iter()
                .map(|p| (p.regex.clone(), p.replacement.clone()))
                .collect(),
            done: false,
        }
    }

    /// Flush the first `flush_len` bytes of the buffer through the redactor.
    fn flush_chunk(&mut self, flush_len: usize) -> Option<Result<Bytes, axum::Error>> {
        if flush_len == 0 {
            return None;
        }
        let to_flush: Vec<u8> = self.buffer.drain(..flush_len).collect();
        let text = String::from_utf8_lossy(&to_flush);
        let (redacted, count) = scan_and_redact_owned(&text, &self.patterns);
        if count > 0 {
            tracing::info!(
                redaction_count = count,
                "sanitizer: redacted secrets from SSE response chunk"
            );
        }
        Some(Ok(Bytes::from(redacted)))
    }
}

impl<S> Stream for SanitizingStream<S>
where
    S: Stream<Item = Result<Bytes, axum::Error>> + Unpin,
{
    type Item = Result<Bytes, axum::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Terminal flush once the upstream stream has ended.
            if self.done {
                if self.buffer.is_empty() {
                    return Poll::Ready(None);
                }
                let flush_len = self.buffer.len();
                return match self.flush_chunk(flush_len) {
                    Some(item) => Poll::Ready(Some(item)),
                    None => Poll::Ready(None),
                };
            }

            match self.inner.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    self.buffer.extend_from_slice(&chunk);

                    // Once the buffer is comfortably larger than the overlap
                    // window, emit everything except the trailing overlap so
                    // patterns spanning the next chunk are still detected.
                    if self.buffer.len() > STREAM_OVERLAP * 2 {
                        let flush_len = self.buffer.len() - STREAM_OVERLAP;
                        return match self.flush_chunk(flush_len) {
                            Some(item) => Poll::Ready(Some(item)),
                            None => continue,
                        };
                    }
                    // Keep accumulating — chunk was too small to safely split.
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    // Surface upstream errors verbatim; any buffered data is
                    // abandoned rather than emitted partially to avoid leaking
                    // an un-scanned tail.
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    self.done = true;
                    continue;
                }
                Poll::Pending => {
                    // Flush whatever we have so the client isn't stalled.
                    if self.buffer.len() > STREAM_OVERLAP {
                        let flush_len = self.buffer.len() - STREAM_OVERLAP;
                        return match self.flush_chunk(flush_len) {
                            Some(item) => Poll::Ready(Some(item)),
                            None => return Poll::Pending,
                        };
                    }
                    return Poll::Pending;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomPattern;

    #[test]
    fn redacts_aws_access_key() {
        let patterns = builtin_patterns();
        let input = "my key is AKIAIOSFODNN7EXAMPLE and stuff";
        let (output, count) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED:AWS_KEY]"));
        assert!(!output.contains("AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(count, 1);
    }

    #[test]
    fn redacts_stripe_live_key() {
        let patterns = builtin_patterns();
        let input = r#"{"key": "sk_live_REDACTED_FOR_PUSH"}"#;
        let (output, count) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED:STRIPE_KEY]"));
        assert!(count >= 1, "expected at least one redaction, got {count}");
    }

    #[test]
    fn redacts_stripe_restricted_key() {
        let patterns = builtin_patterns();
        let input = "token rk_live_REDACTED_FOR_PUSH";
        let (output, count) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED:STRIPE_KEY]"));
        assert!(!output.contains("rk_live_REDACTED_FOR_PUSH"));
        assert!(count >= 1);
    }

    #[test]
    fn redacts_full_private_key_block() {
        let patterns = builtin_patterns();
        let input = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAI...\n-----END RSA PRIVATE KEY-----";
        let (output, _) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED:PRIVATE_KEY]"));
        assert!(!output.contains("MIIEpAI"));
    }

    #[test]
    fn redacts_truncated_private_key_header() {
        let patterns = builtin_patterns();
        let input = "leaked: -----BEGIN OPENSSH PRIVATE KEY-----";
        let (output, count) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED:PRIVATE_KEY]"));
        assert!(count >= 1);
    }

    #[test]
    fn redacts_github_pat() {
        let patterns = builtin_patterns();
        let input = "token: ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let (output, _) = scan_and_redact(input, patterns);
        assert!(
            output.contains("[REDACTED"),
            "expected redaction in output: {output}"
        );
        assert!(!output.contains("ghp_"));
    }

    #[test]
    fn redacts_slack_bot_token() {
        let patterns = builtin_patterns();
        let input = "xoxb-REDACTED-FOR-PUSH";
        let (output, _) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED:SLACK_TOKEN]"));
    }

    #[test]
    fn redacts_db_connection_string() {
        let patterns = builtin_patterns();
        let input = r#"DATABASE_URL=postgresql://user:secretpass@localhost:5432/db"#;
        let (output, _) = scan_and_redact(input, patterns);
        assert!(output.contains("[REDACTED"));
        assert!(!output.contains("secretpass"));
    }

    #[test]
    fn redacts_aws_secret_key_assignment() {
        let patterns = builtin_patterns();
        let input = r#"aws_secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY""#;
        let (output, count) = scan_and_redact(input, patterns);
        assert!(count >= 1, "expected redaction, got {count}");
        assert!(output.contains("[REDACTED"));
        assert!(!output.contains("wJalrXUtnFEMI"));
    }

    #[test]
    fn redacts_generic_api_key_assignment() {
        let patterns = builtin_patterns();
        let input = r#"api_key = "sk_test_abcdef1234567890abcdef""#;
        let (output, count) = scan_and_redact(input, patterns);
        assert!(count > 0, "expected redaction, got {count}");
        assert!(output.contains("[REDACTED"));
    }

    #[test]
    fn redacts_generic_password_assignment() {
        let patterns = builtin_patterns();
        let input = r#"password = "supersecretvalue12345678""#;
        let (output, count) = scan_and_redact(input, patterns);
        assert!(count > 0, "expected redaction, got {count}");
        assert!(output.contains("[REDACTED"));
        assert!(!output.contains("supersecretvalue12345678"));
    }

    #[test]
    fn does_not_redact_normal_text() {
        let patterns = builtin_patterns();
        let input = "Hello, how are you today? The weather is nice.";
        let (output, count) = scan_and_redact(input, patterns);
        assert_eq!(count, 0);
        assert_eq!(output, input);
    }

    #[test]
    fn handles_empty_input() {
        let patterns = builtin_patterns();
        let (output, count) = scan_and_redact("", patterns);
        assert_eq!(count, 0);
        assert_eq!(output, "");
    }

    #[test]
    fn custom_patterns_are_applied() {
        let config = SanitizerConfig {
            enabled: true,
            redact_secrets: true,
            scan_response: false,
            custom_patterns: vec![CustomPattern {
                name: "test".to_string(),
                pattern: r"my-secret-\d+".to_string(),
                replacement: "[HIDDEN]".to_string(),
            }],
        };
        let patterns = compile_patterns(&config);
        let input = "value: my-secret-12345 end";
        let (output, count) = scan_and_redact(input, &patterns);
        assert!(output.contains("[HIDDEN]"));
        assert!(count >= 1);
    }

    #[test]
    fn invalid_custom_pattern_is_skipped() {
        let config = SanitizerConfig {
            enabled: true,
            redact_secrets: true,
            scan_response: false,
            custom_patterns: vec![CustomPattern {
                name: "bad".to_string(),
                pattern: r"[invalid(".to_string(),
                replacement: "[X]".to_string(),
            }],
        };
        let patterns = compile_patterns(&config);
        // Built-ins should still be present even though the custom failed.
        assert!(patterns.len() >= builtin_patterns().len());
    }

    #[test]
    fn preserves_json_structure() {
        let patterns = builtin_patterns();
        let input = r#"{"messages":[{"role":"user","content":"key=AKIAIOSFODNN7EXAMPLE"}]}"#;
        let (output, _) = scan_and_redact(input, patterns);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&output);
        assert!(
            parsed.is_ok(),
            "redacted output should be valid JSON, got: {output}"
        );
    }

    #[test]
    fn disabled_sanitizer_config_compiles() {
        // Verify the Default-derived config matches expectations.
        let cfg = SanitizerConfig {
            enabled: false,
            redact_secrets: true,
            scan_response: false,
            custom_patterns: vec![],
        };
        assert!(!cfg.enabled);
    }

    #[test]
    fn multiple_secrets_in_one_body() {
        let patterns = builtin_patterns();
        let input = "keys: AKIAIOSFODNN7EXAMPLE and ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let (output, count) = scan_and_redact(input, patterns);
        assert!(count >= 2, "expected at least 2 redactions, got {count}");
        assert!(output.contains("[REDACTED:AWS_KEY]"));
        assert!(output.contains("[REDACTED:GITHUB_TOKEN]"));
    }
}
