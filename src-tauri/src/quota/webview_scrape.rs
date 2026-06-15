use crate::proxy::openai::AppState;
use crate::quota::collectors::webview_scripts::aliyun::ALIYUN_BAILIAN_SCRIPT;
use crate::quota::collectors::webview_scripts::anthropic::{
    AnthropicUsage, ANTHROPIC_USAGE_SCRIPT,
};
use crate::quota::collectors::webview_scripts::baidu::BAIDU_CODING_PLAN_SCRIPT;
use crate::quota::collectors::webview_scripts::doubao::DOUBAO_USAGE_SCRIPT;
use crate::quota::{QuotaInfo, QuotaItem};
use std::sync::Arc;
use tauri::{Listener, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

/// Try to extract an f64 from a JSON value — accept either a number or a numeric string.
fn value_as_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
}

/// Configuration for a provider's WebView scraping session.
struct ScrapeConfig {
    /// URL to navigate to (where cookies are valid).
    target_url: &'static str,
    /// JavaScript to inject after page load.
    script: &'static str,
    /// Provider identifier.
    provider_id: &'static str,
}

fn resolve_scrape_config(provider: &str, base_url: &str) -> Option<ScrapeConfig> {
    match provider {
        "anthropic" => Some(ScrapeConfig {
            target_url: "https://console.anthropic.com",
            script: ANTHROPIC_USAGE_SCRIPT,
            provider_id: "anthropic",
        }),
        "baidu" | "qianfan" => Some(ScrapeConfig {
            target_url: "https://console.bce.baidu.com/qianfan",
            script: BAIDU_CODING_PLAN_SCRIPT,
            provider_id: "baidu",
        }),
        "aliyun" | "dashscope" | "bailian" => Some(ScrapeConfig {
            target_url: "https://bailian.console.aliyun.com",
            script: ALIYUN_BAILIAN_SCRIPT,
            provider_id: "aliyun",
        }),
        "doubao" | "volcengine" | "bytedance" => Some(ScrapeConfig {
            target_url: "https://console.volcengine.com/ark",
            script: DOUBAO_USAGE_SCRIPT,
            provider_id: "doubao",
        }),
        _ => {
            let lower = base_url.to_lowercase();
            if lower.contains("dashscope") || lower.contains("aliyun") {
                Some(ScrapeConfig {
                    target_url: "https://bailian.console.aliyun.com",
                    script: ALIYUN_BAILIAN_SCRIPT,
                    provider_id: "aliyun",
                })
            } else if lower.contains("bigmodel") {
                // Zhipu GLM — now handled by HTTP collector
                None
            } else {
                None
            }
        }
    }
}

