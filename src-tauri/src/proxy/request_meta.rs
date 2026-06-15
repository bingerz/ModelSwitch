use axum::http::HeaderMap;
use serde_json::Value;
use uuid::Uuid;

use crate::router::affinity::SessionAffinity;

/// Metadata extracted from the incoming request at the start of dispatch.
pub(super) struct RequestMeta<'a> {
    pub original_model: String,
    pub is_stream: bool,
    pub request_id: Option<&'a str>,
    pub session_id: Option<String>,
    pub affinity_channel: Option<Uuid>,
}

/// Check if an affinity channel is still valid (available, supports model, not circuit-open).
pub(super) async fn is_affinity_valid(
    affinity_id: Uuid,
    channels: &crate::channel::SharedChannels,
    requested_model: &str,
) -> bool {
    let guard = channels.read().await;
    let valid = guard.iter().any(|c| {
        if c.id != affinity_id {
            return false;
        }
        let mut c = c.clone();
        c.recover_if_expired();
        c.is_available()
            && (c.model_mapping.is_empty() || c.model_mapping.contains_key(requested_model))
    });
    drop(guard);
    valid
}

/// Extract the virtual key id (if any) injected by `virtual_key_middleware`.
/// Returns None when no virtual keys are configured or the header is absent.
pub(super) fn extract_virtual_key_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("x-virtual-key-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// Extract request metadata (model, stream flag, session affinity, request ID).
pub(super) async fn extract_request_meta<'a>(
    body: &Value,
    original_headers: &'a HeaderMap,
    state: &std::sync::Arc<crate::proxy::openai::AppState>,
    default_model: &'static str,
) -> RequestMeta<'a> {
    let original_model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or(default_model)
        .to_string();
    let is_stream = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let request_id = original_headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok());
    let session_id = SessionAffinity::extract_session_id(body);
    let affinity_channel = if let Some(ref sid) = &session_id {
        state.router.session_affinity.get_channel(sid).await
    } else {
        None
    };
    RequestMeta {
        original_model,
        is_stream,
        request_id,
        session_id,
        affinity_channel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{
        Channel, ChannelStatus, Credential, CredentialType, Provider, SharedChannels,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_test_channel(id: Uuid, model_mapping: HashMap<String, String>) -> Channel {
        Channel {
            id,
            name: "test-channel".to_string(),
            provider: Provider::OpenAI,
            priority: 1,
            weight: 1,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential: Credential {
                cred_type: CredentialType::ApiKey,
                key_ref: "test-key".to_string(),
                api_key: Some("sk-test".to_string()),
                expires_at: None,
            },
            enabled: true,
            status: ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: "https://api.openai.com/v1".to_string(),
            model_mapping,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            avg_latency_ms: 0,
            consecutive_failures: 0,
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            failure_window_start: None,
            window_failure_count: 0,
            max_concurrent: None,
        }
    }

    #[test]
    fn extract_virtual_key_id_from_valid_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-virtual-key-id",
            "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        );
        let id = extract_virtual_key_id(&headers);
        assert!(id.is_some());
        assert_eq!(
            id.unwrap().to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn extract_virtual_key_id_returns_none_when_absent() {
        let headers = HeaderMap::new();
        assert!(extract_virtual_key_id(&headers).is_none());
    }

    #[test]
    fn extract_virtual_key_id_returns_none_for_invalid_uuid() {
        let mut headers = HeaderMap::new();
        headers.insert("x-virtual-key-id", "not-a-uuid".parse().unwrap());
        assert!(extract_virtual_key_id(&headers).is_none());
    }

    #[tokio::test]
    async fn affinity_valid_for_healthy_channel_with_matching_model() {
        let channel_id = Uuid::new_v4();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("gpt-4".to_string(), "gpt-4".to_string());
        let channel = make_test_channel(channel_id, model_mapping);
        let channels: SharedChannels = Arc::new(RwLock::new(vec![channel]));
        assert!(is_affinity_valid(channel_id, &channels, "gpt-4").await);
    }

    #[tokio::test]
    async fn affinity_valid_for_channel_with_empty_mapping() {
        let channel_id = Uuid::new_v4();
        let channel = make_test_channel(channel_id, HashMap::new());
        let channels: SharedChannels = Arc::new(RwLock::new(vec![channel]));
        assert!(is_affinity_valid(channel_id, &channels, "gpt-4").await);
    }

    #[tokio::test]
    async fn affinity_invalid_for_unknown_channel() {
        let channels: SharedChannels = Arc::new(RwLock::new(vec![]));
        let random_id = Uuid::new_v4();
        assert!(!is_affinity_valid(random_id, &channels, "gpt-4").await);
    }

    #[tokio::test]
    async fn affinity_invalid_for_channel_without_model() {
        let channel_id = Uuid::new_v4();
        let mut model_mapping = HashMap::new();
        model_mapping.insert("claude-3".to_string(), "claude-3-opus".to_string());
        let channel = make_test_channel(channel_id, model_mapping);
        let channels: SharedChannels = Arc::new(RwLock::new(vec![channel]));
        assert!(!is_affinity_valid(channel_id, &channels, "gpt-4").await);
    }
}
