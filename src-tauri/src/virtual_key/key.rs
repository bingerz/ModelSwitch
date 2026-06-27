//! Virtual API keys with budget tracking for multi-agent spend isolation.

use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::channel::matches_glob;

/// A virtual API key with budget limits and spend tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualKey {
    pub id: Uuid,
    /// SHA-256 hex hash of the plaintext key. Plaintext is shown only at creation time.
    pub key_hash: String,
    /// Key prefix for identification (first 16 chars of plaintext, for display).
    pub key_prefix: String,
    pub name: String,
    /// Daily budget in cents (USD). None = unlimited.
    #[serde(default)]
    pub daily_budget_cents: Option<u64>,
    /// Monthly budget in cents (USD). None = unlimited.
    #[serde(default)]
    pub monthly_budget_cents: Option<u64>,
    pub enabled: bool,
    pub created_at: chrono::DateTime<Utc>,
    /// Spend tracking (mutable, updated by accumulate_spend).
    #[serde(default)]
    pub spend: VirtualKeySpend,
    /// Allowed models. None = all models allowed. Some(vec) = only these models.
    /// Supports glob matching: "gpt-4*" matches "gpt-4", "gpt-4o", "gpt-4-turbo", etc.
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    /// Denied models. Any model matching a pattern in this list is blocked,
    /// even if it would otherwise be allowed. Uses glob matching.
    #[serde(default)]
    pub denied_models: Vec<String>,
    /// IP allowlist for this key. Empty = allow all IPs.
    /// Supports exact IPs ("192.168.1.5") and IPv4 CIDR ranges ("192.168.1.0/24").
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    /// Per-key requests-per-minute limit. None = no per-key RPM limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub rpm_limit: Option<u32>,
    /// Per-key tokens-per-minute limit. None = no per-key TPM limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub tpm_limit: Option<u32>,
    /// Optional expiry timestamp. When set and the current time is past it,
    /// the key is treated as invalid (validate returns None).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional department/group label for aggregating spend by team.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VirtualKeySpend {
    /// Spend today in cents, with the date it tracks.
    #[serde(default)]
    pub today: DailySpend,
    /// Spend this month in cents, with the month it tracks.
    #[serde(default)]
    pub this_month: MonthlySpend,
    /// All-time total spend in cents.
    #[serde(default)]
    pub total_cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DailySpend {
    /// YYYY-MM-DD in the gateway's local timezone.
    pub date: String,
    pub cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MonthlySpend {
    /// YYYY-MM in the gateway's local timezone.
    pub month: String,
    pub cents: u64,
}

impl VirtualKey {
    /// Check if this key has exceeded its budget. Stale spend periods (where
    /// the stored date/month does not match the current local date/month) are
    /// treated as zero, so a new day/month automatically resets the cap.
    pub fn is_budget_exceeded(&self) -> bool {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();

        let daily_spend = if self.spend.today.date == today {
            self.spend.today.cents
        } else {
            0
        };
        let monthly_spend = if self.spend.this_month.month == this_month {
            self.spend.this_month.cents
        } else {
            0
        };

        if let Some(daily_limit) = self.daily_budget_cents {
            if daily_spend >= daily_limit {
                return true;
            }
        }
        if let Some(monthly_limit) = self.monthly_budget_cents {
            if monthly_spend >= monthly_limit {
                return true;
            }
        }
        false
    }

    /// Check if a model is allowed for this key.
    /// If allowed_models is None, all models are permitted.
    /// If allowed_models is Some, the model must match one of the entries
    /// via prefix matching (backward compat, e.g., "gpt-4" matches "gpt-4o")
    /// or glob matching (e.g., "gpt-4*" matches "gpt-4o").
    pub fn is_model_allowed(&self, model: &str) -> bool {
        match &self.allowed_models {
            None => true,
            Some(allowed) => allowed
                .iter()
                .any(|pattern| model.starts_with(pattern) || matches_glob(pattern, model)),
        }
    }

    /// Check if a model is explicitly denied for this key.
    /// Uses glob matching only against the `denied_models` patterns.
    /// To deny a model family, use explicit glob patterns like `gpt-4*`.
    /// An empty list denies nothing.
    pub fn is_model_denied(&self, model: &str) -> bool {
        self.denied_models
            .iter()
            .any(|pattern| matches_glob(pattern, model))
    }

    /// Check if an IP address is allowed to use this key.
    /// If `allowed_ips` is empty, all IPs are permitted.
    /// Otherwise the IP must match an exact entry or fall within a CIDR range.
    pub fn check_ip_allowed(&self, ip: &str) -> bool {
        if self.allowed_ips.is_empty() {
            return true;
        }
        for allowed in &self.allowed_ips {
            if allowed == ip {
                return true;
            }
            if allowed.contains('/') && ip_in_cidr(ip, allowed) {
                return true;
            }
        }
        false
    }

    /// Check if this key has expired. Returns true when `expires_at` is set
    /// and the current time is past it. Returns false when no expiry is set.
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() > exp,
            None => false,
        }
    }
}

