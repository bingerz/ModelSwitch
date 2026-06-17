use reqwest::RequestBuilder;
use serde_json::Value;

/// Validate and sanitize a model name for safe URL interpolation.
///
/// Model names must be flat identifiers (e.g. `gpt-4`, `claude-3-opus-20240229`,
/// `gemini-1.5-pro`). This function:
/// - Removes any character outside `[a-zA-Z0-9._:-]`
/// - Strips leading/trailing dots and collapses consecutive dots to prevent
///   path-traversal sequences (`..`)
/// - Rejects empty results (returns "unknown" as fallback)
pub(super) fn sanitize_model_for_url(model: &str) -> String {
    let mut sanitized: String = model
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'))
        .collect();

    while sanitized.contains("..") {
        sanitized = sanitized.replace("..", ".");
    }
    sanitized = sanitized.trim_matches('.').to_string();

    if sanitized.is_empty() {
        tracing::warn!(original = %model, "Model name was entirely unsafe, using fallback");
        return "unknown".to_string();
    }

    if sanitized != model {
        tracing::warn!(
            original = %model,
            sanitized = %sanitized,
            "Model name contained unsafe characters, sanitized for URL"
        );
    }
    sanitized
}

/// Provider-specific request building, authentication, and response handling.
///
/// Each LLM provider (OpenAI, Anthropic, Gemini) implements this trait to
/// encapsulate differences in URL construction, authentication headers,
/// request/response format translation, and streaming behavior.
pub(crate) trait ProviderAdaptor: Send + Sync {
    /// Default model name when the client doesn't specify one.
    fn default_model(&self) -> &'static str;

    /// Build the upstream URL for a request.
    fn build_url(&self, base_url: &str, upstream_model: &str, is_stream: bool) -> String;

    /// Apply provider-specific auth headers to the request builder.
    ///
    /// When `is_web_session` is true, use Cookie auth instead of the
    /// provider's native auth mechanism.
    fn apply_auth(
        &self,
        builder: RequestBuilder,
        api_key: &str,
        is_web_session: bool,
    ) -> RequestBuilder;

    /// Whether this provider needs `stream_options.include_usage` injection
    /// for streaming requests. Default: `false`.
    fn inject_stream_usage(&self) -> bool {
        false
    }

    /// Whether this provider's streaming uses Gemini-style SSE format.
    /// Default: `false`.
    fn is_gemini_stream(&self) -> bool {
        false
    }

    /// Translate the request body before forwarding upstream.
    /// Default: pass-through (return a clone).
    fn transform_request(&self, body: &Value) -> Value {
        body.clone()
    }

    /// Translate a non-streaming response body after receiving.
    /// Default: pass-through (return a clone).
    fn transform_response(&self, body: &Value, _model: &str) -> Value {
        body.clone()
    }
}

// ── OpenAI ──────────────────────────────────────────────────────────────────

/// Adaptor for OpenAI-compatible providers (Bearer auth, standard chat completions).
pub(crate) struct OpenAIAdaptor;

const OPENAI_DEFAULT_MODEL: &str = "gpt-3.5-turbo";
const OPENAI_UPSTREAM_PATH: &str = "v1/chat/completions";

impl ProviderAdaptor for OpenAIAdaptor {
    fn default_model(&self) -> &'static str {
        OPENAI_DEFAULT_MODEL
    }

    fn build_url(&self, base_url: &str, _upstream_model: &str, _is_stream: bool) -> String {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            OPENAI_UPSTREAM_PATH
        )
    }

    fn apply_auth(
        &self,
        builder: RequestBuilder,
        api_key: &str,
        is_web_session: bool,
    ) -> RequestBuilder {
        if is_web_session {
            builder
                .header("Cookie", api_key)
                .header("Content-Type", "application/json")
        } else {
            builder
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
        }
    }

    fn inject_stream_usage(&self) -> bool {
        true
    }
}

// ── Anthropic ───────────────────────────────────────────────────────────────

/// Adaptor for Anthropic (x-api-key auth, /v1/messages endpoint).
pub(crate) struct AnthropicAdaptor;

const ANTHROPIC_DEFAULT_MODEL: &str = "claude-3-5-sonnet-20241022";
const ANTHROPIC_UPSTREAM_PATH: &str = "v1/messages";

