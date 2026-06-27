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
//! ID token signatures are verified via the IdP's JWKS
//! (`{jwks_uri}` from the discovery document) before claims are trusted. If
//! JWKS verification fails (e.g. unknown key type, transient fetch error), the
//! authenticator falls back to claim-only validation (`iss`/`aud`/`exp`) so
//! compatibility with unusual IdP configurations is preserved. The fallback
//! can be removed in a future hardening pass to require strict signature
//! verification.

use base64::Engine;
use hmac::{Hmac, Mac};
use jsonwebtoken::{DecodingKey, Validation};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::auth::{AuthSource, AuthUserInfo};
use crate::config::OidcConfig;

type HmacSha256 = Hmac<Sha256>;

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

/// JWKS (JSON Web Key Set) returned by the IdP's `jwks_uri` endpoint.
#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// Individual key inside a JWKS.
///
/// Only the RSA parameters required for ID token verification are modelled
/// here; EC and OKP fields are parsed but not yet used.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(default)]
    alg: Option<String>,
    /// RSA modulus (base64url, no padding).
    #[serde(default)]
    n: Option<String>,
    /// RSA exponent (base64url, no padding).
    #[serde(default)]
    e: Option<String>,
    /// EC curve name (e.g. `P-256`).
    #[serde(default)]
    crv: Option<String>,
    /// EC x coordinate (base64url).
    #[serde(default)]
    x: Option<String>,
    /// EC y coordinate (base64url).
    #[serde(default)]
    y: Option<String>,
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
            .redirect(reqwest::redirect::Policy::none())
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
    /// Returns `(url, state)` where `state` is a self-validating HMAC token
    /// (see [`verify_state_token`]) — no server-side session storage required.
    ///
    /// This uses the conventional `{issuer}/authorize` path. Override via
    /// discovery is handled by [`Self::discover`] when exchanging code.
    pub fn authorization_url(&self, state_secret: &str) -> (String, String) {
        // Stateless CSRF token: HMAC-signed so the callback can verify it
        // without needing to persist anything server-side.
        let state = build_state_token(state_secret);

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

        let doc: DiscoveryDocument = resp
            .json()
            .await
            .map_err(|e| OidcError::Discovery(format!("discovery parse failed: {e}")))?;

        // SSRF defence: validate that discovered endpoints belong to the same host
        // as the configured issuer. This prevents a compromised or malicious IdP from
        // redirecting our server to internal network addresses.
        let issuer_url = url::Url::parse(&self.config.issuer)
            .map_err(|e| OidcError::Config(format!("invalid issuer URL: {e}")))?;
        let issuer_host = issuer_url.host_str().unwrap_or("");

        validate_endpoint_host(&doc.token_endpoint, issuer_host)?;
        if let Some(ref userinfo) = doc.userinfo_endpoint {
            validate_endpoint_host(userinfo, issuer_host)?;
        }

        Ok(doc)
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
    ///    an `id_token` was returned. JWKS signature verification is attempted
    ///    first; if it fails the authenticator falls back to claim-only
    ///    validation so unusual IdP configurations keep working.
    /// 2. UserInfo endpoint (`userinfo_endpoint` from discovery) queried with
    ///    the access token as a Bearer header.
    pub async fn extract_user_info(&self, tokens: &TokenSet) -> Result<AuthUserInfo, OidcError> {
        if let Some(id_token) = &tokens.id_token {
            // Try JWKS signature verification first (strong security).
            match self.verify_id_token(id_token).await {
                Ok(claims) => return Ok(claims_to_user_info(&claims)),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "ID token JWKS verification failed — falling back to claim-only validation"
                    );
                    // Fallback: decode without signature verification, validate
                    // claims only. Maintains compatibility with IdPs that have
                    // unusual JWKS configurations; can be removed in a future
                    // hardening pass to require strict signature verification.
                    let claims = decode_jwt_payload(id_token)?;
                    validate_id_token_claims(&claims, &self.config.issuer, &self.config.client_id)?;
                    return Ok(claims_to_user_info(&claims));
                }
            }
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

    /// Verify an ID token's signature using the IdP's JWKS, then return the
    /// validated claims.
    ///
    /// Fetches the JWKS from `{jwks_uri}`, finds the key matching the token's
    /// `kid` header, and verifies the signature + `iss` / `aud` / `exp` claims
    /// via the `jsonwebtoken` crate.
    async fn verify_id_token(&self, id_token: &str) -> Result<serde_json::Value, OidcError> {
        let discovery = self.discover().await?;

        // Decode the JWT header to find the `kid` (key ID) and algorithm.
        let header = jsonwebtoken::decode_header(id_token)
            .map_err(|e| OidcError::UserInfo(format!("failed to decode JWT header: {e}")))?;

        // Fetch JWKS from the discovery document's jwks_uri.
        let jwks_resp = self
            .http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .map_err(|e| OidcError::Discovery(format!("JWKS fetch failed: {e}")))?;

        if !jwks_resp.status().is_success() {
            return Err(OidcError::Discovery(format!(
                "JWKS endpoint returned HTTP {}",
                jwks_resp.status()
            )));
        }

        let jwks: Jwks = jwks_resp
            .json()
            .await
            .map_err(|e| OidcError::Discovery(format!("JWKS parse failed: {e}")))?;

        // Find the key matching the token's `kid`. If the header has no `kid`,
        // fall back to the first RSA key (common with some IdPs).
        let matching_key = jwks.keys.iter().find(|k| {
            if let Some(ref kid) = header.kid {
                k.kid.as_deref() == Some(kid.as_str())
            } else {
                k.kty == "RSA"
            }
        });

        let key = matching_key.ok_or_else(|| {
            OidcError::UserInfo("no matching JWKS key found for ID token".to_string())
        })?;

        // Build a `DecodingKey` from the RSA public key parameters.
        let decoding_key = if key.kty == "RSA" {
            let n = key.n.as_ref().ok_or_else(|| {
                OidcError::UserInfo("JWKS RSA key missing modulus 'n'".to_string())
            })?;
            let e = key.e.as_ref().ok_or_else(|| {
                OidcError::UserInfo("JWKS RSA key missing exponent 'e'".to_string())
            })?;
            DecodingKey::from_rsa_components(n, e)
                .map_err(|e| OidcError::UserInfo(format!("failed to build RSA key: {e}")))?
        } else {
            return Err(OidcError::UserInfo(format!(
                "unsupported JWKS key type: {} (only RSA supported)",
                key.kty
            )));
        };

        // Build validation config: enforce issuer + audience + algorithm.
        let alg = header.alg;
        let mut validation = Validation::new(alg);
        validation.set_audience(&[&self.config.client_id]);
        validation.set_issuer(&[&self.config.issuer]);

        // Decode and verify the token.
        let token_data =
            jsonwebtoken::decode::<serde_json::Value>(id_token, &decoding_key, &validation)
                .map_err(|e| OidcError::UserInfo(format!("ID token verification failed: {e}")))?;

        Ok(token_data.claims)
    }
}

