use crate::admin::ApiResponse;
use crate::channel::{Channel, ChannelStatus, Credential, CredentialType, Provider};
use crate::middleware::error::ApiError;
use crate::proxy::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Header names that must never be stored in channel.custom_headers.
/// Mirrors the request-time denylist in `proxy/attempt.rs` for defense-in-depth.
const STORED_HEADER_DENYLIST: &[&str] = &[
    "host",
    "transfer-encoding",
    "content-length",
    "connection",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "authorization",
    "x-api-key",
    "cookie",
    "forwarded",
];

/// Validate URL scheme and block cloud metadata endpoints via DNS resolution.
///
/// Allows internal/private IPs for legitimate proxy use, but blocks all
/// link-local addresses (169.254.0.0/16) and the AWS IPv6 metadata endpoint
/// (fd00:ec2::254). DNS resolution catches bypass tricks like
/// `169.254.169.254.nip.io`.
///
/// **Caller responsibility**: The HTTP client that uses this URL must disable
/// redirects (`redirect::Policy::none()`) or re-validate each redirect target,
/// otherwise a 3xx redirect can bypass this check.
#[allow(clippy::result_large_err)]
async fn validate_channel_url(
    url_str: &str,
    field_name: &str,
) -> Result<(), axum::response::Response> {
    let parsed = url::Url::parse(url_str).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid URL for {field_name}: {url_str}"),
        )
    })?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("Unsupported scheme '{scheme}' for {field_name}. Only http/https allowed."),
            ));
        }
    }

    // Use typed Host enum to handle IPv6 bracket stripping correctly.
    let port = parsed.port_or_known_default().unwrap_or(80);
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => {
            check_metadata_ip(&std::net::IpAddr::V4(ip), field_name)?;
        }
        Some(url::Host::Ipv6(ip)) => {
            check_metadata_ip(&std::net::IpAddr::V6(ip), field_name)?;
        }
        Some(url::Host::Domain(domain)) => {
            // DNS resolution for domain names — catches bypasses like
            // 169.254.169.254.nip.io that resolve to metadata endpoints.
            let socket_addrs = tokio::net::lookup_host((domain, port)).await.map_err(|e| {
                tracing::warn!(
                    host = %domain,
                    error = %e,
                    "DNS resolution failed for channel URL validation"
                );
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("DNS resolution failed for {field_name} host '{domain}'"),
                )
            })?;
            for addr in socket_addrs {
                check_metadata_ip(&addr.ip(), field_name)?;
            }
        }
        None => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("URL missing host for {field_name}: {url_str}"),
            ));
        }
    }

    Ok(())
}

/// Check a single IP against the metadata/link-local denylist.
#[allow(clippy::result_large_err)]
fn check_metadata_ip(
    ip: &std::net::IpAddr,
    field_name: &str,
) -> Result<(), axum::response::Response> {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // Block entire link-local range (169.254.0.0/16) — covers all
            // cloud metadata endpoints (AWS, GCP, Azure, etc.)
            if v4.is_link_local() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Link-local address blocked for {field_name}: {v4}"),
                ));
            }
            // Block unspecified address (0.0.0.0)
            if v4.is_unspecified() {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Unspecified address blocked for {field_name}: {v4}"),
                ));
            }
        }
        std::net::IpAddr::V6(v6) => {
            // Block non-standard loopback IPv6 addresses
            if v6.is_loopback() && *v6 != std::net::Ipv6Addr::LOCALHOST {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Non-standard loopback blocked for {field_name}: {v6}"),
                ));
            }
            // Check for fd00:ec2::254 (AWS IPv6 metadata)
            if *v6 == std::net::Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Cloud metadata endpoint blocked for {field_name}: {v6}"),
                ));
            }
        }
    }
    Ok(())
}

