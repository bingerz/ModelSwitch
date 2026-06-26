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
}

/// OIDC authenticator handling the authorization code flow.
///
/// **Scaffold**: only the authorization URL builder is implemented.
/// Token exchange, ID token validation, and userinfo retrieval will be
/// added in a subsequent phase.
pub struct OidcAuthenticator {
    config: OidcConfig,
}

impl OidcAuthenticator {
    /// Create a new OIDC authenticator with the given configuration.
    pub fn new(config: OidcConfig) -> Self {
        Self { config }
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
            // Scopes are space-separated in the URL.
            // Each scope value is URL-encoded individually.
            &self
                .config
                .scopes
                .iter()
                .map(|s| urlencoding::encode(s).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
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
        let auth = OidcAuthenticator::new(test_config());
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
        let auth = OidcAuthenticator::new(test_config());
        let (_, state1) = auth.authorization_url();
        let (_, state2) = auth.authorization_url();
        assert_ne!(state1, state2, "state must be unique per request");
    }

    #[test]
    fn authorization_url_includes_configured_scopes() {
        let auth = OidcAuthenticator::new(test_config());
        let (url, _) = auth.authorization_url();
        // Scopes are URL-encoded in the query string.
        assert!(url.contains("openid"));
        assert!(url.contains("email"));
        assert!(url.contains("profile"));
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
        let auth = OidcAuthenticator::new(cfg);
        let (url, _) = auth.authorization_url();
        // Should not have a double slash before "authorize".
        assert!(url.starts_with("https://login.example.com/authorize?"));
        assert!(!url.contains("//authorize"));
    }

    #[tokio::test]
    async fn exchange_code_returns_not_implemented() {
        let auth = OidcAuthenticator::new(test_config());
        let result = auth.exchange_code("fake-code", "fake-state").await;
        assert!(result.is_err());
    }
}
