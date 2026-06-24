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
    /// Account group tag from `X-Account-Group` header for tag-based routing.
    /// When set, only channels with a matching `account_group` or no group
    /// are considered for dispatch.
    pub account_group: Option<String>,
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
    state: &std::sync::Arc<crate::proxy::AppState>,
    default_model: &str,
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
    let session_id = SessionAffinity::extract_session_id(original_headers, body);
    let affinity_channel = if let Some(ref sid) = &session_id {
        state.router.session_affinity.get_channel(sid).await
    } else {
        None
    };
    let account_group = original_headers
        .get("x-account-group")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    RequestMeta {
        original_model,
        is_stream,
        request_id,
        session_id,
        affinity_channel,
        account_group,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
