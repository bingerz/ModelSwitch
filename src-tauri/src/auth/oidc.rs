//! OIDC / OAuth2 SSO authentication (scaffold).
//!
//! This module provides the authorization URL builder for the OAuth2
//! authorization code flow. Token validation and userinfo retrieval
//! are not yet implemented.

use crate::config::OidcConfig;

/// OIDC authentication errors.
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    /// The authorization server returned an error.
    #[error("OIDC authorization error: {0}")]
    Authorization(String),
    /// Token exchange failed.
    #[error("OIDC token exchange error: {0}")]
    TokenExchange(String),
    /// Userinfo retrieval failed.
    #[error("OIDC userinfo error: {0}")]
    UserInfo(String),
    /// Configuration error.
    #[error("OIDC configuration error: {0}")]
    Config(String),
    /// Invalid issuer URL (wrong scheme, malformed, etc.).
    #[error("Invalid OIDC issuer: {0}")]
    InvalidIssuer(String),
}

/// OIDC authenticator handling the authorization code flow.
///
/// **Scaffold**: only the authorization URL builder is implemented.
/// Token exchange, ID token validation, and userinfo retrieval will be
/// added in a subsequent phase.
#[derive(Debug)]
pub struct OidcAuthenticator {
    config: OidcConfig,
}

impl OidcAuthenticator {
    /// Create a new OIDC authenticator with the given configuration.
    ///
    /// Validates that the issuer URL uses `https://` (or `http://` for
    /// localhost development) to prevent SSRF and credential leakage.
    pub fn new(config: OidcConfig) -> Result<Self, OidcError> {
        let parsed = url::Url::parse(&config.issuer)
            .map_err(|e| OidcError::InvalidIssuer(format!("malformed issuer URL: {e}")))?;

        let is_localhost = matches!(
            parsed.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("::1")
        );
        if parsed.scheme() != "https" && !is_localhost {
            return Err(OidcError::InvalidIssuer(
                "OIDC issuer must use https:// scheme (or http:// for localhost development)"
                    .to_string(),
            ));
        }

        Ok(Self { config })
    }

    /// Build the authorization URL to redirect the user to.
    ///
    /// Constructs the authorization endpoint URL with the required
    /// `client_id`, `redirect_uri`, `response_type=code`, `scope`,
    /// and a random `state` parameter for CSRF protection.
    ///
    /// Returns `(url, state)` where `state` should be stored (e.g., in a
    /// session or cookie) and verified when the callback is received.
    pub fn authorization_url(&self) -> (String, String) {
        // Generate a random state parameter for CSRF protection.
        let state = uuid::Uuid::new_v4().simple().to_string();

        let scopes = if self.config.scopes.is_empty() {
            "openid"
        } else {
            // Scopes are space-separated and encoded once at the query-string level
            // (the `urlencoding::encode(v)` call below handles all encoding).
            &self.config.scopes.join(" ")
        };

        let params: Vec<(String, String)> = vec![
            ("response_type".to_string(), "code".to_string()),
            ("client_id".to_string(), self.config.client_id.clone()),
            ("redirect_uri".to_string(), self.config.redirect_uri.clone()),
            ("scope".to_string(), scopes.to_string()),
            ("state".to_string(), state.clone()),
        ];

        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        // The authorization endpoint is derived from the issuer URL.
        // Standard OIDC: {issuer}/authorize (discovery can override this,
        // but for the scaffold we use the conventional path).
        let auth_endpoint = format!("{}/authorize", self.config.issuer.trim_end_matches('/'));

        let url = format!("{auth_endpoint}?{query}");

        (url, state)
    }