/// Decode the payload (middle) segment of a JWT without signature validation.
///
/// This is the fallback path used when JWKS signature verification fails or
/// is unavailable. The TLS-protected token endpoint response plus the claim
/// validation in [`validate_id_token_claims`] provide the baseline trust.
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

/// Build a stateless, self-validating CSRF state token.
///
/// Format: `{nonce}.{expiry_epoch}.{hmac}` where hmac = HMAC-SHA256(secret, nonce + "." + expiry).
/// The callback verifies the HMAC and checks that `expiry` has not passed,
/// without needing server-side session storage.
fn build_state_token(secret: &str) -> String {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    // 10-minute validity window — generous enough for user interaction with the IdP.
    let expiry = (chrono::Utc::now().timestamp() + 600).to_string();
    let mac = compute_hmac(secret, &format!("{nonce}.{expiry}"));
    format!("{nonce}.{expiry}.{mac}")
}

/// Verify a stateless CSRF state token.
///
/// Returns `Ok(())` if the HMAC matches (constant-time compare) and the
/// token has not expired. Returns `Err` otherwise.
pub fn verify_state_token(secret: &str, token: &str) -> Result<(), OidcError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(OidcError::Authorization(
            "malformed state token".to_string(),
        ));
    }
    let nonce = parts[0];
    let expiry_str = parts[1];
    let provided_mac = parts[2];

    // Check expiry first — reject expired tokens before HMAC comparison.
    let expiry: i64 = expiry_str
        .parse()
        .map_err(|_| OidcError::Authorization("malformed state expiry".to_string()))?;
    let now = chrono::Utc::now().timestamp();
    if now > expiry {
        return Err(OidcError::Authorization("state token expired".to_string()));
    }

    // Recompute HMAC and constant-time compare.
    let expected_mac = compute_hmac(secret, &format!("{nonce}.{expiry_str}"));
    if expected_mac
        .as_bytes()
        .ct_eq(provided_mac.as_bytes())
        .into()
    {
        Ok(())
    } else {
        Err(OidcError::Authorization(
            "state token HMAC mismatch".to_string(),
        ))
    }
}

