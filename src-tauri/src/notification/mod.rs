pub mod bark;
pub mod webhook;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Notification event types dispatched by the gateway to configured channels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum NotificationEvent {
    /// Budget threshold reached (e.g., 80% of daily budget).
    BudgetThreshold {
        key_name: String,
        threshold_pct: u8,
        period: String,
    },
    /// Budget exhausted — all future requests rejected.
    BudgetExhausted { key_name: String, period: String },
    /// Channel automatically disabled due to failures.
    ChannelDisabled {
        channel_name: String,
        reason: String,
    },
    /// Channel recovered from circuit-open state.
    ChannelRecovered { channel_name: String },
}

/// Configuration for notification channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Webhook URL for HTTP POST notifications. HMAC-SHA256 signed when
    /// `webhook_secret` is set.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Secret used for HMAC-SHA256 webhook signing.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Bark push URL (e.g., `https://api.day.app/{key}`).
    #[serde(default)]
    pub bark_url: Option<String>,
    /// Budget threshold percentage that triggers a notification (default 80).
    #[serde(default = "default_threshold")]
    pub budget_threshold_pct: u8,
}

fn default_threshold() -> u8 {
    80
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            webhook_url: None,
            webhook_secret: None,
            bark_url: None,
            budget_threshold_pct: default_threshold(),
        }
    }
}

/// Notification service that dispatches events to configured channels.
///
/// Holds its own `reqwest::Client` (with a short timeout) and tracks which
/// budget keys have already crossed their threshold, so we don't spam
/// duplicate notifications on every request after the threshold is hit.
pub struct NotificationService {
    config: Arc<RwLock<NotificationConfig>>,
    http: reqwest::Client,
    /// Tracks which budget thresholds have already been notified to avoid
    /// duplicate alerts. Cleared automatically when usage drops back below
    /// the threshold.
    notified_thresholds: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl NotificationService {
    pub fn new(config: NotificationConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            notified_thresholds: Arc::new(RwLock::new(std::collections::HashSet::new())),
        }
    }

    /// Send a notification event to all configured channels.
    ///
    /// Non-blocking — each channel dispatch is spawned as an independent
    /// task. Errors are logged via `tracing::warn!` but never propagated
    /// to the caller, so notification failures cannot break request handling.
    pub async fn notify(&self, event: NotificationEvent) {
        let config = self.config.read().await;
        let event_json = serde_json::to_value(&event).unwrap_or_default();

        if let Some(ref url) = config.webhook_url {
            let secret = config.webhook_secret.clone();
            let http = self.http.clone();
            let payload = event_json.clone();
            let url = url.clone();
            tokio::spawn(async move {
                if let Err(e) = webhook::send_webhook(&http, &url, &secret, &payload).await {
                    tracing::warn!(error = %e, "Webhook notification failed");
                }
            });
        }

        if let Some(ref url) = config.bark_url {
            let http = self.http.clone();
            let title = event_title(&event);
            let body = serde_json::to_string(&event).unwrap_or_default();
            let url = url.clone();
            tokio::spawn(async move {
                if let Err(e) = bark::send_bark(&http, &url, &title, &body).await {
                    tracing::warn!(error = %e, "Bark notification failed");
                }
            });
        }
    }

    /// Check whether a budget threshold should trigger a notification.
    ///
    /// Returns `true` the first time a key crosses the configured threshold
    /// within the current "above-threshold" window. Returns `false` on
    /// subsequent calls until usage drops back below the threshold, at which
    /// point the state is reset and the next crossing will notify again.
    pub async fn check_budget_threshold(&self, key: &str, pct: u8) -> bool {
        let threshold = self.config.read().await.budget_threshold_pct;
        if pct < threshold {
            let mut notified = self.notified_thresholds.write().await;
            notified.remove(key);
            return false;
        }
        let mut notified = self.notified_thresholds.write().await;
        notified.insert(key.to_string())
    }

    /// Return the current notification configuration.
    pub async fn get_config(&self) -> NotificationConfig {
        self.config.read().await.clone()
    }

    /// Replace the live notification configuration (e.g., from a config reload).
    pub async fn update_config(&self, config: NotificationConfig) {
        *self.config.write().await = config;
    }
}

