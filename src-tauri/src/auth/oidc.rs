//! OIDC / OAuth2 SSO authentication (authorization code flow).
//!
//! This module implements the OAuth2 authorization code flow:
//! 1. [`OidcAuthenticator::authorization_url`] builds the IdP authorization URL.
//! 2. [`OidcAuthenticator::exchange_code`] exchanges the redirect `code` for a
//!    [`TokenSet`] via the token endpoint discovered from the issuer's
//!    `.well-known/openid-configuration` document.
//! 3. [`OidcAuthenticator::extract_user_info`] derives an [`AuthUserInfo`] from
//!    the returned ID token (JWT claims decoded locally) or, when no ID token
//!    is present, by calling the IdP's userinfo endpoint with the access token.
//!
//! Signature validation of the ID token is intentionally deferred — the
//! gateway relies on the TLS-protected token endpoint response and the
//! single-use authorization code binding. Production deployments that need
//! defence-in-depth should add JWKS signature verification.

use base64::Engine;
use serde::Deserialize;

use crate::auth::{AuthSource, AuthUserInfo};
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
    /// Discovery document fetch or parse failure.
    #[error("OIDC discovery error: {0}")]
    Discovery(String),
}

/// Token set returned by a successful OAuth2 token exchange.
#[derive(Debug, Clone)]
pub struct TokenSet {
    /// Bearer access token used to call resource / userinfo endpoints.
    pub access_token: String,
    /// OIDC ID token (JWT) when the `openid` scope was granted.
    pub id_token: Option<String>,
    /// Token type, almost always `Bearer`.
    pub token_type: String,
    /// Access token lifetime in seconds.
    pub expires_in: Option<u64>,
    /// Refresh token, when the `offline_access` scope was granted.
    pub refresh_token: Option<String>,
}

/// Discovery document returned by `{issuer}/.well-known/openid-configuration`.
///
/// Only the fields used by this implementation are deserialised; unknown
/// fields are ignored so provider-specific extensions do not break discovery.
#[allow(dead_code)] // fields are asserted in tests; production reads token_/userinfo_endpoint
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscoveryDocument {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    /// Optional userinfo endpoint — some providers omit it.
    #[serde(default)]
    userinfo_endpoint: Option<String>,
}

