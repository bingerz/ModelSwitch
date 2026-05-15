/// Passive rate-limit header extraction from upstream responses.
/// Supports both `x-ratelimit-*` (OpenAI/Azure/pipeLLM) and
/// `anthropic-ratelimit-*` header prefixes.

/// Extracted rate-limit data from response headers.
#[derive(Debug, Clone, Default)]
pub struct QuotaHeaders {
    pub remaining_requests: Option<u64>,
    pub limit_requests: Option<u64>,
    pub remaining_tokens: Option<u64>,
    pub limit_tokens: Option<u64>,
}

impl QuotaHeaders {
    /// Extract rate-limit headers from an upstream response.
    /// Returns `Some(QuotaHeaders)` if any rate-limit data was found, `None` otherwise.
    pub fn extract(provider: &str, headers: &reqwest::header::HeaderMap) -> Option<Self> {
        let result = if provider == "anthropic" {
            Self::extract_anthropic(headers)
        } else {
            Self::extract_x_ratelimit(headers)
        };

        if result.remaining_requests.is_some()
            || result.limit_requests.is_some()
            || result.remaining_tokens.is_some()
            || result.limit_tokens.is_some()
        {
            Some(result)
        } else {
            None
        }
    }

    /// Extract `x-ratelimit-*` headers (OpenAI, Azure, pipeLLM, generic).
    fn extract_x_ratelimit(headers: &reqwest::header::HeaderMap) -> Self {
        let get = |key: &str| -> Option<u64> {
            headers.get(key).and_then(|v| v.to_str().ok()?.parse().ok())
        };

        Self {
            remaining_requests: get("x-ratelimit-remaining-requests")
                .or_else(|| get("x-ratelimit-remaining")),
            limit_requests: get("x-ratelimit-limit-requests")
                .or_else(|| get("x-ratelimit-limit")),
            remaining_tokens: get("x-ratelimit-remaining-tokens"),
            limit_tokens: get("x-ratelimit-limit-tokens"),
        }
    }

    /// Extract `anthropic-ratelimit-*` headers.
    fn extract_anthropic(headers: &reqwest::header::HeaderMap) -> Self {
        let get = |key: &str| -> Option<u64> {
            headers.get(key).and_then(|v| v.to_str().ok()?.parse().ok())
        };

        Self {
            remaining_requests: get("anthropic-ratelimit-requests-remaining"),
            limit_requests: get("anthropic-ratelimit-requests-limit"),
            remaining_tokens: get("anthropic-ratelimit-tokens-remaining"),
            limit_tokens: get("anthropic-ratelimit-tokens-limit"),
        }
    }
}