/// Validate a webhook or bark URL to prevent SSRF attacks.
///
/// Rejects non-HTTP(S) schemes, private/loopback/link-local IPs, and
/// cloud metadata endpoints. Also resolves the hostname via DNS and
/// checks each resolved IP address against the same denylist.
pub async fn validate_notification_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;

    // Only allow http and https schemes
    match parsed.scheme() {
        "https" => {}
        "http" => {
            let host = parsed.host_str().unwrap_or("");
            if host != "localhost" && host != "127.0.0.1" {
                return Err(
                    "HTTP is only allowed for localhost. Use HTTPS for production webhooks.".into(),
                );
            }
        }
        scheme => return Err(format!("Unsupported URL scheme: {scheme}. Only http/https allowed.")),
    }

    let host = parsed.host_str().unwrap_or("");
    let port = parsed.port().unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

    // Check hostname string for literal IP matches first
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        check_ip(&ip)?;
    }

    // DNS resolution: resolve hostname and check all resolved IPs.
    // This catches DNS-based bypasses like 169.254.169.254.nip.io.
    let socket_addrs = match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(e) => return Err(format!("DNS resolution failed for {host}: {e}")),
    };

    if socket_addrs.is_empty() {
        return Err(format!("DNS resolution returned no addresses for {host}"));
    }

    for addr in socket_addrs {
        check_ip(&addr.ip())?;
    }

    Ok(())
}

/// Check a single IP address against the SSRF denylist.
fn check_ip(ip: &std::net::IpAddr) -> Result<(), String> {
    match ip {
        std::net::IpAddr::V4(v4) => {
            if v4.is_private() {
                return Err(format!("Private IP address not allowed: {v4}"));
            }
            if v4.is_link_local() {
                return Err(format!("Link-local address not allowed: {v4}"));
            }
            if v4.is_unspecified() {
                return Err(format!("Unspecified address not allowed: {v4}"));
            }
            if v4.is_broadcast() {
                return Err(format!("Broadcast address not allowed: {v4}"));
            }
            // Allow 127.0.0.1 for dev, block other loopback
            if v4.is_loopback() && *v4 != std::net::Ipv4Addr::new(127, 0, 0, 1) {
                return Err(format!("Non-standard loopback address not allowed: {v4}"));
            }
        }
        std::net::IpAddr::V6(v6) => {
            // Allow ::1 for localhost dev
            if v6.is_loopback() && *v6 != std::net::Ipv6Addr::LOCALHOST {
                return Err(format!("Non-standard IPv6 loopback not allowed: {v6}"));
            }
            if v6.is_unspecified() {
                return Err(format!("Unspecified IPv6 address not allowed: {v6}"));
            }
            if v6.is_multicast() {
                return Err(format!("Multicast address not allowed: {v6}"));
            }
        }
    }
    Ok(())
}