impl ProviderAdaptor for AnthropicAdaptor {
    fn default_model(&self) -> &'static str {
        ANTHROPIC_DEFAULT_MODEL
    }

    fn build_url(&self, base_url: &str, _upstream_model: &str, _is_stream: bool) -> String {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            ANTHROPIC_UPSTREAM_PATH
        )
    }

    fn apply_auth(
        &self,
        builder: RequestBuilder,
        api_key: &str,
        is_web_session: bool,
    ) -> RequestBuilder {
        if is_web_session {
            builder
                .header("Cookie", api_key)
                .header("Content-Type", "application/json")
        } else {
            builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
        }
    }
}

// ── Gemini ──────────────────────────────────────────────────────────────────

/// Adaptor for Google Gemini (x-goog-api-key, model-in-URL, format translation).
pub(crate) struct GeminiAdaptor;

const GEMINI_DEFAULT_MODEL: &str = "gemini-pro";

impl ProviderAdaptor for GeminiAdaptor {
    fn default_model(&self) -> &'static str {
        GEMINI_DEFAULT_MODEL
    }

    fn build_url(&self, base_url: &str, upstream_model: &str, is_stream: bool) -> String {
        let base = base_url.trim_end_matches('/');
        let model = sanitize_model_for_url(upstream_model);
        if is_stream {
            format!(
                "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
                base, model
            )
        } else {
            format!("{}/v1beta/models/{}:generateContent", base, model)
        }
    }

    fn apply_auth(
        &self,
        builder: RequestBuilder,
        api_key: &str,
        is_web_session: bool,
    ) -> RequestBuilder {
        if is_web_session {
            builder
                .header("Cookie", api_key)
                .header("Content-Type", "application/json")
        } else {
            builder
                .header("x-goog-api-key", api_key)
                .header("Content-Type", "application/json")
        }
    }

    fn is_gemini_stream(&self) -> bool {
        true
    }

    fn transform_request(&self, body: &Value) -> Value {
        crate::proxy::translate::openai_to_gemini(body)
    }

    fn transform_response(&self, body: &Value, model: &str) -> Value {
        crate::proxy::translate::gemini_to_openai(body, model)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- OpenAIAdaptor tests --

    #[test]
    fn openai_build_url_correct() {
        let adaptor = OpenAIAdaptor;
        let url = adaptor.build_url("https://api.openai.com/v1", "gpt-4", false);
        assert_eq!(url, "https://api.openai.com/v1/v1/chat/completions");
    }

    #[test]
    fn openai_build_url_trims_trailing_slash() {
        let adaptor = OpenAIAdaptor;
        let url = adaptor.build_url("https://api.openai.com/v1/", "gpt-4", true);
        assert_eq!(url, "https://api.openai.com/v1/v1/chat/completions");
    }

    #[test]
    fn openai_applies_bearer_auth() {
        let adaptor = OpenAIAdaptor;
        let client = reqwest::Client::new();
        let builder = client.post("http://test").json(&serde_json::json!({}));
        let builder = adaptor.apply_auth(builder, "sk-test-key", false);
        let request = builder.build().unwrap();
        let headers = request.headers();
        assert_eq!(headers.get("authorization").unwrap(), "Bearer sk-test-key");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn openai_applies_cookie_for_web_session() {
        let adaptor = OpenAIAdaptor;
        let client = reqwest::Client::new();
        let builder = client.post("http://test").json(&serde_json::json!({}));
        let builder = adaptor.apply_auth(builder, "session-cookie", true);
        let request = builder.build().unwrap();
        let headers = request.headers();
        assert_eq!(headers.get("cookie").unwrap(), "session-cookie");
        assert!(headers.get("authorization").is_none());
    }

    #[test]
    fn openai_injects_stream_usage() {
        let adaptor = OpenAIAdaptor;
        assert!(adaptor.inject_stream_usage());
    }

    #[test]
    fn openai_is_not_gemini_stream() {
        let adaptor = OpenAIAdaptor;
        assert!(!adaptor.is_gemini_stream());
    }

    // -- AnthropicAdaptor tests --

    #[test]
    fn anthropic_build_url_correct() {
        let adaptor = AnthropicAdaptor;
        let url = adaptor.build_url("https://api.anthropic.com", "claude-3", false);
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn anthropic_build_url_trims_trailing_slash() {
        let adaptor = AnthropicAdaptor;
        let url = adaptor.build_url("https://api.anthropic.com/", "claude-3", true);
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn anthropic_applies_api_key_auth() {
        let adaptor = AnthropicAdaptor;
        let client = reqwest::Client::new();
        let builder = client.post("http://test").json(&serde_json::json!({}));
        let builder = adaptor.apply_auth(builder, "sk-ant-key", false);
        let request = builder.build().unwrap();
        let headers = request.headers();
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-ant-key");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn anthropic_applies_cookie_for_web_session() {
        let adaptor = AnthropicAdaptor;
        let client = reqwest::Client::new();
        let builder = client.post("http://test").json(&serde_json::json!({}));
        let builder = adaptor.apply_auth(builder, "session-cookie", true);
        let request = builder.build().unwrap();
        let headers = request.headers();
        assert_eq!(headers.get("cookie").unwrap(), "session-cookie");
        assert!(headers.get("x-api-key").is_none());
    }

    #[test]
    fn anthropic_no_stream_usage() {
        let adaptor = AnthropicAdaptor;
        assert!(!adaptor.inject_stream_usage());
    }

    // -- GeminiAdaptor tests --

    #[test]
    fn gemini_build_url_stream() {
        let adaptor = GeminiAdaptor;
        let url = adaptor.build_url(
            "https://generativelanguage.googleapis.com",
            "gemini-pro",
            true,
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn gemini_build_url_non_stream() {
        let adaptor = GeminiAdaptor;
        let url = adaptor.build_url(
            "https://generativelanguage.googleapis.com",
            "gemini-pro",
            false,
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent"
        );
    }

    #[test]
    fn gemini_build_url_trims_trailing_slash() {
        let adaptor = GeminiAdaptor;
        let url = adaptor.build_url(
            "https://generativelanguage.googleapis.com/",
            "gemini-1.5-flash",
            true,
        );
        assert!(url.starts_with("https://generativelanguage.googleapis.com/v1beta/"));
    }

    #[test]
    fn gemini_build_url_sanitizes_model() {
        let adaptor = GeminiAdaptor;
        let url = adaptor.build_url("https://api.test", "../etc/passwd", false);
        assert!(
            !url.contains(".."),
            "URL should not contain path-traversal sequences: {}",
            url
        );
    }

    #[test]
    fn gemini_applies_goog_api_key() {
        let adaptor = GeminiAdaptor;
        let client = reqwest::Client::new();
        let builder = client.post("http://test").json(&serde_json::json!({}));
        let builder = adaptor.apply_auth(builder, "AIza-test-key", false);
        let request = builder.build().unwrap();
        let headers = request.headers();
        assert_eq!(headers.get("x-goog-api-key").unwrap(), "AIza-test-key");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn gemini_applies_cookie_for_web_session() {
        let adaptor = GeminiAdaptor;
        let client = reqwest::Client::new();
        let builder = client.post("http://test").json(&serde_json::json!({}));
        let builder = adaptor.apply_auth(builder, "session-cookie", true);
        let request = builder.build().unwrap();
        let headers = request.headers();
        assert_eq!(headers.get("cookie").unwrap(), "session-cookie");
        assert!(headers.get("x-goog-api-key").is_none());
    }

    #[test]
    fn gemini_is_gemini_stream() {
        let adaptor = GeminiAdaptor;
        assert!(adaptor.is_gemini_stream());
    }

    #[test]
    fn gemini_no_stream_usage() {
        let adaptor = GeminiAdaptor;
        assert!(!adaptor.inject_stream_usage());
    }

    // -- sanitize_model_for_url tests --

    #[test]
    fn sanitize_simple_model() {
        assert_eq!(sanitize_model_for_url("gpt-4"), "gpt-4");
    }

    #[test]
    fn sanitize_model_with_path_traversal() {
        let result = sanitize_model_for_url("../etc/passwd");
        assert!(!result.contains(".."));
        assert!(!result.contains("/"));
    }

    #[test]
    fn sanitize_empty_model_returns_fallback() {
        assert_eq!(sanitize_model_for_url(""), "unknown");
        assert_eq!(sanitize_model_for_url("!!!"), "unknown");
    }
}
