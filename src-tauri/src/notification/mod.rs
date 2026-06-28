pub mod bark;
pub mod email;
pub mod webhook;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

pub use email::{EmailNotifier, NotificationError};

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
    /// Enable SMTP email notifications.
    #[serde(default)]
    pub smtp_enabled: bool,
    /// SMTP server hostname (e.g., `smtp.gmail.com`).
    #[serde(default)]
    pub smtp_host: Option<String>,
    /// SMTP server port (typically 587 for STARTTLS, 465 for implicit TLS).
    #[serde(default)]
    pub smtp_port: Option<u16>,
    /// SMTP username for authentication.
    #[serde(default)]
    pub smtp_username: Option<String>,
    /// SMTP password for authentication. Redacted on GET responses.
    #[serde(default)]
    pub smtp_password: Option<String>,
    /// From address used on outgoing messages (e.g., `alerts@example.com`).
    #[serde(default)]
    pub smtp_from: Option<String>,
    /// Recipient for budget/operational alerts (admin distribution list).
    #[serde(default)]
    pub smtp_admin_email: Option<String>,
    /// Require STARTTLS before sending (recommended).
    #[serde(default = "default_smtp_use_tls")]
    pub smtp_use_tls: bool,
}

fn default_smtp_use_tls() -> bool {
    true
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
            smtp_enabled: false,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from: None,
            smtp_admin_email: None,
            smtp_use_tls: default_smtp_use_tls(),
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

        if config.smtp_enabled {
            if let Some(ref recipient) = config.smtp_admin_email {
                let subject = format!("[ModelSwitch] {}", event_title(&event));
                let body = serde_json::to_string_pretty(&event).unwrap_or_default();
                let recipient = recipient.clone();
                let smtp = build_email_notifier(&config).await;
                tokio::spawn(async move {
                    match smtp {
                        Ok(notifier) => {
                            if let Err(e) = notifier.send(&recipient, &subject, &body).await {
                                tracing::warn!(error = %e, "SMTP notification failed");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "SMTP notifier build failed");
                        }
                    }
                });
            }
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

/// SMTP-specific IP check. Like `check_ip` but allows private IPs since
/// internal SMTP relays are a legitimate use case.
fn check_smtp_ip(ip: &std::net::IpAddr) -> Result<(), String> {
    match ip {
        std::net::IpAddr::V4(v4) => {
            if v4.is_link_local() {
                return Err(format!("Link-local address not allowed: {v4}"));
            }
            if v4.is_unspecified() {
                return Err(format!("Unspecified address not allowed: {v4}"));
            }
            if v4.is_broadcast() {
                return Err(format!("Broadcast address not allowed: {v4}"));
            }
            // Allow 127.0.0.1 for local SMTP, block other loopback
            if v4.is_loopback() && *v4 != std::net::Ipv4Addr::new(127, 0, 0, 1) {
                return Err(format!("Non-standard loopback address not allowed: {v4}"));
            }
        }
        std::net::IpAddr::V6(v6) => {
            // Allow ::1 for local SMTP
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

/// Validate an SMTP host to prevent SSRF attacks.
///
/// For literal IP addresses, checks the IP directly. For hostnames, resolves
/// via DNS and checks each resolved IP. Unlike webhook validation, allows
/// private IPs (internal SMTP relays are a legitimate use case).
async fn validate_smtp_host(host: &str) -> Result<(), String> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return check_smtp_ip(&ip);
    }

    // Hostname — resolve via DNS and check each resolved IP.
    // This catches DNS-based bypasses like 169.254.169.254.nip.io.
    let socket_addrs = match tokio::net::lookup_host((host, 0)).await {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(e) => {
            // DNS failure is not a hard block for SMTP — the connection
            // will fail naturally if the host is unreachable.
            tracing::warn!(%host, error = %e, "Could not resolve SMTP host for SSRF validation");
            return Ok(());
        }
    };

    for addr in &socket_addrs {
        check_smtp_ip(&addr.ip())?;
    }

    Ok(())
}

/// Construct an `EmailNotifier` from the live notification configuration.
///
/// Returns `None`-like errors when mandatory SMTP fields are missing so
/// the dispatcher can log a clear reason without panicking.
async fn build_email_notifier(
    config: &NotificationConfig,
) -> Result<EmailNotifier, NotificationError> {
    let host = config
        .smtp_host
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| NotificationError::Smtp("smtp_host not configured".into()))?;

    // Validate SMTP host to prevent SSRF attacks
    validate_smtp_host(host)
        .await
        .map_err(NotificationError::Smtp)?;

    let port = config.smtp_port.unwrap_or(587);
    let username = config.smtp_username.as_deref().unwrap_or("");
    let password = config.smtp_password.as_deref().unwrap_or("");
    let from = config
        .smtp_from
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| NotificationError::Smtp("smtp_from not configured".into()))?;
    EmailNotifier::new(host, port, username, password, from, config.smtp_use_tls)
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
        scheme => {
            return Err(format!(
                "Unsupported URL scheme: {scheme}. Only http/https allowed."
            ))
        }
    }

    let host = parsed.host_str().unwrap_or("");
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

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
        assert!(!cfg.smtp_enabled);
        assert!(cfg.smtp_host.is_none());
        assert!(cfg.smtp_password.is_none());
        assert!(cfg.smtp_admin_email.is_none());
        assert!(cfg.smtp_use_tls);
    }

    #[tokio::test]
    async fn build_email_notifier_returns_err_when_host_missing() {
        let cfg = NotificationConfig::default();
        let result = build_email_notifier(&cfg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn build_email_notifier_returns_err_when_from_missing() {
        let cfg = NotificationConfig {
            smtp_enabled: true,
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(587),
            smtp_from: None,
            ..NotificationConfig::default()
        };
        let result = build_email_notifier(&cfg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn build_email_notifier_succeeds_with_complete_config() {
        let cfg = NotificationConfig {
            smtp_enabled: true,
            smtp_host: Some("127.0.0.1".into()),
            smtp_port: Some(587),
            smtp_username: Some("alerts".into()),
            smtp_password: Some("pass".into()),
            smtp_from: Some("alerts@example.com".into()),
            ..NotificationConfig::default()
        };
        let result = build_email_notifier(&cfg).await;
        assert!(result.is_ok());
    }

    #[test]
    fn smtp_password_serializes_like_webhook_secret() {
        // SMTP password follows the same redaction pattern as webhook_secret:
        // serialized normally, but manually redacted in the admin GET endpoint
        // so the frontend can round-trip the object without losing the value.
        let cfg = NotificationConfig {
            smtp_password: Some("sekret".into()),
            ..NotificationConfig::default()
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(
            json.contains("sekret"),
            "password must be present in raw serialization (redaction happens in admin layer): {json}"
        );
    }

    #[tokio::test]
    async fn validate_https_url_passes() {
        // Only run if DNS is available
        let result = validate_notification_url("https://hooks.slack.com/services/T00/B00/XX").await;
        assert!(result.is_ok() || result.unwrap_err().contains("DNS resolution failed"));
    }

    #[tokio::test]
    async fn validate_localhost_http_passes() {
        assert!(validate_notification_url("http://localhost:8080/webhook")
            .await
            .is_ok());
        assert!(validate_notification_url("http://127.0.0.1:9090/webhook")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn validate_non_localhost_http_rejected() {
        let result = validate_notification_url("http://example.com/webhook").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HTTPS"));
    }

    #[tokio::test]
    async fn validate_private_ip_rejected() {
        assert!(validate_notification_url("https://10.0.0.1/webhook")
            .await
            .is_err());
        assert!(validate_notification_url("https://192.168.1.1/webhook")
            .await
            .is_err());
        assert!(validate_notification_url("https://172.16.0.1/webhook")
            .await
            .is_err());
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
        assert!(validate_notification_url("file:///etc/passwd")
            .await
            .is_err());
        assert!(validate_notification_url("ftp://example.com/")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn validate_smtp_host_allows_localhost() {
        // Localhost should be allowed for SMTP (unlike general URL validation)
        let result = validate_smtp_host("localhost").await;
        // DNS may or may not resolve localhost; either way it should be Ok
        assert!(result.is_ok(), "localhost should pass SMTP host validation");
    }

    #[tokio::test]
    async fn validate_smtp_host_allows_literal_loopback() {
        let result = validate_smtp_host("127.0.0.1").await;
        assert!(result.is_ok(), "127.0.0.1 should pass SMTP host validation");
    }

    #[tokio::test]
    async fn check_smtp_ip_allows_private_range() {
        // SMTP host validation intentionally allows private ranges so that
        // internal SMTP relays (e.g., 10.x, 192.168.x, 172.16-31.x) work.
        for literal in ["10.0.0.1", "192.168.1.1", "172.16.0.1"] {
            let ip: std::net::IpAddr = literal.parse().unwrap();
            let result = check_smtp_ip(&ip);
            assert!(
                result.is_ok(),
                "private IP {literal} should be allowed for SMTP: {:?}",
                result.err()
            );
        }
    }

    /// DNS resolution failure for a nonexistent host must return `Ok(())` —
    /// SMTP host validation intentionally does not hard-block on DNS errors
    /// because the SMTP connection itself will fail naturally if the host
    /// is unreachable.
    #[tokio::test]
    async fn validate_smtp_host_dns_failure_returns_ok() {
        let result =
            validate_smtp_host("nonexistent-host-that-should-not-resolve-12345.invalid").await;
        assert!(
            result.is_ok(),
            "DNS failure should not block SMTP host validation"
        );
    }

    #[tokio::test]
    async fn check_smtp_ip_rejects_cloud_metadata() {
        let ip: std::net::IpAddr = "169.254.169.254".parse().unwrap();
        let result = check_smtp_ip(&ip);
        assert!(
            result.is_err(),
            "cloud metadata IP should be rejected even for SMTP"
        );
    }
}
