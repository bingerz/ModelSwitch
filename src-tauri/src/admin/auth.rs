use crate::auth::ldap::LdapAuthenticator;
use crate::auth::oidc::OidcAuthenticator;
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

/// Response payload for the `/api/auth/me` endpoint.
#[derive(serde::Serialize)]
pub struct AuthMeResponse {
    pub role: String,
    pub authenticated: bool,
}

/// Returns the caller's role and authentication status.
///
/// After `admin_auth_middleware` runs, the role is injected into request
/// extensions. When no auth is configured (open-proxy mode), the role is
/// absent and `authenticated` is `false`.
pub async fn auth_me(req: axum::extract::Request) -> Json<super::ApiResponse<AuthMeResponse>> {
    let role = req
        .extensions()
        .get::<crate::middleware::rbac::Role>()
        .copied();

    let (role_str, authenticated) = match role {
        Some(crate::middleware::rbac::Role::SuperAdmin) => ("super_admin", true),
        Some(crate::middleware::rbac::Role::KeyManager) => ("key_manager", true),
        Some(crate::middleware::rbac::Role::Auditor) => ("auditor", true),
        None => ("none", false),
    };

    Json(super::ApiResponse::ok(AuthMeResponse {
        role: role_str.to_string(),
        authenticated,
    }))
}

/// Receive cookies from the WebView login flow.
/// The WebView injects JS that POSTs cookies here after login succeeds.
pub async fn receive_login_cookies(
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let cookies = body.get("cookies").and_then(|v| v.as_str()).unwrap_or("");

    if cookies.is_empty() {
        tracing::warn!(provider, "WebView login returned empty cookies");
        return ApiError::new(StatusCode::BAD_REQUEST, "Empty cookies");
    }

    tracing::info!(
        provider,
        cookie_len = cookies.len(),
        "Received login cookies from WebView"
    );

    // Store in a temporary file for the frontend to pick up
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch");
    let _ = std::fs::create_dir_all(&dir);
    let pending_file = dir.join("pending_cookies.json");

    let data = serde_json::json!({
        "provider": provider,
        "cookies": cookies,
        "received_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Err(e) = std::fs::write(&pending_file, data.to_string()) {
        tracing::error!("Failed to write pending cookies: {}", e);
        return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Failed to write cookies");
    }
    // Restrict file permissions to owner-only (sensitive session cookies)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&pending_file, std::fs::Permissions::from_mode(0o600));
    }

    StatusCode::OK.into_response()
}

/// Return the most recent pending cookies from WebView login (one-shot read).
pub async fn get_pending_cookies() -> Json<super::ApiResponse<Option<serde_json::Value>>> {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch");
    let pending_file = dir.join("pending_cookies.json");

    if !pending_file.exists() {
        return Json(super::ApiResponse::ok(None));
    }

    match std::fs::read_to_string(&pending_file) {
        Ok(content) => {
            // Delete after reading (one-shot)
            let _ = std::fs::remove_file(&pending_file);
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(v) => Json(super::ApiResponse::ok(Some(v))),
                Err(_) => Json(super::ApiResponse::ok(None)),
            }
        }
        Err(_) => Json(super::ApiResponse::ok(None)),
    }
}

// ─── LDAP Login ────────────────────────────────────────

/// Request body for LDAP login.
#[derive(Debug, serde::Deserialize)]
pub struct LdapLoginRequest {
    pub username: String,
    pub password: String,
}

/// Response body for successful LDAP login.
#[derive(Debug, serde::Serialize)]
pub struct LdapLoginResponse {
    pub key: String,
    pub key_prefix: String,
    pub username: String,
    pub group: String,
}