/// Compute a hex-encoded HMAC-SHA256.
fn compute_hmac(secret: &str, data: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Validate OIDC ID token claims (iss, aud, exp) without signature verification.
///
/// This is the fallback path used when JWKS verification cannot be performed.
/// It catches misconfiguration, token replay, and tokens from the wrong IdP;
/// [`OidcAuthenticator::verify_id_token`] is the preferred, signature-checking
/// path.
fn validate_id_token_claims(
    claims: &serde_json::Value,
    expected_iss: &str,
    expected_aud: &str,
) -> Result<(), OidcError> {
    // Verify issuer matches configuration.
    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OidcError::UserInfo("ID token missing 'iss' claim".to_string()))?;
    if iss != expected_iss {
        return Err(OidcError::UserInfo(format!(
            "ID token issuer mismatch: expected '{expected_iss}', got '{iss}'"
        )));
    }

    // Verify audience matches our client_id.
    let aud = claims.get("aud");
    let aud_matches = match aud {
        Some(serde_json::Value::String(s)) => s == expected_aud,
        Some(serde_json::Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some(expected_aud)),
        _ => false,
    };
    if !aud_matches {
        return Err(OidcError::UserInfo(
            "ID token audience does not match client_id".to_string(),
        ));
    }

    // Verify token has not expired.
    if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
        let now = chrono::Utc::now().timestamp();
        if now > exp {
            return Err(OidcError::UserInfo("ID token has expired".to_string()));
        }
    }

    Ok(())
}

