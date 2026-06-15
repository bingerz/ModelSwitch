//! WebView-based quota scraping for providers without billing APIs.
//! Feature-gated behind `tauri` feature.
//!
//! Opens a hidden WebView, injects JS to call billing API with session cookies,
//! and collects results via Tauri events.

use crate::quota::collectors::webview_scripts;

use crate::quota::QuotaInfo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

/// Maximum number of concurrent WebViews.
#[allow(dead_code)]
const MAX_ACTIVE_WEBVIEWS: usize = 3;

/// Pending result receivers keyed by a unique scrape ID.
#[allow(dead_code)]
type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<WebViewQuotaResult>>>>;

/// Result from a WebView scrape operation.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WebViewQuotaResult {
    pub provider: String,
    pub success: bool,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Manages WebView-based quota scraping sessions.
#[allow(dead_code)]
pub struct WebViewManager {
    pending: PendingMap,
    active_count: Arc<Mutex<usize>>,
}

#[allow(dead_code)]
impl WebViewManager {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            active_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Receive a result from a WebView JS callback (called from Tauri event handler).
    pub async fn handle_result(&self, result: WebViewQuotaResult) {
        // Find and complete the pending scrape by provider name
        let mut pending = self.pending.lock().await;
        // Find and remove one entry to complete
        let key = pending.keys().next().cloned();
        if let Some(k) = key {
            if let Some(tx) = pending.remove(&k) {
                let _ = tx.send(result);
            }
        }
    }

    /// Scrape Anthropic OAuth usage via WebView.
    /// Returns QuotaInfo with 5h/7d utilization groups.
    pub async fn scrape_anthropic(
        &self,
        channel_id: Uuid,
        channel_name: &str,
        app: &tauri::AppHandle,
    ) -> QuotaInfo {
        let mut active = self.active_count.lock().await;
        if *active >= MAX_ACTIVE_WEBVIEWS {
            return QuotaInfo::new(channel_id, channel_name, "anthropic", "webview")
                .with_error("too many active WebViews");
        }
        *active += 1;
        drop(active);

        let result = self
            .run_webview_scrape(
                "https://console.anthropic.com",
                webview_scripts::anthropic::ANTHROPIC_USAGE_SCRIPT,
                app,
            )
            .await;

        {
            let mut active = self.active_count.lock().await;
            *active = active.saturating_sub(1);
        }

        match result {
            Some(r) if r.success => {
                let mut info = QuotaInfo::new(channel_id, channel_name, "anthropic", "webview");
                if let Some(data) = r.data {
                    if let Ok(usage) =
                        serde_json::from_value::<webview_scripts::anthropic::AnthropicUsage>(data)
                    {
                        info.groups = usage.to_groups();
                        info.compact_text = Some(format!(
                            "5h: {}%, 7d: {}%",
                            usage
                                .five_hour
                                .as_ref()
                                .map(|w| w.utilization)
                                .unwrap_or(0.0),
                            usage
                                .seven_day
                                .as_ref()
                                .map(|w| w.utilization)
                                .unwrap_or(0.0),
                        ));
                    }
                }
                info
            }
            Some(r) => QuotaInfo::new(channel_id, channel_name, "anthropic", "webview")
                .with_error(r.error.as_deref().unwrap_or("unknown error")),
            None => QuotaInfo::new(channel_id, channel_name, "anthropic", "webview")
                .with_error("WebView scrape timed out"),
        }
    }

    /// Open a hidden WebView, inject JS, and wait for result via event.
    async fn run_webview_scrape(
        &self,
        _url: &str,
        _script: &str,
        _app: &tauri::AppHandle,
    ) -> Option<WebViewQuotaResult> {
        let (tx, rx) = oneshot::channel();
        let scrape_id = uuid::Uuid::new_v4().to_string();

        {
            let mut pending = self.pending.lock().await;
            pending.insert(scrape_id.clone(), tx);
        }

        // Note: Actual WebView creation requires Tauri AppHandle and is
        // done through tauri::WebviewWindowBuilder. This is a framework
        // placeholder — the real implementation creates a hidden window,
        // navigates to `url`, waits for load, then injects `script`.
        // Results come back via the Tauri event system.

        // For now, we set a timeout and wait
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx).await;

        match result {
            Ok(Ok(r)) => Some(r),
            _ => {
                let mut pending = self.pending.lock().await;
                pending.remove(&scrape_id);
                None
            }
        }
    }
}

/// Helper trait for QuotaInfo builder pattern.
#[allow(dead_code)]
trait QuotaInfoExt {
    fn with_error(self, msg: &str) -> Self;
}

impl QuotaInfoExt for QuotaInfo {
    fn with_error(mut self, msg: &str) -> Self {
        self.error = Some(msg.to_string());
        self
    }
}