/// LDAP login endpoint.
///
/// Authenticates a user against the configured LDAP/AD server and provisions
/// a virtual API key for gateway access. Returns 503 if LDAP is not configured.
pub async fn ldap_login(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<LdapLoginRequest>,
) -> axum::response::Response {
    let ldap_config = match &state.ldap_config {
        Some(cfg) => cfg.clone(),
        None => {
            return ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "LDAP authentication is not configured",
            )
            .into_response();
        }
    };

    if req.username.is_empty() || req.password.is_empty() {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "Username and password are required",
        )
        .into_response();
    }

    let authenticator = LdapAuthenticator::new(ldap_config);
    match authenticator
        .authenticate(&req.username, &req.password)
        .await
    {
        Ok(user_info) => {
            let store = &state.billing.virtual_key_store;
            let key_name = format!("ldap:{}", user_info.username);

            // Dedupe: if a key for this user already exists, delete it so the
            // user gets a fresh plaintext on re-login. Without this, every
            // successful login accumulates a new key indefinitely.
            let existing = store.list().await.into_iter().find(|k| k.name == key_name);
            if let Some(old_key) = existing {
                store.delete(old_key.id).await;
            }

            let (new_key, plaintext) = store
                .create(
                    key_name,
                    None,   // daily_budget_cents
                    None,   // monthly_budget_cents
                    None,   // allowed_models
                    vec![], // denied_models
                    vec![], // allowed_ips
                    None,   // rpm_limit
                    None,   // tpm_limit
                    None,   // expires_at
                    Some(user_info.groups.first().cloned().unwrap_or_default()),
                )
                .await;

            tracing::info!(
                username = %user_info.username,
                key_prefix = %new_key.key_prefix,
                "LDAP login succeeded, virtual key provisioned"
            );

            let resp = LdapLoginResponse {
                key: plaintext,
                key_prefix: new_key.key_prefix,
                username: user_info.username,
                group: user_info.groups.first().cloned().unwrap_or_default(),
            };
            Json(super::ApiResponse::ok(resp)).into_response()
        }
        Err(e) => {
            tracing::warn!(username = %req.username, error = %e, "LDAP login failed");
            // Return generic messages to the client — never leak internal
            // error details (LDAP URLs, connection diagnostics, etc.) to
            // unauthenticated callers. The full error is logged above.
            let (status, message) = match &e {
                crate::auth::ldap::LdapAuthError::BindFailed { .. } => {
                    (StatusCode::UNAUTHORIZED, "Invalid credentials")
                }
                crate::auth::ldap::LdapAuthError::InsecureConnection(_) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Authentication service configuration error",
                ),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Authentication service temporarily unavailable",
                ),
            };
            ApiError::new(status, message).into_response()
        }
    }
}

// ─── OIDC SSO ──────────────────────────────────────────

/// Response payload for `GET /api/auth/oidc/login` — the URL the SPA should
/// redirect the user agent to in order to start the IdP authorization flow.
#[derive(Debug, serde::Serialize)]
pub struct OidcLoginResponse {
    pub url: String,
}

/// Query parameters supplied by the IdP on the redirect back to
/// `GET /api/auth/oidc/callback`.
#[derive(Debug, serde::Deserialize)]
pub struct OidcCallbackParams {
    pub code: String,
    pub state: String,
    /// IdPs may surface authorization errors via `error` / `error_description`.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Result of a successful OIDC callback — same shape as the LDAP login
/// response plus an optional email.
#[derive(Debug, serde::Serialize)]
pub struct OidcCallbackResponse {
    pub key: String,
    pub key_prefix: String,
    pub username: String,
    pub group: String,
    pub email: Option<String>,
}

/// `GET /api/auth/oidc/login` — begin the OIDC authorization code flow.
///
/// Returns the IdP authorization URL as JSON. The SPA is responsible for
/// performing the actual redirect (more flexible than an HTTP 30x for SPA
/// front-ends). Returns 503 if OIDC is not configured.
pub async fn oidc_login(State(state): State<std::sync::Arc<AppState>>) -> axum::response::Response {
    let oidc_config = match &state.oidc_config {
        Some(cfg) => cfg.clone(),
        None => {
            return ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "OIDC authentication is not configured",
            )
            .into_response();
        }
    };

    let authenticator = match OidcAuthenticator::new(oidc_config) {
        Ok(auth) => auth,
        Err(e) => {
            tracing::error!(error = %e, "OIDC authenticator construction failed");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authentication service configuration error",
            )
            .into_response();
        }
    };

    let (url, _state) = authenticator.authorization_url();
    // TODO: persist `_state` in a server-side session/cookie and verify it
    // in the callback. For this phase we rely on the single-use code +
    // redirect_uri binding for CSRF defence.
    Json(super::ApiResponse::ok(OidcLoginResponse { url })).into_response()
}