/// Parse the raw JSON from Anthropic usage API into QuotaInfo.
fn parse_anthropic_result(
    data: &serde_json::Value,
    channel_id: uuid::Uuid,
    channel_name: &str,
) -> Result<QuotaInfo, String> {
    let usage: AnthropicUsage =
        serde_json::from_value(data.clone()).map_err(|e| format!("parse error: {}", e))?;

    let groups = usage.to_groups();

    // Build a compact text summary
    let compact = groups
        .iter()
        .map(|g| {
            format!(
                "{}: {:.0}%{}",
                g.window,
                g.utilization_pct.unwrap_or(0.0),
                g.resets_at
                    .as_ref()
                    .map(|r| format!(" (resets {})", r))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut info = QuotaInfo::new(channel_id, channel_name, "anthropic", "webview");
    info.groups = groups;
    info.compact_text = if compact.is_empty() {
        None
    } else {
        Some(compact)
    };
    Ok(info)
}

/// Parse the raw JSON from Baidu Qianfan coding plan API into QuotaInfo.
fn parse_baidu_result(
    data: &serde_json::Value,
    channel_id: uuid::Uuid,
    channel_name: &str,
) -> Result<QuotaInfo, String> {
    // Response shape: { "result": [ { "resourceName", "totalAmount", "remainAmount", "usedAmount" } ] }
    let resources = data
        .get("result")
        .or_else(|| data.get("data"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no result/data array in Baidu response".to_string())?;

    let mut items = Vec::new();
    let mut total_remain = 0.0_f64;
    let mut total_amount = 0.0_f64;

    for res in resources {
        let remain = res
            .get("remainAmount")
            .and_then(value_as_f64)
            .unwrap_or(0.0);
        let total = res.get("totalAmount").and_then(value_as_f64).unwrap_or(0.0);
        let _used = res.get("usedAmount").and_then(value_as_f64).unwrap_or(0.0);

        total_remain += remain;
        total_amount += total;

        if let Some(name) = res.get("resourceName").and_then(|v| v.as_str()) {
            items.push(QuotaItem {
                label: name.to_string(),
                value: format!("{:.2} / {:.2}", remain, total),
            });
        }
    }

    Ok(QuotaInfo {
        channel_id,
        channel_name: channel_name.to_string(),
        provider: "baidu".to_string(),
        balance: if total_remain > 0.0 {
            Some(total_remain)
        } else {
            None
        },
        limit: if total_amount > 0.0 {
            Some(total_amount)
        } else {
            None
        },
        usage: if total_amount > total_remain {
            Some(total_amount - total_remain)
        } else {
            None
        },
        items,
        source: "webview".to_string(),
        ..QuotaInfo::new(channel_id, channel_name, "baidu", "webview")
    })
}

/// Parse the raw JSON from Aliyun Bailian console into QuotaInfo.
fn parse_aliyun_result(
    data: &serde_json::Value,
    channel_id: uuid::Uuid,
    channel_name: &str,
) -> Result<QuotaInfo, String> {
    // Try structured billing data first
    // DashScope format: { "data": { "totalAmount": ..., "usedAmount": ..., "remainAmount": ... } }
    // Bailian format: { "data": { "items": [ { "commodityCode", "deductQuantity", ... } ] } }
    let mut info = QuotaInfo::new(channel_id, channel_name, "aliyun", "webview");

    if let Some(inner) = data.get("data") {
        // Try numeric fields
        if let Some(v) = inner.get("remainAmount").and_then(value_as_f64) {
            info.balance = Some(v);
        }
        if let Some(v) = inner.get("totalAmount").and_then(value_as_f64) {
            info.limit = Some(v);
        }
        if let Some(v) = inner.get("usedAmount").and_then(value_as_f64) {
            info.usage = Some(v);
        }

        // Try items array
        if let Some(items_arr) = inner.get("items").and_then(|v| v.as_array()) {
            let mut quota_items = Vec::new();
            for item in items_arr {
                if let (Some(name), Some(value)) = (
                    item.get("commodityCode")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str()),
                    item.get("deductQuantity")
                        .or_else(|| item.get("remainQuantity"))
                        .and_then(value_as_f64),
                ) {
                    quota_items.push(QuotaItem {
                        label: name.to_string(),
                        value: format!("{:.2}", value),
                    });
                }
            }
            if !quota_items.is_empty() {
                info.items = quota_items;
            }
        }
    }

    // If no structured data, try page_text fallback
    if info.balance.is_none() && info.limit.is_none() {
        if let Some(text) = data.get("page_text").and_then(|v| v.as_str()) {
            info.compact_text = Some(text.chars().take(200).collect());
        }
    }

    Ok(info)
}

/// Parse the raw JSON from DouBao/Volcengine Ark console into QuotaInfo.
fn parse_doubao_result(
    data: &serde_json::Value,
    channel_id: uuid::Uuid,
    channel_name: &str,
) -> Result<QuotaInfo, String> {
    let mut info = QuotaInfo::new(channel_id, channel_name, "doubao", "webview");

    // Try structured billing data
    // Expected: { "data": { "balance": ..., "totalQuota": ..., "usedQuota": ... } }
    // or: { "data": { "items": [ { "name", "remainAmount", "totalAmount" } ] } }
    if let Some(inner) = data.get("data") {
        if let Some(v) = inner.get("balance").and_then(value_as_f64) {
            info.balance = Some(v);
        }
        if let Some(v) = inner
            .get("totalQuota")
            .or_else(|| inner.get("totalAmount"))
            .and_then(value_as_f64)
        {
            info.limit = Some(v);
        }
        if let Some(v) = inner
            .get("usedQuota")
            .or_else(|| inner.get("usedAmount"))
            .and_then(value_as_f64)
        {
            info.usage = Some(v);
        }

        // Items array
        if let Some(items_arr) = inner.get("items").and_then(|v| v.as_array()) {
            let mut quota_items = Vec::new();
            for item in items_arr {
                if let (Some(name), Some(value)) = (
                    item.get("name")
                        .or_else(|| item.get("resourceName"))
                        .and_then(|v| v.as_str()),
                    item.get("remainAmount")
                        .or_else(|| item.get("remain"))
                        .and_then(value_as_f64),
                ) {
                    quota_items.push(QuotaItem {
                        label: name.to_string(),
                        value: format!("{:.2}", value),
                    });
                }
            }
            if !quota_items.is_empty() {
                info.items = quota_items;
            }
        }
    }

    // If no structured data, try page_text fallback
    if info.balance.is_none() && info.limit.is_none() {
        if let Some(text) = data.get("page_text").and_then(|v| v.as_str()) {
            info.compact_text = Some(text.chars().take(200).collect());
        }
    }

    Ok(info)
}

#[tauri::command]
pub async fn scrape_webview_quota(
    channel_id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let ch_id =
        uuid::Uuid::parse_str(&channel_id).map_err(|e| format!("Invalid channel ID: {}", e))?;

    let channel = state
        .channel_mgr
        .get(ch_id)
        .await
        .ok_or_else(|| "Channel not found".to_string())?;

    let config =
        resolve_scrape_config(channel.provider.as_str(), &channel.base_url).ok_or_else(|| {
            format!(
                "WebView scraping not supported for provider '{}' / URL '{}'",
                channel.provider.as_str(),
                channel.base_url
            )
        })?;

    let label = format!("quota-scrape-{}", uuid::Uuid::new_v4());

    // Create a oneshot channel to receive the result
    let (tx, rx) = oneshot::channel();

    // Wrap tx in Arc<Mutex> so the closure can consume it
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    // Listen for the JS result event
    let listener_id = app.listen("webview-quota-result", move |event| {
        let tx = tx.clone();
        tauri::async_runtime::spawn(async move {
            let mut guard = tx.lock().await;
            if let Some(tx) = guard.take() {
                let payload = event.payload().to_string();
                let _ = tx.send(payload);
            }
        });
    });

    // Create a hidden WebView window
    let url = config
        .target_url
        .parse()
        .map_err(|e| format!("Invalid URL: {}", e))?;

    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title("Quota Scrape")
        .inner_size(1.0, 1.0)
        .visible(false)
        .build()
        .map_err(|e| format!("Failed to create WebView: {}", e))?;

    // Wait for page to load, then inject script
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    window
        .eval(config.script)
        .map_err(|e| format!("Failed to inject script: {}", e))?;

    // Wait for result with timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), rx).await;

    // Cleanup: remove listener and close window
    app.unlisten(listener_id);
    let _ = window.close();

    let payload = result
        .map_err(|_| "Timeout waiting for WebView result".to_string())?
        .map_err(|_| "WebView result channel closed".to_string())?;

    // Parse the result
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).map_err(|e| format!("Invalid JSON from WebView: {}", e))?;

    let success = parsed
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !success {
        let error_msg = parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        let mut info = QuotaInfo::new(ch_id, &channel.name, config.provider_id, "webview");
        info.error = Some(error_msg.to_string());
        state.billing.quota_store.update(info.clone()).await;
        return Err(error_msg.to_string());
    }

    let data = parsed
        .get("data")
        .ok_or_else(|| "No data in WebView result".to_string())?;

    let info = match config.provider_id {
        "anthropic" => parse_anthropic_result(data, ch_id, &channel.name),
        "baidu" => parse_baidu_result(data, ch_id, &channel.name),
        "aliyun" => parse_aliyun_result(data, ch_id, &channel.name),
        "doubao" => parse_doubao_result(data, ch_id, &channel.name),
        _ => Err(format!("No parser for provider '{}'", config.provider_id)),
    };

    match info {
        Ok(mut info) => {
            info.updated_at = chrono::Utc::now();
            state.billing.quota_store.update(info.clone()).await;
            Ok(serde_json::to_value(info).unwrap_or_default())
        }
        Err(e) => {
            let mut info = QuotaInfo::new(ch_id, &channel.name, config.provider_id, "webview");
            info.error = Some(e.clone());
            state.billing.quota_store.update(info).await;
            Err(e)
        }
    }
}
