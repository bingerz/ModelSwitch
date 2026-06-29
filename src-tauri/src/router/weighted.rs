use crate::channel::Channel;
use rand::Rng;

/// Weighted random selection from a list of channels within the same priority.
pub fn weighted_random(candidates: &[Channel]) -> Option<Channel> {
    if candidates.is_empty() {
        return None;
    }

    let total_weight: u64 = candidates.iter().map(|c| c.weight as u64).sum();
    if total_weight == 0 {
        // Fallback: uniform random if all weights are zero
        let idx = rand::rng().random_range(0..candidates.len());
        return Some(candidates[idx].clone());
    }

    let mut roll = rand::rng().random_range(1..=total_weight);
    for ch in candidates {
        if roll <= ch.weight as u64 {
            return Some(ch.clone());
        }
        roll -= ch.weight as u64;
    }

    candidates.last().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelStatus, Credential, CredentialType, Provider};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_channel(name: &str, weight: u32, priority: u8) -> Channel {
        Channel {
            id: Uuid::new_v4(),
            name: name.to_string(),
            provider: Provider::OpenAI,
            priority,
            weight,
            cost_per_token: None,
            input_cost_per_mtok: None,
            output_cost_per_mtok: None,
            credential: Credential {
                cred_type: CredentialType::ApiKey,
                key_ref: format!("key_{}", name),
                api_key: None,
                expires_at: None,
            },
            enabled: true,
            status: ChannelStatus::Healthy,
            circuit_open_until: None,
            base_url: "https://api.openai.com".to_string(),
            model_mapping: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            avg_latency_ms: 0,
            consecutive_failures: 0,
            cooldown_minutes: None,
            rpm_limit: None,
            tpm_limit: None,
            account_group: None,
            max_concurrent: None,
            api_keys: vec![],
            excluded_models: vec![],
            model_cooldowns: HashMap::new(),
            proxy_url: None,
            headers: HashMap::new(),
            max_retries: None,
            models_endpoint: None,
            models_refresh_interval_secs: 300,
            tags: vec![],
            quota: None,
        }
    }

    #[test]
    fn test_weighted_random_single() {
        let channels = vec![make_channel("a", 100, 1)];
        let result = weighted_random(&channels);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "a");
    }

    #[test]
    fn test_weighted_random_distribution() {
        let channels = vec![make_channel("heavy", 900, 1), make_channel("light", 100, 1)];

        let mut heavy_count = 0u32;
        let iterations = 1000u32;
        for _ in 0..iterations {
            if let Some(ch) = weighted_random(&channels) {
                if ch.name == "heavy" {
                    heavy_count += 1;
                }
            }
        }

        // Should be roughly 90% heavy
        assert!(heavy_count > 800, "heavy_count was {heavy_count}");
        assert!(heavy_count < 980, "heavy_count was {heavy_count}");
    }

    #[test]
    fn test_weighted_random_empty() {
        let channels: Vec<Channel> = vec![];
        assert!(weighted_random(&channels).is_none());
    }

    #[test]
    fn test_weighted_random_zero_weights() {
        let channels = vec![make_channel("a", 0, 1), make_channel("b", 0, 1)];
        let result = weighted_random(&channels);
        assert!(result.is_some());
    }
}