/// `GET /api/auth/oidc/callback` — handle the IdP redirect after user consent.
///
/// Exchanges the one-time authorization code for tokens, extracts user
/// identity (ID token claims or userinfo endpoint), and provisions a virtual
/// API key mirroring the LDAP login path.
pub async fn oidc_callback(
    State(state): State<std::sync::Arc<AppState>>,
    Query(params): Query<OidcCallbackParams>,
) -> axum::response::Response {
    let oidc_config = match &state.oidc_config {
        Some(cfg) => cfg.clone(),
        None => {
            return ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "OIDC authentication is not configured",
            )
            .into_response();
        }
    };

    // Surface IdP-originated authorization errors (user denied consent, etc.).
    if let Some(err) = &params.error {
        tracing::warn!(
            error = %err,
            description = ?params.error_description,
            "OIDC authorization server returned an error"
        );
        return ApiError::new(StatusCode::BAD_REQUEST, "Authorization failed").into_response();
    }

    // TODO: validate `params.state` against the value issued in `oidc_login`.
    // Requires server-side session storage (cookie or KV). Skipped for this
    // phase; the authorization code is one-time-use and bound to redirect_uri.
    tracing::debug!(callback_state = %params.state, "OIDC callback received");

    let authenticator = match OidcAuthenticator::new(oidc_config) {
        Ok(auth) => auth,
        Err(e) => {
            tracing::error!(error = %e, "OIDC authenticator construction failed");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authentication service configuration error",
            )
            .into_response();
        }
    };

    let tokens = match authenticator.exchange_code(&params.code).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "OIDC token exchange failed");
            return ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Authentication service temporarily unavailable",
            )
            .into_response();
        }
    };

    let user_info = match authenticator.extract_user_info(&tokens).await {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!(error = %e, "OIDC userinfo extraction failed");
            return ApiError::new(
                StatusCode::BAD_GATEWAY,
                "Authentication service temporarily unavailable",
            )
            .into_response();
        }
    };

    let store = &state.billing.virtual_key_store;
    let key_name = format!("oidc:{}", user_info.username);

    // Dedupe: delete any pre-existing virtual key for this user so each login
    // yields a fresh plaintext (mirrors the LDAP login path).
    let existing = store.list().await.into_iter().find(|k| k.name == key_name);
    if let Some(old_key) = existing {
        store.delete(old_key.id).await;
    }

    let group = user_info.groups.first().cloned().unwrap_or_default();
    let (new_key, plaintext) = store
        .create(
            key_name,
            None,   // daily_budget_cents
            None,   // monthly_budget_cents
            None,   // allowed_models
            vec![], // denied_models
            vec![], // allowed_ips
            None,   // rpm_limit
            None,   // tpm_limit
            None,   // expires_at
            Some(group.clone()),
        )
        .await;

    tracing::info!(
        username = %user_info.username,
        key_prefix = %new_key.key_prefix,
        "OIDC login succeeded, virtual key provisioned"
    );

    let resp = OidcCallbackResponse {
        key: plaintext,
        key_prefix: new_key.key_prefix,
        username: user_info.username,
        group,
        email: user_info.email,
    };
    Json(super::ApiResponse::ok(resp)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::ApiResponse;
    use axum::Json;
    use serde_json::json;
    use std::sync::Mutex;

    /// Serializes tests that share the pending_cookies.json file.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn pending_path() -> std::path::PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("modelswitch")
            .join("pending_cookies.json")
    }

    fn cleanup() {
        let _ = std::fs::remove_file(pending_path());
    }

    #[tokio::test]
    async fn receive_cookies_rejects_empty() {
        let _guard = TEST_LOCK.lock().unwrap();
        cleanup();
        let resp = receive_login_cookies(Json(json!({
            "provider": "openai",
            "cookies": ""
        })))
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        cleanup();
    }

    #[tokio::test]
    async fn receive_cookies_accepts_valid() {
        let _guard = TEST_LOCK.lock().unwrap();
        cleanup();
        let resp = receive_login_cookies(Json(json!({
            "provider": "openai",
            "cookies": "session=abc123"
        })))
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(
            pending_path().exists(),
            "pending_cookies.json should be created"
        );
        cleanup();
    }

    #[tokio::test]
    async fn receive_cookies_rejects_missing_cookies_field() {
        let _guard = TEST_LOCK.lock().unwrap();
        cleanup();
        let resp = receive_login_cookies(Json(json!({
            "provider": "openai"
        })))
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        cleanup();
    }

    #[tokio::test]
    async fn get_pending_returns_none_when_no_file() {
        let _guard = TEST_LOCK.lock().unwrap();
        cleanup();
        let Json(resp) = get_pending_cookies().await;
        assert!(resp.ok);
        assert!(resp.data.is_none());
        cleanup();
    }

    #[tokio::test]
    async fn get_pending_returns_data_then_clears() {
        let _guard = TEST_LOCK.lock().unwrap();
        cleanup();
        // Write a valid pending cookies file
        let path = pending_path();
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        let data = json!({
            "provider": "openai",
            "cookies": "session=xyz",
            "received_at": "2024-01-01T00:00:00Z"
        });
        std::fs::write(pending_path(), data.to_string()).unwrap();

        // First call should return data
        let Json(resp1) = get_pending_cookies().await;
        assert!(resp1.ok);
        assert!(resp1.data.is_some(), "first call should return data");

        // Second call should return None (file was deleted after first read)
        let Json(resp2) = get_pending_cookies().await;
        assert!(resp2.ok);
        assert!(
            resp2.data.is_none(),
            "second call should return None after file deletion"
        );
        cleanup();
    }
}