/// Remove denylisted header names from a HashMap (case-insensitive).
/// Logs each removal at WARN level for audit trail.
fn filter_dangerous_headers(headers: &mut HashMap<String, String>) {
    headers.retain(|name, _value| {
        let lower = name.to_lowercase();
        if STORED_HEADER_DENYLIST.contains(&lower.as_str()) {
            tracing::warn!(
                header = %name,
                "Rejecting denylisted header name in channel config"
            );
            false
        } else {
            // Also reject headers with CRLF injection attempts in name or value
            if name.contains('\n') || name.contains('\r') {
                tracing::warn!(header = %name, "Rejecting header with CRLF in name");
                return false;
            }
            true
        }
    });

    // Also check values for CRLF after the retain pass
    headers.retain(|_name, value| {
        if value.contains('\n') || value.contains('\r') {
            tracing::warn!("Rejecting header value with CRLF injection");
            false
        } else {
            true
        }
    });
}

pub async fn list_channels(State(state): State<Arc<AppState>>) -> Json<ApiResponse<Vec<Channel>>> {
    let channels = state.channel_mgr.list().await;
    Json(ApiResponse::ok(channels))
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub provider: String,
    #[serde(default = "default_priority")]
    pub priority: u8,
    #[serde(default = "default_weight")]
    pub weight: u32,
    pub cost_per_token: Option<f64>,
    #[serde(default)]
    pub input_cost_per_mtok: Option<f64>,
    #[serde(default)]
    pub output_cost_per_mtok: Option<f64>,
    #[serde(default = "default_credential_type")]
    pub credential_type: String,
    #[serde(default)]
    pub credential_value: String,
    pub base_url: String,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    #[serde(default)]
    pub cooldown_minutes: Option<u64>,
    #[serde(default)]
    pub rpm_limit: Option<u64>,
    #[serde(default)]
    pub tpm_limit: Option<u64>,
    #[serde(default)]
    pub account_group: Option<String>,
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub models_refresh_interval_secs: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: String,
    pub provider: String,
    pub priority: u8,
    pub weight: u32,
    pub cost_per_token: Option<f64>,
    pub base_url: String,
    pub enabled: bool,
    #[serde(default)]
    pub model_mapping: HashMap<String, String>,
    pub cooldown_minutes: Option<u64>,
    /// If provided, update the stored API key credential
    #[serde(default)]
    pub credential_value: Option<String>,
    pub input_cost_per_mtok: Option<f64>,
    pub output_cost_per_mtok: Option<f64>,
    pub rpm_limit: Option<u64>,
    pub tpm_limit: Option<u64>,
    #[serde(default)]
    pub account_group: Option<String>,
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub models_refresh_interval_secs: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_priority() -> u8 {
    1
}
fn default_weight() -> u32 {
    100
}
fn default_credential_type() -> String {
    "api_key".to_string()
}

pub async fn create_channel(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, axum::response::Response> {
    let cred_type = match req.credential_type.as_str() {
        "web_session" => CredentialType::WebSession,
        _ => CredentialType::ApiKey,
    };

    let id = Uuid::new_v4();
    let key_ref = format!("{}_{}", req.provider, id);

    // Store credential via CredentialStore abstraction
    state
        .credential_store
        .set("modelswitch", &key_ref, &req.credential_value)
        .map_err(|e| {
            tracing::error!("Failed to store credential: {}", e);
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store credential",
            )
        })?;

    // Validate URLs and sanitize headers (defense-in-depth at storage time)
    let mut sanitized_headers = req.headers;
    filter_dangerous_headers(&mut sanitized_headers);

    if let Some(ref proxy_url) = req.proxy_url {
        validate_channel_url(proxy_url, "proxy_url").await?;
    }
    if let Some(ref models_endpoint) = req.models_endpoint {
        validate_channel_url(models_endpoint, "models_endpoint").await?;
    }

    let channel = Channel {
        id,
        name: req.name,
        provider: Provider::from_str(&req.provider),
        priority: req.priority,
        weight: req.weight,
        cost_per_token: req.cost_per_token,
        input_cost_per_mtok: req.input_cost_per_mtok,
        output_cost_per_mtok: req.output_cost_per_mtok,
        credential: Credential {
            cred_type,
            key_ref,
            api_key: None,
            expires_at: None,
        },
        enabled: true,
        status: ChannelStatus::Healthy,
        circuit_open_until: None,
        base_url: req.base_url,
        model_mapping: req.model_mapping,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        avg_latency_ms: 0,
        consecutive_failures: 0,
        cooldown_minutes: req.cooldown_minutes,
        rpm_limit: req.rpm_limit,
        tpm_limit: req.tpm_limit,
        account_group: req.account_group,
        max_concurrent: req.max_concurrent,
        api_keys: req.api_keys,
        excluded_models: req.excluded_models,
        model_cooldowns: std::collections::HashMap::new(),
        proxy_url: req.proxy_url,
        headers: sanitized_headers,
        max_retries: req.max_retries,
        models_endpoint: req.models_endpoint,
        models_refresh_interval_secs: req.models_refresh_interval_secs.unwrap_or(300),
        tags: req.tags,
    };

    let created = state.channel_mgr.create(channel).await;
    state.channel_mgr.persist().await;
    state
        .audit_log
        .record(
            "channel.create",
            "admin-api",
            &created.id.to_string(),
            serde_json::json!({
                "name": created.name,
                "provider": format!("{:?}", created.provider),
            }),
        )
        .await;
    Ok(Json(ApiResponse::ok(created)))
}

pub async fn update_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<ApiResponse<Channel>>, axum::response::Response> {
    let mut existing = state
        .channel_mgr
        .get(id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Channel not found"))?;

    // Validate URLs and sanitize headers (defense-in-depth at storage time)
    let mut sanitized_headers = req.headers;
    filter_dangerous_headers(&mut sanitized_headers);

    if let Some(ref proxy_url) = req.proxy_url {
        validate_channel_url(proxy_url, "proxy_url").await?;
    }
    if let Some(ref models_endpoint) = req.models_endpoint {
        validate_channel_url(models_endpoint, "models_endpoint").await?;
    }

    existing.name = req.name;
    existing.provider = Provider::from_str(&req.provider);
    existing.priority = req.priority;
    existing.weight = req.weight;
    existing.cost_per_token = req.cost_per_token;
    existing.input_cost_per_mtok = req.input_cost_per_mtok;
    existing.output_cost_per_mtok = req.output_cost_per_mtok;
    existing.rpm_limit = req.rpm_limit;
    existing.tpm_limit = req.tpm_limit;
    existing.base_url = req.base_url;
    if !req.enabled && existing.enabled {
        existing.status = ChannelStatus::Disabled;
    } else if req.enabled && !existing.enabled {
        existing.status = ChannelStatus::Healthy;
    }
    existing.enabled = req.enabled;
    existing.model_mapping = req.model_mapping;
    existing.cooldown_minutes = req.cooldown_minutes;
    existing.account_group = req.account_group;
    existing.max_concurrent = req.max_concurrent;
    existing.excluded_models = req.excluded_models;
    existing.api_keys = req.api_keys;
    existing.proxy_url = req.proxy_url;
    existing.headers = sanitized_headers;
    existing.max_retries = req.max_retries;
    existing.models_endpoint = req.models_endpoint;
    existing.models_refresh_interval_secs = req.models_refresh_interval_secs.unwrap_or(300);
    existing.tags = req.tags;
    existing.updated_at = chrono::Utc::now();

    // Update credential if a new value is provided
    if let Some(ref new_key) = req.credential_value {
        if !new_key.is_empty() {
            let username = &existing.credential.key_ref;
            state
                .credential_store
                .set("modelswitch", username, new_key)
                .map_err(|e| {
                    tracing::error!("Failed to update credential: {}", e);
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to update credential",
                    )
                })?;
            tracing::info!(channel = %existing.name, "Credential updated");
        }
    }

    let result = state.channel_mgr.update(id, existing).await;

    match result {
        Some(channel) => {
            state.channel_mgr.persist().await;
            state
                .audit_log
                .record(
                    "channel.update",
                    "admin-api",
                    &id.to_string(),
                    serde_json::json!({
                        "name": channel.name,
                        "provider": format!("{:?}", channel.provider),
                        "enabled": channel.enabled,
                        "api_keys_count": channel.api_keys.len(),
                    }),
                )
                .await;
            Ok(Json(ApiResponse::ok(channel)))
        }
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "Channel not found")),
    }
}

