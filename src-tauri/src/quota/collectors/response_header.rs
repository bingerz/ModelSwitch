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
            limit_requests: get("x-ratelimit-limit-requests").or_else(|| get("x-ratelimit-limit")),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_header(key: &str, val: &str) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
            reqwest::header::HeaderValue::from_str(val).unwrap(),
        );
        headers
    }

    #[test]
    fn extract_x_ratelimit_full_headers() {
        let headers = {
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("x-ratelimit-remaining-requests", "42".parse().unwrap());
            h.insert("x-ratelimit-limit-requests", "100".parse().unwrap());
            h.insert("x-ratelimit-remaining-tokens", "5000".parse().unwrap());
            h.insert("x-ratelimit-limit-tokens", "10000".parse().unwrap());
            h
        };

        let result = QuotaHeaders::extract("openai", &headers).unwrap();
        assert_eq!(result.remaining_requests, Some(42));
        assert_eq!(result.limit_requests, Some(100));
        assert_eq!(result.remaining_tokens, Some(5000));
        assert_eq!(result.limit_tokens, Some(10000));
    }

    #[test]
    fn extract_x_ratelimit_fallback_short_keys() {
        // Some providers use shorter keys without the "-requests" suffix
        let headers = {
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("x-ratelimit-remaining", "7".parse().unwrap());
            h.insert("x-ratelimit-limit", "20".parse().unwrap());
            h
        };

        let result = QuotaHeaders::extract("azure", &headers).unwrap();
        assert_eq!(result.remaining_requests, Some(7));
        assert_eq!(result.limit_requests, Some(20));
        assert_eq!(result.remaining_tokens, None);
        assert_eq!(result.limit_tokens, None);
    }

    #[test]
    fn extract_anthropic_headers() {
        let headers = {
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                "anthropic-ratelimit-requests-remaining",
                "15".parse().unwrap(),
            );
            h.insert("anthropic-ratelimit-requests-limit", "50".parse().unwrap());
            h.insert(
                "anthropic-ratelimit-tokens-remaining",
                "8000".parse().unwrap(),
            );
            h.insert("anthropic-ratelimit-tokens-limit", "16000".parse().unwrap());
            h
        };

        let result = QuotaHeaders::extract("anthropic", &headers).unwrap();
        assert_eq!(result.remaining_requests, Some(15));
        assert_eq!(result.limit_requests, Some(50));
        assert_eq!(result.remaining_tokens, Some(8000));
        assert_eq!(result.limit_tokens, Some(16000));
    }

    #[test]
    fn extract_no_headers_returns_none() {
        let headers = reqwest::header::HeaderMap::new();
        let result = QuotaHeaders::extract("openai", &headers);
        assert!(result.is_none());
    }

    #[test]
    fn extract_malformed_values_returns_none_for_field() {
        let headers = make_header("x-ratelimit-remaining-requests", "not-a-number");
        let result = QuotaHeaders::extract("openai", &headers);
        // The malformed field should produce None, and since it's the only field,
        // the overall result should be None (nothing valid found)
        assert!(result.is_none());
    }

    #[test]
    fn extract_partial_headers_returns_some() {
        let headers = make_header("x-ratelimit-limit-tokens", "4096");
        let result = QuotaHeaders::extract("openai", &headers).unwrap();
        assert_eq!(result.remaining_requests, None);
        assert_eq!(result.limit_requests, None);
        assert_eq!(result.remaining_tokens, None);
        assert_eq!(result.limit_tokens, Some(4096));
    }
}