    /// Exchange an authorization code for tokens.
    ///
    /// **Not yet implemented.** This will perform the token endpoint
    /// POST request and validate the returned ID token.
    pub async fn exchange_code(&self, _code: &str, _state: &str) -> Result<(), OidcError> {
        Err(OidcError::TokenExchange(
            "Not yet implemented — OIDC token exchange is a scaffold".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> OidcConfig {
        OidcConfig {
            issuer: "https://login.example.com".into(),
            client_id: "test-client-id".into(),
            client_secret: Some("test-secret".into()),
            redirect_uri: "http://localhost:3000/api/auth/oidc/callback".into(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
        }
    }

    #[test]
    fn authorization_url_contains_required_params() {
        let auth = OidcAuthenticator::new(test_config()).unwrap();
        let (url, state) = auth.authorization_url();

        assert!(url.starts_with("https://login.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope="));
        assert!(url.contains("state="));
        assert!(!state.is_empty());
    }

    #[test]
    fn authorization_url_has_unique_state() {
        let auth = OidcAuthenticator::new(test_config()).unwrap();
        let (_, state1) = auth.authorization_url();
        let (_, state2) = auth.authorization_url();
        assert_ne!(state1, state2, "state must be unique per request");
    }

    #[test]
    fn authorization_url_includes_configured_scopes() {
        let auth = OidcAuthenticator::new(test_config()).unwrap();
        let (url, _) = auth.authorization_url();
        // Scopes are URL-encoded in the query string.
        assert!(url.contains("openid"));
        assert!(url.contains("email"));
        assert!(url.contains("profile"));
    }

    #[test]
    fn authorization_url_does_not_double_encode_scopes() {
        let cfg = OidcConfig {
            issuer: "https://idp.example.com".to_string(),
            client_id: "test-client".to_string(),
            client_secret: None,
            redirect_uri: "https://app.example.com/callback".to_string(),
            scopes: vec!["openid".to_string(), "custom:read-write".to_string()],
        };
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let (url, _state) = auth.authorization_url();
        // The colon in "custom:read-write" should be encoded once (as %3A),
        // not double-encoded to %253A.
        assert!(
            !url.contains("%253A"),
            "Scope value was double-encoded: {url}"
        );
        assert!(
            url.contains("custom%3Aread-write") || url.contains("custom:read-write"),
            "Expected scope to appear in URL: {url}"
        );
    }

    #[test]
    fn authorization_url_strips_trailing_slash_from_issuer() {
        let cfg = OidcConfig {
            issuer: "https://login.example.com/".into(),
            client_id: "test".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            scopes: vec!["openid".into()],
        };
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let (url, _) = auth.authorization_url();
        // Should not have a double slash before "authorize".
        assert!(url.starts_with("https://login.example.com/authorize?"));
        assert!(!url.contains("//authorize"));
    }

    #[tokio::test]
    async fn exchange_code_returns_not_implemented() {
        let auth = OidcAuthenticator::new(test_config()).unwrap();
        let result = auth.exchange_code("fake-code", "fake-state").await;
        assert!(result.is_err());
    }

    #[test]
    fn rejects_http_issuer_for_non_localhost() {
        let cfg = OidcConfig {
            issuer: "http://login.example.com".into(),
            client_id: "test".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            scopes: vec!["openid".into()],
        };
        let result = OidcAuthenticator::new(cfg);
        assert!(result.is_err());
        match result.unwrap_err() {
            OidcError::InvalidIssuer(msg) => assert!(msg.contains("https://")),
            other => panic!("expected InvalidIssuer error, got: {other}"),
        }
    }

    #[test]
    fn accepts_https_issuer() {
        let cfg = OidcConfig {
            issuer: "https://login.microsoftonline.com/tenant/v2.0".into(),
            client_id: "test".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            scopes: vec!["openid".into()],
        };
        let result = OidcAuthenticator::new(cfg);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_localhost_http_for_dev() {
        let cfg = OidcConfig {
            issuer: "http://localhost:8080".into(),
            client_id: "test".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            scopes: vec!["openid".into()],
        };
        let result = OidcAuthenticator::new(cfg);
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_malformed_issuer_url() {
        let cfg = OidcConfig {
            issuer: "not a valid url".into(),
            client_id: "test".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            scopes: vec!["openid".into()],
        };
        let result = OidcAuthenticator::new(cfg);
        assert!(result.is_err());
    }
}