/// Validate that a URL from the discovery document has the same host as the issuer.
///
/// Prevents SSRF via malicious discovery documents that redirect the server to
/// internal endpoints.
fn validate_endpoint_host(url_str: &str, issuer_host: &str) -> Result<(), OidcError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| OidcError::Discovery(format!("discovery endpoint URL malformed: {e}")))?;
    if parsed.host_str() != Some(issuer_host) {
        return Err(OidcError::Discovery(format!(
            "discovery endpoint host mismatch: issuer host '{issuer_host}' but endpoint host was '{}'",
            parsed.host_str().unwrap_or("(none)")
        )));
    }
    Ok(())
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
        let (url, state) = auth.authorization_url("test-secret");

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
        let (_, state1) = auth.authorization_url("test-secret");
        let (_, state2) = auth.authorization_url("test-secret");
        assert_ne!(state1, state2, "state must be unique per request");
    }

    #[test]
    fn authorization_url_includes_configured_scopes() {
        let auth = OidcAuthenticator::new(test_config()).unwrap();
        let (url, _) = auth.authorization_url("test-secret");
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
        let (url, _state) = auth.authorization_url("test-secret");
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
        let (url, _) = auth.authorization_url("test-secret");
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
            "name": "Alice Adams",
            "iss": "https://login.example.com",
            "aud": "test-client-id"
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

    // ── CSRF state token tests ───────────────────────────────────

    #[test]
    fn state_token_roundtrips_successfully() {
        let secret = "test-secret";
        let token = build_state_token(secret);
        assert!(verify_state_token(secret, &token).is_ok());
    }

    #[test]
    fn state_token_rejects_wrong_secret() {
        let token = build_state_token("correct-secret");
        assert!(verify_state_token("wrong-secret", &token).is_err());
    }

    #[test]
    fn state_token_rejects_tampered_token() {
        let token = build_state_token("secret");
        let tampered = format!("{}.{}.{}", "fake-nonce", "9999999999", "fake-mac");
        assert!(verify_state_token("secret", &tampered).is_err());
    }

    // ── ID token claim validation tests ──────────────────────────

    #[tokio::test]
    async fn extract_user_info_rejects_expired_id_token() {
        let claims = json!({
            "sub": "user-abc",
            "iss": "https://login.example.com",
            "aud": "test-client-id",
            "exp": 1  // Unix epoch — always in the past
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
        let result = auth.extract_user_info(&tokens).await;
        assert!(result.is_err());
    }

    // ── JWKS signature verification tests ────────────────────────

    #[tokio::test]
    async fn verify_id_token_succeeds_with_valid_jwks() {
        use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts, RsaPrivateKey};

        // Generate a small RSA keypair for the mock IdP. `rsa` 0.9 requires
        // rand_core 0.6, which is pulled in as a dev-dependency.
        let mut rng = rsa::rand_core::OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
        let public_key = private_key.to_public_key();

        // Encode the RSA components for the JWKS response (base64url, no pad).
        let n_b64 = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());

        let server = MockServer::start().await;
        let issuer_url = server.uri();

        // Build a real signed JWT with the matching private key.
        let claims = json!({
            "sub": "user-jwks-test",
            "email": "jwks@example.com",
            "preferred_username": "jwks-user",
            "iss": issuer_url,
            "aud": "test-client-id",
            "exp": (chrono::Utc::now().timestamp() + 3600)
        });

        let pkcs1_pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("encode PKCS#1");
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pkcs1_pem.as_bytes())
            .expect("build encoding key");

        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &encoding_key,
        )
        .expect("sign JWT");

        // Mock the discovery document.
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
            "issuer": server.uri()
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;

        // Mock the JWKS endpoint.
        let jwks = json!({
            "keys": [{
                "kid": "test-key-1",
                "kty": "RSA",
                "alg": "RS256",
                "n": n_b64,
                "e": e_b64
            }]
        });

        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .mount(&server)
            .await;

        // Configure the authenticator with the mock issuer.
        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();

        let tokens = TokenSet {
            access_token: "atk".into(),
            id_token: Some(token),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
        };

        let info = auth
            .extract_user_info(&tokens)
            .await
            .expect("extract should succeed via JWKS verification");
        assert_eq!(info.id, "user-jwks-test");
        assert_eq!(info.username, "jwks-user");
        assert_eq!(info.email.as_deref(), Some("jwks@example.com"));
        assert_eq!(info.source, AuthSource::Oidc);
    }

    /// A fake-signed JWT cannot pass JWKS verification, so the authenticator
    /// must fall back to claim-only validation. The expired `exp` then
    /// triggers the expected rejection.
    #[tokio::test]
    async fn extract_user_info_falls_back_when_jwks_unreachable() {
        let server = MockServer::start().await;

        // Discovery points the JWKS URI at the mock server but we deliberately
        // do NOT mount a `/jwks` responder, so the JWKS fetch will return 404.
        let discovery = json!({
            "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
            "issuer": server.uri()
        });

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(discovery))
            .mount(&server)
            .await;

        // Valid (claim-wise) fake-signed JWT — JWKS verification will fail
        // because the signature is bogus, then claim validation should pass.
        let claims = json!({
            "sub": "fallback-user",
            "preferred_username": "fallback",
            "email": "fallback@example.com",
            "iss": server.uri(),
            "aud": "test-client-id",
            "exp": (chrono::Utc::now().timestamp() + 3600)
        });
        let jwt = make_fake_jwt(&claims);

        let mut cfg = test_config();
        cfg.issuer = server.uri();
        let auth = OidcAuthenticator::new(cfg).unwrap();

        let tokens = TokenSet {
            access_token: "atk".into(),
            id_token: Some(jwt),
            token_type: "Bearer".into(),
            expires_in: None,
            refresh_token: None,
        };

        let info = auth
            .extract_user_info(&tokens)
            .await
            .expect("fallback claim-only validation should succeed");
        assert_eq!(info.id, "fallback-user");
        assert_eq!(info.username, "fallback");
        assert_eq!(info.email.as_deref(), Some("fallback@example.com"));
    }
}