/// Raw token endpoint response. Fields beyond the core OAuth2 set are ignored.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// OIDC authenticator handling the authorization code flow.
pub struct OidcAuthenticator {
    config: OidcConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for OidcAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcAuthenticator")
            .field("issuer", &self.config.issuer)
            .field("client_id", &self.config.client_id)
            .finish()
    }
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

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| OidcError::Config(format!("failed to build HTTP client: {e}")))?;

        Ok(Self { config, http })
    }

    /// Build the authorization URL to redirect the user to.
    ///
    /// Constructs the authorization endpoint URL with the required
    /// `client_id`, `redirect_uri`, `response_type=code`, `scope`,
    /// and a random `state` parameter for CSRF protection.
    ///
    /// Returns `(url, state)` where `state` should be stored (e.g., in a
    /// session or cookie) and verified when the callback is received.
    ///
    /// This uses the conventional `{issuer}/authorize` path. Override via
    /// discovery is handled by [`Self::discover`] when exchanging code.
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
        // Standard OIDC: {issuer}/authorize. Discovery can override this
        // for the callback flow; the URL builder keeps the conventional path
        // so it works without a network round-trip.
        let auth_endpoint = format!("{}/authorize", self.config.issuer.trim_end_matches('/'));

        let url = format!("{auth_endpoint}?{query}");

        (url, state)
    }

    /// Fetch the OIDC discovery document from
    /// `{issuer}/.well-known/openid-configuration`.
    ///
    /// Used to resolve the token and userinfo endpoints dynamically instead
    /// of hard-coding IdP-specific paths.
    pub(crate) async fn discover(&self) -> Result<DiscoveryDocument, OidcError> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| OidcError::Discovery(format!("discovery request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(OidcError::Discovery(format!(
                "discovery returned HTTP {}",
                resp.status()
            )));
        }

        resp.json::<DiscoveryDocument>()
            .await
            .map_err(|e| OidcError::Discovery(format!("discovery parse failed: {e}")))
    }

    /// Exchange an authorization code for a [`TokenSet`].
    ///
    /// Performs the standard OAuth2 token endpoint POST with
    /// `grant_type=authorization_code`. Discovery is run first to resolve the
    /// correct token endpoint URI.
    pub async fn exchange_code(&self, code: &str) -> Result<TokenSet, OidcError> {
        let discovery = self.discover().await?;

        let mut form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), self.config.redirect_uri.clone()),
            ("client_id".to_string(), self.config.client_id.clone()),
        ];
        if let Some(secret) = &self.config.client_secret {
            form.push(("client_secret".to_string(), secret.clone()));
        }

        let resp = self
            .http
            .post(&discovery.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| OidcError::TokenExchange(format!("token endpoint request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            // Best-effort error body extraction; fall back to status code.
            let body = resp.text().await.unwrap_or_default();
            return Err(OidcError::TokenExchange(format!(
                "token endpoint returned HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            )));
        }

        let token_resp = resp.json::<TokenResponse>().await.map_err(|e| {
            OidcError::TokenExchange(format!("token endpoint response parse failed: {e}"))
        })?;

        Ok(TokenSet {
            access_token: token_resp.access_token,
            id_token: token_resp.id_token,
            token_type: token_resp
                .token_type
                .unwrap_or_else(|| "Bearer".to_string()),
            expires_in: token_resp.expires_in,
            refresh_token: token_resp.refresh_token,
        })
    }

    /// Derive [`AuthUserInfo`] from the token set.
    ///
    /// Preference order:
    /// 1. ID token claims (`sub`, `email`, `name` / `preferred_username`) when
    ///    an `id_token` was returned.
    /// 2. UserInfo endpoint (`userinfo_endpoint` from discovery) queried with
    ///    the access token as a Bearer header.
    pub async fn extract_user_info(&self, tokens: &TokenSet) -> Result<AuthUserInfo, OidcError> {
        if let Some(id_token) = &tokens.id_token {
            let claims = decode_jwt_payload(id_token)?;
            return Ok(claims_to_user_info(&claims));
        }

        // No ID token — fall back to userinfo endpoint.
        let discovery = self.discover().await?;
        let userinfo_url = match discovery.userinfo_endpoint.as_deref() {
            Some(url) if !url.is_empty() => url.to_string(),
            _ => {
                return Err(OidcError::UserInfo(
                    "no ID token present and IdP discovery omitted userinfo_endpoint".to_string(),
                ));
            }
        };

        let resp = self
            .http
            .get(&userinfo_url)
            .bearer_auth(&tokens.access_token)
            .send()
            .await
            .map_err(|e| OidcError::UserInfo(format!("userinfo request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(OidcError::UserInfo(format!(
                "userinfo endpoint returned HTTP {}",
                resp.status()
            )));
        }

        let claims: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| OidcError::UserInfo(format!("userinfo parse failed: {e}")))?;

        Ok(claims_to_user_info(&claims))
    }
}

/// Decode the payload (middle) segment of a JWT without signature validation.
///
/// OIDC ID tokens are JWS (signed) JWTs. For user-info extraction we only
/// need the claims — we trust the TLS-protected token endpoint response and
/// the single-use authorization code binding. Signature verification via JWKS
/// is a deferred hardening step.
fn decode_jwt_payload(token: &str) -> Result<serde_json::Value, OidcError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err(OidcError::UserInfo("malformed JWT".to_string()));
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| OidcError::UserInfo(format!("JWT payload decode error: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| OidcError::UserInfo(format!("JWT payload parse error: {e}")))
}

/// Map a JWT/userinfo JSON claim set to [`AuthUserInfo`].
fn claims_to_user_info(claims: &serde_json::Value) -> AuthUserInfo {
    let id = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let username = claims
        .get("preferred_username")
        .and_then(|v| v.as_str())
        .or_else(|| claims.get("name").and_then(|v| v.as_str()))
        .or_else(|| claims.get("sub").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.clone());
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    AuthUserInfo {
        id,
        username,
        email,
        groups: Vec::new(),
        source: AuthSource::Oidc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config() -> OidcConfig {
        OidcConfig {
            issuer: "https://login.example.com".into(),
            client_id: "test-client-id".into(),
            client_secret: Some("test-secret".into()),
            redirect_uri: "http://localhost:3000/api/auth/oidc/callback".into(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
        }
    }

    /// Build a (fake-signed) JWT with the given claims.
    fn make_fake_jwt(claims: &serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"RS256\",\"typ\":\"JWT\"}");
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap());
        let signature = URL_SAFE_NO_PAD.encode(b"fake-signature");
        format!("{header}.{payload}.{signature}")
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

    // ── New tests for the full flow ───────────────────────────────

    #[test]
    fn decode_jwt_payload_extracts_claims() {
        let claims = json!({
            "sub": "user-123",
            "email": "alice@example.com",
            "name": "Alice Adams",
            "preferred_username": "alice"
        });
        let jwt = make_fake_jwt(&claims);

        let decoded = decode_jwt_payload(&jwt).expect("decode should succeed");
        assert_eq!(decoded["sub"], "user-123");
        assert_eq!(decoded["email"], "alice@example.com");
        assert_eq!(decoded["preferred_username"], "alice");
    }

    #[test]
    fn decode_jwt_payload_rejects_malformed_token() {
        let result = decode_jwt_payload("not-a-jwt");
        assert!(result.is_err());
    }

    #[test]
    fn claims_to_user_info_prefers_preferred_username() {
        let claims = json!({
            "sub": "sub-1",
            "preferred_username": "alice",
            "name": "Alice Adams",
            "email": "alice@example.com"
        });
        let info = claims_to_user_info(&claims);
        assert_eq!(info.id, "sub-1");
        assert_eq!(info.username, "alice");
        assert_eq!(info.email.as_deref(), Some("alice@example.com"));
        assert_eq!(info.source, AuthSource::Oidc);
        assert!(info.groups.is_empty());
    }

    #[test]
    fn claims_to_user_info_falls_back_to_name_then_sub() {
        let claims_no_pref = json!({
            "sub": "sub-2",
            "name": "Bob Builder",
            "email": "bob@example.com"
        });
        let info = claims_to_user_info(&claims_no_pref);
        assert_eq!(info.username, "Bob Builder");

        let claims_only_sub = json!({"sub": "sub-3"});
        let info = claims_to_user_info(&claims_only_sub);
        assert_eq!(info.username, "sub-3");
        assert!(info.email.is_none());
    }

    #[tokio::test]
    async fn discover_fetches_endpoints() {
        let server = MockServer::start().await;
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
            "userinfo_endpoint": format!("{}/userinfo", server.uri()),
            "issuer": server.uri()
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let doc = auth.discover().await.expect("discovery should succeed");
        assert!(doc.token_endpoint.ends_with("/token"));
        assert!(doc.jwks_uri.ends_with("/jwks"));
        assert!(doc
            .userinfo_endpoint
            .as_ref()
            .unwrap()
            .ends_with("/userinfo"));
    }

    #[tokio::test]
    async fn discover_returns_error_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let result = auth.discover().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            OidcError::Discovery(_) => {}
            other => panic!("expected Discovery error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn exchange_code_returns_error_on_invalid_code() {
        let server = MockServer::start().await;
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "the code is invalid"
            })))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let result = auth.exchange_code("invalid-code").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            OidcError::TokenExchange(_) => {}
            other => panic!("expected TokenExchange error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn exchange_code_succeeds_with_token_set() {
        let server = MockServer::start().await;
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "atk-123",
                "id_token": "header.eyJzdWIiOiJ1LTEifQ.sig",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token": "rtk-abc"
            })))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let tokens = auth.exchange_code("valid-code").await.expect("exchange ok");
        assert_eq!(tokens.access_token, "atk-123");
        assert_eq!(tokens.token_type, "Bearer");
        assert_eq!(tokens.expires_in, Some(3600));
        assert_eq!(tokens.refresh_token.as_deref(), Some("rtk-abc"));
        assert!(tokens.id_token.is_some());
    }

    #[tokio::test]
    async fn exchange_code_includes_client_secret_in_form() {
        let server = MockServer::start().await;
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(wiremock::matchers::body_string_contains(
                "client_secret=topsecret",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "atk",
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        cfg.client_secret = Some("topsecret".into());
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let result = auth.exchange_code("code").await;
        assert!(result.is_ok(), "expected secret-conditional mock to match");
    }

    #[tokio::test]
    async fn extract_user_info_from_id_token() {
        let claims = json!({
            "sub": "user-abc",
            "email": "alice@example.com",
            "preferred_username": "alice",
            "name": "Alice Adams"
        });
        let jwt = make_fake_jwt(&claims);

        let cfg = test_config();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let tokens = TokenSet {
            access_token: "atk".into(),
            id_token: Some(jwt),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
        };

        let info = auth.extract_user_info(&tokens).await.expect("extract ok");
        assert_eq!(info.id, "user-abc");
        assert_eq!(info.username, "alice");
        assert_eq!(info.email.as_deref(), Some("alice@example.com"));
        assert_eq!(info.source, AuthSource::Oidc);
    }

    #[tokio::test]
    async fn extract_user_info_uses_userinfo_endpoint() {
        let server = MockServer::start().await;
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
            "userinfo_endpoint": format!("{}/userinfo", server.uri()),
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "sub": "userinfo-sub",
                "email": "info@example.com",
                "name": "Info User"
            })))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let tokens = TokenSet {
            access_token: "atk".into(),
            id_token: None,
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
        };

        let info = auth.extract_user_info(&tokens).await.expect("extract ok");
        assert_eq!(info.id, "userinfo-sub");
        assert_eq!(info.username, "Info User");
        assert_eq!(info.email.as_deref(), Some("info@example.com"));
    }

    #[tokio::test]
    async fn extract_user_info_errors_without_id_token_or_userinfo_endpoint() {
        let server = MockServer::start().await;
        // Discovery document without userinfo_endpoint.
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri())
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();
        let tokens = TokenSet {
            access_token: "atk".into(),
            id_token: None,
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
        };

        let result = auth.extract_user_info(&tokens).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            OidcError::UserInfo(_) => {}
            other => panic!("expected UserInfo error, got: {other}"),
        }
    }
}
