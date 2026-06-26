use crate::middleware::error::ApiError;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

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
