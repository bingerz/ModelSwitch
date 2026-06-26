use crate::proxy::AppState;
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

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // value_as_f64
    // ------------------------------------------------------------------

    #[test]
    fn value_as_f64_parses_number() {
        let v = serde_json::json!(42.5);
        assert_eq!(value_as_f64(&v), Some(42.5));
    }

    #[test]
    fn value_as_f64_parses_numeric_string() {
        let v = serde_json::json!("42.5");
        assert_eq!(value_as_f64(&v), Some(42.5));
    }

    #[test]
    fn value_as_f64_rejects_non_numeric_string() {
        assert_eq!(value_as_f64(&serde_json::json!("abc")), None);
        assert_eq!(value_as_f64(&serde_json::json!("")), None);
    }

    #[test]
    fn value_as_f64_rejects_null_and_bool() {
        assert_eq!(value_as_f64(&serde_json::json!(null)), None);
        assert_eq!(value_as_f64(&serde_json::json!(true)), None);
        assert_eq!(value_as_f64(&serde_json::json!(false)), None);
    }

    // ------------------------------------------------------------------
    // resolve_scrape_config
    // ------------------------------------------------------------------

    #[test]
    fn resolve_scrape_config_anthropic() {
        let cfg = resolve_scrape_config("anthropic", "https://api.anthropic.com");
        assert!(cfg.is_some());
        let cfg = cfg.unwrap();
        assert_eq!(cfg.provider_id, "anthropic");
        assert!(cfg.target_url.contains("console.anthropic.com"));
    }

    #[test]
    fn resolve_scrape_config_baidu_aliases() {
        for provider in &["baidu", "qianfan"] {
            let cfg = resolve_scrape_config(provider, "");
            assert!(cfg.is_some(), "provider '{}' should resolve", provider);
            assert_eq!(cfg.unwrap().provider_id, "baidu");
        }
    }

    #[test]
    fn resolve_scrape_config_aliyun_aliases() {
        for provider in &["aliyun", "dashscope", "bailian"] {
            let cfg = resolve_scrape_config(provider, "");
            assert!(cfg.is_some(), "provider '{}' should resolve", provider);
            assert_eq!(cfg.unwrap().provider_id, "aliyun");
        }
    }

    #[test]
    fn resolve_scrape_config_doubao_aliases() {
        for provider in &["doubao", "volcengine", "bytedance"] {
            let cfg = resolve_scrape_config(provider, "");
            assert!(cfg.is_some(), "provider '{}' should resolve", provider);
            assert_eq!(cfg.unwrap().provider_id, "doubao");
        }
    }

    #[test]
    fn resolve_scrape_config_unknown_returns_none() {
        assert!(resolve_scrape_config("unknown", "https://example.com").is_none());
    }

    #[test]
    fn resolve_scrape_config_url_substring_fallback() {
        // URL containing "dashscope" → aliyun
        let cfg = resolve_scrape_config("custom", "https://dashscope.example.com");
        assert!(cfg.is_some());
        assert_eq!(cfg.unwrap().provider_id, "aliyun");

        // URL containing "aliyun" → aliyun
        let cfg = resolve_scrape_config("custom", "https://aliyun.example.com");
        assert!(cfg.is_some());
        assert_eq!(cfg.unwrap().provider_id, "aliyun");

        // "bigmodel" in URL → None (handled by HTTP collector, not WebView)
        assert!(resolve_scrape_config("custom", "https://bigmodel.cn").is_none());
    }

    // ------------------------------------------------------------------
    // parse_anthropic_result
    // ------------------------------------------------------------------

    #[test]
    fn parse_anthropic_result_extracts_fields() {
        let data = serde_json::json!({
            "five_hour": { "utilization": 42.5, "resets_at": "2024-01-01T00:00:00Z" },
            "seven_day": { "utilization": 75.0 }
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_anthropic_result(&data, id, "TestChannel").expect("should parse");

        assert_eq!(info.channel_id, id);
        assert_eq!(info.channel_name, "TestChannel");
        assert_eq!(info.provider, "anthropic");
        assert_eq!(info.source, "webview");
        assert_eq!(info.groups.len(), 2);
        assert_eq!(info.groups[0].window, "5h");
        assert_eq!(info.groups[0].utilization_pct, Some(42.5));
        assert_eq!(
            info.groups[0].resets_at.as_deref(),
            Some("2024-01-01T00:00:00Z")
        );
        assert_eq!(info.groups[1].window, "7d");
        assert_eq!(info.groups[1].utilization_pct, Some(75.0));
        assert!(info.groups[1].resets_at.is_none());
        // compact_text should contain both windows
        let compact = info.compact_text.expect("compact_text should be set");
        assert!(compact.contains("5h"));
        assert!(compact.contains("7d"));
    }

    #[test]
    fn parse_anthropic_result_err_on_invalid_shape() {
        // "five_hour" must be an object with "utilization"; a string fails deserialization
        let data = serde_json::json!({"five_hour": "not-an-object"});
        let id = uuid::Uuid::new_v4();
        let result = parse_anthropic_result(&data, id, "ch");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parse error"));
    }

    #[test]
    fn parse_anthropic_result_empty_object_ok() {
        // Empty object — all Option fields default to None, groups empty, no panic
        let data = serde_json::json!({});
        let id = uuid::Uuid::new_v4();
        let info = parse_anthropic_result(&data, id, "ch").expect("should parse");
        assert!(info.groups.is_empty());
        assert!(info.compact_text.is_none());
    }

    // ------------------------------------------------------------------
    // parse_baidu_result
    // ------------------------------------------------------------------

    #[test]
    fn parse_baidu_result_sums_totals() {
        let data = serde_json::json!({
            "result": [
                { "resourceName": "ERP_Coding", "totalAmount": 100, "remainAmount": 60, "usedAmount": 40 },
                { "resourceName": "ERP_Inference", "totalAmount": 200, "remainAmount": 150, "usedAmount": 50 }
            ]
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_baidu_result(&data, id, "BaiduChan").expect("should parse");

        assert_eq!(info.provider, "baidu");
        // Sums: remain = 60 + 150 = 210, total = 100 + 200 = 300
        assert_eq!(info.balance, Some(210.0));
        assert_eq!(info.limit, Some(300.0));
        // usage = total - remain = 300 - 210 = 90
        assert_eq!(info.usage, Some(90.0));
        assert_eq!(info.items.len(), 2);
        assert_eq!(info.items[0].label, "ERP_Coding");
        assert!(info.items[0].value.contains("60.00"));
    }

    #[test]
    fn parse_baidu_result_err_on_missing_array() {
        let data = serde_json::json!({"foo": "bar"});
        let id = uuid::Uuid::new_v4();
        let result = parse_baidu_result(&data, id, "ch");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no result/data array"));
    }

    #[test]
    fn parse_baidu_result_accepts_data_key_and_numeric_strings() {
        // Some Baidu responses use "data" instead of "result", and values may be strings
        let data = serde_json::json!({
            "data": [
                { "resourceName": "Res1", "totalAmount": "50", "remainAmount": "30" }
            ]
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_baidu_result(&data, id, "ch").expect("should parse");
        assert_eq!(info.balance, Some(30.0));
        assert_eq!(info.limit, Some(50.0));
        assert_eq!(info.items.len(), 1);
    }

    // ------------------------------------------------------------------
    // parse_aliyun_result
    // ------------------------------------------------------------------

    #[test]
    fn parse_aliyun_result_extracts_numeric_fields() {
        let data = serde_json::json!({
            "data": {
                "remainAmount": 123.45,
                "totalAmount": 500.0,
                "usedAmount": 376.55
            }
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_aliyun_result(&data, id, "AliChan").expect("should parse");

        assert_eq!(info.provider, "aliyun");
        assert_eq!(info.balance, Some(123.45));
        assert_eq!(info.limit, Some(500.0));
        assert_eq!(info.usage, Some(376.55));
    }

    #[test]
    fn parse_aliyun_result_extracts_items() {
        let data = serde_json::json!({
            "data": {
                "items": [
                    { "commodityCode": "MODEL_A", "deductQuantity": 12.5 },
                    { "name": "Model B", "remainQuantity": 7.3 }
                ]
            }
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_aliyun_result(&data, id, "ch").expect("should parse");
        assert_eq!(info.items.len(), 2);
        assert_eq!(info.items[0].label, "MODEL_A");
        assert_eq!(info.items[0].value, "12.50");
        assert_eq!(info.items[1].label, "Model B");
    }

    #[test]
    fn parse_aliyun_result_falls_back_to_page_text() {
        // No structured "data" block — should fall back to page_text (truncated to 200 chars)
        let long_text = "x".repeat(300);
        let data = serde_json::json!({ "page_text": long_text });
        let id = uuid::Uuid::new_v4();
        let info = parse_aliyun_result(&data, id, "ch").expect("should parse");

        assert!(info.balance.is_none());
        assert!(info.limit.is_none());
        let compact = info.compact_text.expect("compact_text should be set");
        assert_eq!(compact.len(), 200);
    }

    // ------------------------------------------------------------------
    // parse_doubao_result
    // ------------------------------------------------------------------

    #[test]
    fn parse_doubao_result_extracts_balance() {
        let data = serde_json::json!({
            "data": {
                "balance": 42.5,
                "totalQuota": 100.0,
                "usedQuota": 57.5
            }
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_doubao_result(&data, id, "DoubaoChan").expect("should parse");

        assert_eq!(info.provider, "doubao");
        assert_eq!(info.balance, Some(42.5));
        assert_eq!(info.limit, Some(100.0));
        assert_eq!(info.usage, Some(57.5));
    }

    #[test]
    fn parse_doubao_result_extracts_items_and_alias_fields() {
        // totalAmount/usedAmount should work as fallback field names alongside items
        let data = serde_json::json!({
            "data": {
                "balance": 10.0,
                "totalAmount": 200.0,
                "usedAmount": 190.0,
                "items": [
                    { "name": "Rocket", "remainAmount": 10.0 },
                    { "resourceName": "Doubao Pro", "remain": 5.5 }
                ]
            }
        });
        let id = uuid::Uuid::new_v4();
        let info = parse_doubao_result(&data, id, "ch").expect("should parse");
        assert_eq!(info.limit, Some(200.0));
        assert_eq!(info.usage, Some(190.0));
        assert_eq!(info.items.len(), 2);
        assert_eq!(info.items[0].label, "Rocket");
        assert_eq!(info.items[1].label, "Doubao Pro");
    }

    #[test]
    fn parse_doubao_result_falls_back_to_page_text() {
        let data = serde_json::json!({ "page_text": "remaining: 50 CNY" });
        let id = uuid::Uuid::new_v4();
        let info = parse_doubao_result(&data, id, "ch").expect("should parse");
        assert!(info.balance.is_none());
        assert!(info.limit.is_none());
        assert_eq!(info.compact_text.as_deref(), Some("remaining: 50 CNY"));
    }
}