/// Check if an IPv4 address falls within the given CIDR block (e.g., "192.168.1.0/24").
/// Returns false on any parse error or non-IPv4 input (fail closed for invalid CIDR).
fn ip_in_cidr(ip: &str, cidr: &str) -> bool {
    let (network, bits) = match cidr.split_once('/') {
        Some((n, b)) => (n, b),
        None => return false,
    };
    let bits: u8 = match bits.parse() {
        Ok(b) if b <= 32 => b,
        _ => return false,
    };
    let ip_u32: u32 = match parse_ipv4(ip) {
        Some(v) => v,
        None => return false,
    };
    let net_u32: u32 = match parse_ipv4(network) {
        Some(v) => v,
        None => return false,
    };
    if bits == 0 {
        return true;
    }
    let mask: u32 = !0u32 << (32 - bits);
    (ip_u32 & mask) == (net_u32 & mask)
}

/// Parse a dotted-quad IPv4 string into a u32. Returns None on malformed input.
fn parse_ipv4(s: &str) -> Option<u32> {
    let octets: Vec<&str> = s.split('.').collect();
    if octets.len() != 4 {
        return None;
    }
    let mut result: u32 = 0;
    for oct in octets {
        let v: u32 = oct.parse().ok()?;
        if v > 255 {
            return None;
        }
        result = (result << 8) | v;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_budget_exceeded_daily() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: Some(100),
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend {
                today: DailySpend {
                    date: today,
                    cents: 100,
                },
                this_month: MonthlySpend {
                    month: this_month,
                    cents: 100,
                },
                total_cents: 100,
            },
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.is_budget_exceeded());
    }

    #[test]
    fn is_budget_exceeded_monthly() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: Some(500),
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend {
                today: DailySpend {
                    date: today,
                    cents: 50,
                },
                this_month: MonthlySpend {
                    month: this_month,
                    cents: 500,
                },
                total_cents: 500,
            },
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.is_budget_exceeded());
    }

    #[test]
    fn is_budget_exceeded_unlimited_when_none() {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let this_month = Local::now().format("%Y-%m").to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend {
                today: DailySpend {
                    date: today,
                    cents: 1_000_000,
                },
                this_month: MonthlySpend {
                    month: this_month,
                    cents: 1_000_000,
                },
                total_cents: 1_000_000,
            },
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(!vk.is_budget_exceeded());
    }

    #[test]
    fn is_model_allowed_permits_all_when_none() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.is_model_allowed("gpt-4"));
        assert!(vk.is_model_allowed("claude-3"));
    }

    #[test]
    fn is_model_allowed_restricts_to_whitelist() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: Some(vec!["gpt-4".to_string(), "claude-3".to_string()]),
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.is_model_allowed("gpt-4"));
        assert!(vk.is_model_allowed("gpt-4o"));
        assert!(vk.is_model_allowed("gpt-4-turbo"));
        assert!(vk.is_model_allowed("claude-3"));
        assert!(vk.is_model_allowed("claude-3-opus"));
        assert!(!vk.is_model_allowed("gemini-pro"));
        assert!(!vk.is_model_allowed("llama-2"));
    }

    #[test]
    fn is_model_denied_exact_match() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec!["gpt-4".to_string()],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.is_model_denied("gpt-4"));
        // Prefix matching must NOT apply to denylist — only glob
        assert!(!vk.is_model_denied("gpt-4o"));
        assert!(!vk.is_model_denied("gpt-4-turbo"));
    }

    #[test]
    fn is_model_denied_glob_pattern() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec!["gpt-4*".to_string()],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.is_model_denied("gpt-4"));
        assert!(vk.is_model_denied("gpt-4o"));
        assert!(vk.is_model_denied("gpt-4-turbo"));
        assert!(!vk.is_model_denied("gpt-3.5"));
    }

    #[test]
    fn is_model_denied_empty_list_denies_nothing() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(!vk.is_model_denied("gpt-4"));
        assert!(!vk.is_model_denied("claude-3"));
    }

    #[test]
    fn ip_allowed_when_no_restriction() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.check_ip_allowed("192.168.1.1"));
        assert!(vk.check_ip_allowed("10.0.0.1"));
        assert!(vk.check_ip_allowed("127.0.0.1"));
    }

    #[test]
    fn ip_allowed_when_exact_match() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec!["192.168.1.5".to_string(), "10.0.0.3".to_string()],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.check_ip_allowed("192.168.1.5"));
        assert!(vk.check_ip_allowed("10.0.0.3"));
    }

    #[test]
    fn ip_denied_when_not_in_list() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec!["192.168.1.5".to_string(), "10.0.0.3".to_string()],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(!vk.check_ip_allowed("192.168.1.6"));
        assert!(!vk.check_ip_allowed("10.0.0.4"));
        assert!(!vk.check_ip_allowed("172.16.0.1"));
    }

    #[test]
    fn ip_allowed_within_cidr() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec!["192.168.1.0/24".to_string()],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(vk.check_ip_allowed("192.168.1.0"));
        assert!(vk.check_ip_allowed("192.168.1.1"));
        assert!(vk.check_ip_allowed("192.168.1.127"));
        assert!(vk.check_ip_allowed("192.168.1.255"));
    }

    #[test]
    fn ip_denied_outside_cidr() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "deadbeef".to_string(),
            key_prefix: "ms-vk-abcdef".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec!["192.168.1.0/24".to_string()],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(!vk.check_ip_allowed("192.168.2.1"));
        assert!(!vk.check_ip_allowed("10.0.0.1"));
        assert!(!vk.check_ip_allowed("192.169.1.1"));
    }

    #[test]
    fn is_expired_returns_false_when_none() {
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "x".to_string(),
            key_prefix: "ms-vk-x".to_string(),
            name: "t".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };
        assert!(!vk.is_expired());
    }

    #[test]
    fn is_expired_boundary_check() {
        // `is_expired` uses a strict `Utc::now() > exp` comparison, so a key
        // whose `expires_at` equals `now` exactly is NOT considered expired.
        // Wall-clock time advances between capture and the assertion, so the
        // test uses a 1-second margin on each side rather than the exact
        // equality case, which would be flaky.
        let mut vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: "x".to_string(),
            key_prefix: "ms-vk-x".to_string(),
            name: "boundary".to_string(),
            daily_budget_cents: None,
            monthly_budget_cents: None,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models: None,
            denied_models: vec![],
            allowed_ips: vec![],
            rpm_limit: None,
            tpm_limit: None,
            expires_at: None,
            group: None,
        };

        // 1 second in the past → expired
        vk.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        assert!(
            vk.is_expired(),
            "key with expires_at 1 second in the past must be expired"
        );

        // 5 seconds in the future → not expired
        vk.expires_at = Some(Utc::now() + chrono::Duration::seconds(5));
        assert!(
            !vk.is_expired(),
            "key with expires_at 5 seconds in the future must not be expired"
        );

        // Boundary documentation: `expires_at == now` returns false (not expired)
        // because the comparison is strict `>`. We do not assert equality
        // directly because wall-clock time advances between the assignment and
        // the check.
    }
}
