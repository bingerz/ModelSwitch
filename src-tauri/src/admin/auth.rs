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
