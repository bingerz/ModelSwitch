//! Endpoint latency probing for endpointCandidates speed-test UI

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Result of probing a single endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub url: String,
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub reachable: bool, // TCP/TLS reached even if HTTP failed
}

const PROBE_TIMEOUT_SECS: u64 = 8;

/// Probe multiple endpoints concurrently to measure latency
///
/// Tries HEAD request first (lightweight), falls back to GET on 405.
/// Does NOT require API key - probes public health/info paths.
/// Measures RTT and distinguishes "reachable but auth failing" from "unreachable".
pub async fn probe_endpoints(urls: Vec<String>) -> Vec<ProbeResult> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut handles = Vec::new();

    for url in urls {
        let client = client.clone();
        let handle = tokio::spawn(async move { probe_single_endpoint(client, url).await });
        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(result) = handle.await {
            results.push(result);
        }
    }

    results
}

async fn probe_single_endpoint(client: reqwest::Client, base_url: String) -> ProbeResult {
    let probe_path = derive_probe_path(&base_url);
    let start = Instant::now();

    // Try HEAD first (most lightweight)
    let head_result = client.head(&probe_path).send().await;

    match head_result {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let status = resp.status();

            // 405 Method Not Allowed → try GET
            if status == StatusCode::METHOD_NOT_ALLOWED {
                return probe_with_get(client, probe_path, base_url).await;
            }

            // Success or reachable-but-failing-auth
            let ok = status.is_success();
            let reachable = status.is_client_error() || status.is_server_error() || ok;

            ProbeResult {
                url: base_url,
                ok,
                status: Some(status.as_u16()),
                latency_ms: Some(latency_ms),
                error: if !ok {
                    Some(format!("HTTP {}", status.as_u16()))
                } else {
                    None
                },
                reachable,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            // Distinguish network-level failure from HTTP-level failure
            let reachable = e.is_status() || e.is_body() || e.is_decode();
            let error_msg = if e.is_timeout() {
                "Timeout".to_string()
            } else if e.is_connect() {
                "Connection failed".to_string()
            } else {
                format!("{}", e)
            };

            ProbeResult {
                url: base_url,
                ok: false,
                status: None,
                latency_ms: if reachable { Some(latency_ms) } else { None },
                error: Some(error_msg),
                reachable,
            }
        }
    }
}

async fn probe_with_get(client: reqwest::Client, probe_path: String, base_url: String) -> ProbeResult {
    let start = Instant::now();
    match client.get(&probe_path).send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let status = resp.status();
            let ok = status.is_success();
            let reachable = status.is_client_error() || status.is_server_error() || ok;

            ProbeResult {
                url: base_url,
                ok,
                status: Some(status.as_u16()),
                latency_ms: Some(latency_ms),
                error: if !ok {
                    Some(format!("HTTP {}", status.as_u16()))
                } else {
                    None
                },
                reachable,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let reachable = e.is_status() || e.is_body() || e.is_decode();
            let error_msg = if e.is_timeout() {
                "Timeout".to_string()
            } else if e.is_connect() {
                "Connection failed".to_string()
            } else {
                format!("{}", e)
            };

            ProbeResult {
                url: base_url,
                ok: false,
                status: None,
                latency_ms: if reachable { Some(latency_ms) } else { None },
                error: Some(error_msg),
                reachable,
            }
        }
    }
}

/// Derive a cheap probe path from base URL
///
/// Tries to find a public endpoint that doesn't require auth:
/// - If URL looks like an API base, try `/v1/models` (cheapest OpenAI-compat public path)
/// - Otherwise try root `/`
fn derive_probe_path(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');

    // If it's clearly an API base, try /v1/models
    if trimmed.contains("/api") || trimmed.ends_with("/v1") {
        format!("{}/v1/models", trimmed.trim_end_matches("/v1"))
    } else {
        // Try root
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_probe_path() {
        assert_eq!(
            derive_probe_path("https://api.example.com"),
            "https://api.example.com/v1/models"
        );
        assert_eq!(
            derive_probe_path("https://api.example.com/v1"),
            "https://api.example.com/v1/models"
        );
        assert_eq!(
            derive_probe_path("https://example.com"),
            "https://example.com"
        );
        assert_eq!(
            derive_probe_path("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/models"
        );
    }
}