fn event_title(event: &NotificationEvent) -> String {
    match event {
        NotificationEvent::BudgetThreshold {
            key_name,
            threshold_pct,
            ..
        } => {
            format!("{} budget at {}%", key_name, threshold_pct)
        }
        NotificationEvent::BudgetExhausted { key_name, .. } => {
            format!("{} budget exhausted", key_name)
        }
        NotificationEvent::ChannelDisabled { channel_name, .. } => {
            format!("Channel {} disabled", channel_name)
        }
        NotificationEvent::ChannelRecovered { channel_name } => {
            format!("Channel {} recovered", channel_name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_title_budget_threshold() {
        let event = NotificationEvent::BudgetThreshold {
            key_name: "openai-prod".into(),
            threshold_pct: 80,
            period: "daily".into(),
        };
        assert_eq!(event_title(&event), "openai-prod budget at 80%");
    }

    #[test]
    fn event_title_budget_exhausted() {
        let event = NotificationEvent::BudgetExhausted {
            key_name: "anthropic".into(),
            period: "monthly".into(),
        };
        assert_eq!(event_title(&event), "anthropic budget exhausted");
    }

    #[test]
    fn event_title_channel_disabled() {
        let event = NotificationEvent::ChannelDisabled {
            channel_name: "azure-east".into(),
            reason: "5 consecutive 5xx".into(),
        };
        assert_eq!(event_title(&event), "Channel azure-east disabled");
    }

    #[test]
    fn event_title_channel_recovered() {
        let event = NotificationEvent::ChannelRecovered {
            channel_name: "vertex".into(),
        };
        assert_eq!(event_title(&event), "Channel vertex recovered");
    }

    #[tokio::test]
    async fn check_threshold_notifies_on_first_crossing() {
        let svc = NotificationService::new(NotificationConfig {
            budget_threshold_pct: 80,
            ..Default::default()
        });

        // Below threshold — no notification, and state stays clear.
        assert!(!svc.check_budget_threshold("k1", 50).await);

        // First time at/above threshold — should notify.
        assert!(svc.check_budget_threshold("k1", 80).await);

        // Second check at/above threshold — should NOT re-notify.
        assert!(!svc.check_budget_threshold("k1", 90).await);
    }

    #[tokio::test]
    async fn check_threshold_resets_after_dropping_below() {
        let svc = NotificationService::new(NotificationConfig {
            budget_threshold_pct: 80,
            ..Default::default()
        });

        // Cross threshold.
        assert!(svc.check_budget_threshold("k1", 85).await);
        // Drop below — resets the notified set.
        assert!(!svc.check_budget_threshold("k1", 40).await);
        // Cross again — should notify once more.
        assert!(svc.check_budget_threshold("k1", 80).await);
    }

    #[tokio::test]
    async fn check_threshold_tracks_keys_independently() {
        let svc = NotificationService::new(NotificationConfig {
            budget_threshold_pct: 80,
            ..Default::default()
        });

        assert!(svc.check_budget_threshold("alpha", 80).await);
        // Different key should also notify on its first crossing.
        assert!(svc.check_budget_threshold("beta", 80).await);
        // Re-checking alpha should not re-notify.
        assert!(!svc.check_budget_threshold("alpha", 80).await);
    }

    #[tokio::test]
    async fn update_config_changes_threshold() {
        let svc = NotificationService::new(NotificationConfig {
            budget_threshold_pct: 80,
            ..Default::default()
        });

        // At 80 with threshold 80 — notifies.
        assert!(svc.check_budget_threshold("k1", 80).await);
        // Reset.
        assert!(!svc.check_budget_threshold("k1", 70).await);

        // Raise threshold — 80 should no longer trigger.
        svc.update_config(NotificationConfig {
            budget_threshold_pct: 90,
            ..Default::default()
        })
        .await;
        assert!(!svc.check_budget_threshold("k1", 80).await);
        assert!(svc.check_budget_threshold("k1", 95).await);
    }

    #[test]
    fn default_threshold_is_80() {
        assert_eq!(default_threshold(), 80);
    }

    #[test]
    fn notification_config_defaults_to_no_channels() {
        let cfg = NotificationConfig::default();
        assert!(cfg.webhook_url.is_none());
        assert!(cfg.webhook_secret.is_none());
        assert!(cfg.bark_url.is_none());
        assert_eq!(cfg.budget_threshold_pct, 80);
    }

    #[tokio::test]
    async fn validate_https_url_passes() {
        // Only run if DNS is available
        let result = validate_notification_url("https://hooks.slack.com/services/T00/B00/XX").await;
        assert!(result.is_ok() || result.unwrap_err().contains("DNS resolution failed"));
    }

    #[tokio::test]
    async fn validate_localhost_http_passes() {
        assert!(validate_notification_url("http://localhost:8080/webhook").await.is_ok());
        assert!(validate_notification_url("http://127.0.0.1:9090/webhook").await.is_ok());
    }

    #[tokio::test]
    async fn validate_non_localhost_http_rejected() {
        let result = validate_notification_url("http://example.com/webhook").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HTTPS"));
    }

    #[tokio::test]
    async fn validate_private_ip_rejected() {
        assert!(validate_notification_url("https://10.0.0.1/webhook").await.is_err());
        assert!(validate_notification_url("https://192.168.1.1/webhook").await.is_err());
        assert!(validate_notification_url("https://172.16.0.1/webhook").await.is_err());
    }

    #[tokio::test]
    async fn validate_cloud_metadata_rejected() {
        let result = validate_notification_url("https://169.254.169.254/latest/meta-data/").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Link-local"));
    }

    #[tokio::test]
    async fn validate_non_standard_loopback_rejected() {
        let result = validate_notification_url("http://127.0.0.2:8080/").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn validate_non_http_scheme_rejected() {
        assert!(validate_notification_url("file:///etc/passwd").await.is_err());
        assert!(validate_notification_url("ftp://example.com/").await.is_err());
    }
}