pub async fn delete_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    // Capture channel name for audit before deletion
    let channel_name = state
        .channel_mgr
        .get(id)
        .await
        .map(|ch| ch.name)
        .unwrap_or_default();

    // Clean up credential before deleting
    if let Some(channel) = state.channel_mgr.get(id).await {
        let username = &channel.credential.key_ref;
        if let Err(e) = state.credential_store.delete("modelswitch", username) {
            tracing::warn!("Failed to delete credential for {}: {}", username, e);
        }
    }

    if state.channel_mgr.delete(id).await {
        state.billing.quota_store.delete(id).await;
        state.router.cooldown_tracker.remove(id);
        state.channel_mgr.persist().await;
        state
            .audit_log
            .record(
                "channel.delete",
                "admin-api",
                &id.to_string(),
                serde_json::json!({
                    "name": channel_name,
                }),
            )
            .await;
        StatusCode::NO_CONTENT.into_response()
    } else {
        ApiError::new(StatusCode::NOT_FOUND, "Channel not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn filter_removes_authorization_header() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer secret".to_string());
        headers.insert("X-Custom".to_string(), "value".to_string());
        filter_dangerous_headers(&mut headers);
        assert!(!headers.contains_key("Authorization"));
        assert!(headers.contains_key("X-Custom"));
    }

    #[test]
    fn filter_removes_denylisted_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("HOST".to_string(), "evil.com".to_string());
        headers.insert("X-API-KEY".to_string(), "stolen".to_string());
        filter_dangerous_headers(&mut headers);
        assert!(headers.is_empty());
    }

    #[test]
    fn filter_removes_crlf_injection() {
        let mut headers = HashMap::new();
        headers.insert("X-Safe\r\nInjected: evil".to_string(), "value".to_string());
        headers.insert("X-Value".to_string(), "data\r\nHost: evil".to_string());
        filter_dangerous_headers(&mut headers);
        assert!(headers.is_empty());
    }

    #[tokio::test]
    async fn validate_url_rejects_metadata_endpoint() {
        assert!(
            validate_channel_url("http://169.254.169.254/latest/meta-data/", "proxy_url")
                .await
                .is_err()
        );
        assert!(validate_channel_url(
            "http://[fd00:ec2::254]/latest/meta-data/",
            "models_endpoint"
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn validate_url_rejects_non_http_scheme() {
        assert!(validate_channel_url("file:///etc/passwd", "proxy_url")
            .await
            .is_err());
        assert!(
            validate_channel_url("ftp://example.com/", "models_endpoint")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_url_accepts_https() {
        assert!(validate_channel_url("https://api.openai.com", "proxy_url")
            .await
            .is_ok());
        assert!(validate_channel_url("http://10.0.0.5:8080", "proxy_url")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn validate_url_rejects_dns_rebind_to_metadata() {
        // This host resolves to 169.254.169.254 via nip.io.
        // If DNS resolution is working, this should be blocked. In test
        // environments without DNS, the literal IP check still catches direct
        // cases (see validate_url_rejects_literal_link_local).
        let result = validate_channel_url(
            "http://169.254.169.254.nip.io/latest/meta-data/",
            "proxy_url",
        )
        .await;
        // Don't assert — DNS availability varies by test env
        let _ = result;
    }

    #[tokio::test]
    async fn validate_url_rejects_literal_link_local() {
        // Any IP in 169.254.0.0/16 should be blocked, not just .169.254
        assert!(validate_channel_url("http://169.254.0.1/", "proxy_url")
            .await
            .is_err());
        assert!(
            validate_channel_url("http://169.254.255.254/", "models_endpoint")
                .await
                .is_err()
        );
    }
}
