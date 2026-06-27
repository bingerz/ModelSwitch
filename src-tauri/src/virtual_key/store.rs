use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::persisted_store::PersistedStore;
use crate::virtual_key::key::{VirtualKey, VirtualKeySpend};

pub struct VirtualKeyStore {
    pub(crate) store: PersistedStore<Uuid, VirtualKey>,
    pub(crate) prefix_index: parking_lot::RwLock<HashMap<String, Uuid>>,
}

pub type SharedVirtualKeyStore = Arc<VirtualKeyStore>;

impl VirtualKeyStore {
    pub fn new() -> Self {
        Self {
            store: PersistedStore::new(persistence_path()),
            prefix_index: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Construct with a custom persistence path (for testing).
    pub fn with_store_path(path: std::path::PathBuf) -> Self {
        Self {
            store: PersistedStore::new(path),
            prefix_index: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// Validate a plaintext key string. Returns the [`VirtualKey`] if it is
    /// valid, enabled, unexpired, and within budget.
    ///
    /// Uses O(1) prefix-index lookup to identify the candidate key, then a
    /// single constant-time hash comparison via the `subtle` crate to confirm
    /// the full key matches. This avoids the O(N) linear scan of all keys.
    pub async fn validate(&self, plaintext: &str) -> Option<VirtualKey> {
        // O(1) lookup: extract prefix → look up UUID → single hash comparison.
        let prefix = plaintext.get(..16).unwrap_or(plaintext);
        let id = {
            let index = self.prefix_index.read();
            index.get(prefix).copied()
        }?;

        let hash = sha256_hex(plaintext);
        let hash_bytes = hash.into_bytes();
        let keys = self.store.read().await;
        let vk = keys.get(&id)?;
        // Single ct_eq comparison to confirm the full key matches.
        let stored = vk.key_hash.as_bytes();
        let matched = stored.len() == hash_bytes.len() && bool::from(stored.ct_eq(&hash_bytes));
        if !matched {
            return None;
        }
        if !vk.enabled {
            return None;
        }
        if vk.is_expired() {
            return None;
        }
        if vk.is_budget_exceeded() {
            return None;
        }
        Some(vk.clone())
    }

    /// Authenticate a plaintext key without enforcing enabled/expired/budget
    /// checks. Returns the [`VirtualKey`] if the hash matches, regardless of
    /// whether the key is currently disabled, expired, or over budget.
    ///
    /// Used by the self-service portal so employees can view their key status
    /// even when the key is inactive.
    pub async fn validate_any(&self, plaintext: &str) -> Option<VirtualKey> {
        let prefix = plaintext.get(..16).unwrap_or(plaintext);
        let id = {
            let index = self.prefix_index.read();
            index.get(prefix).copied()
        }?;

        let hash = sha256_hex(plaintext);
        let hash_bytes = hash.into_bytes();
        let keys = self.store.read().await;
        let vk = keys.get(&id)?;
        let stored = vk.key_hash.as_bytes();
        let matched = stored.len() == hash_bytes.len() && bool::from(stored.ct_eq(&hash_bytes));
        if matched {
            Some(vk.clone())
        } else {
            None
        }
    }

    /// Create a new virtual key. Returns `(VirtualKey, plaintext_key)`.
    /// The plaintext is shown to the user once and never stored.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        name: String,
        daily_budget_cents: Option<u64>,
        monthly_budget_cents: Option<u64>,
        allowed_models: Option<Vec<String>>,
        denied_models: Vec<String>,
        allowed_ips: Vec<String>,
        rpm_limit: Option<u32>,
        tpm_limit: Option<u32>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        group: Option<String>,
    ) -> (VirtualKey, String) {
        let plaintext = format!("ms-vk-{}", Uuid::new_v4().simple());
        let hash = sha256_hex(&plaintext);
        // 16 chars covers "ms-vk-" + 9 chars of uuid — enough to identify a key in the UI.
        let prefix = plaintext[..16].to_string();
        let vk = VirtualKey {
            id: Uuid::new_v4(),
            key_hash: hash,
            key_prefix: prefix,
            name,
            daily_budget_cents,
            monthly_budget_cents,
            enabled: true,
            created_at: Utc::now(),
            spend: VirtualKeySpend::default(),
            allowed_models,
            denied_models,
            allowed_ips,
            rpm_limit,
            tpm_limit,
            expires_at,
            group,
        };
        let vk_clone = vk.clone();
        self.store.write().await.insert(vk.id, vk);
        self.prefix_index
            .write()
            .insert(vk_clone.key_prefix.clone(), vk_clone.id);
        (vk_clone, plaintext)
    }

    pub async fn list(&self) -> Vec<VirtualKey> {
        self.store.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: Uuid) -> Option<VirtualKey> {
        self.store.read().await.get(&id).cloned()
    }

    pub async fn delete(&self, id: Uuid) -> bool {
        let removed = self.store.write().await.remove(&id);
        if let Some(vk) = &removed {
            self.prefix_index.write().remove(&vk.key_prefix);
        }
        removed.is_some()
    }

    /// Update fields on a virtual key. Each `Option<T>` field, when `Some`,
    /// replaces the stored value; `None` leaves it untouched.
    #[allow(clippy::option_option, clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: Uuid,
        name: Option<String>,
        daily_budget_cents: Option<Option<u64>>,
        monthly_budget_cents: Option<Option<u64>>,
        enabled: Option<bool>,
        allowed_models: Option<Option<Vec<String>>>,
        denied_models: Option<Vec<String>>,
        allowed_ips: Option<Vec<String>>,
        rpm_limit: Option<Option<u32>>,
        tpm_limit: Option<Option<u32>>,
        expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
        group: Option<Option<String>>,
    ) -> Option<VirtualKey> {
        let mut keys = self.store.write().await;
        let vk = keys.get_mut(&id)?;
        if let Some(n) = name {
            vk.name = n;
        }
        if let Some(d) = daily_budget_cents {
            vk.daily_budget_cents = d;
        }
        if let Some(m) = monthly_budget_cents {
            vk.monthly_budget_cents = m;
        }
        if let Some(e) = enabled {
            vk.enabled = e;
        }
        if let Some(am) = allowed_models {
            vk.allowed_models = am;
        }
        if let Some(dm) = denied_models {
            vk.denied_models = dm;
        }
        if let Some(ai) = allowed_ips {
            vk.allowed_ips = ai;
        }
        if let Some(rpm) = rpm_limit {
            vk.rpm_limit = rpm;
        }
        if let Some(tpm) = tpm_limit {
            vk.tpm_limit = tpm;
        }
        if let Some(exp) = expires_at {
            vk.expires_at = exp;
        }
        if let Some(g) = group {
            vk.group = g;
        }
        Some(vk.clone())
    }

    /// Returns true if at least one virtual key is configured. The middleware
    /// uses this to decide whether enforcement is active (open proxy vs. gated).
    pub async fn has_keys(&self) -> bool {
        !self.store.read().await.is_empty()
    }

    /// Persist all keys to disk. Best-effort — errors are logged internally.
    pub async fn persist(&self) -> anyhow::Result<()> {
        self.store.persist().await;
        Ok(())
    }

    /// Load keys from disk. A missing file is treated as an empty store.
    /// Rebuilds the prefix index from the loaded data so O(1) prefix lookup
    /// works immediately after startup.
    pub async fn load(&self) -> anyhow::Result<()> {
        self.store.load().await;
        let keys = self.store.read().await;
        let mut index = self.prefix_index.write();
        index.clear();
        for vk in keys.values() {
            index.insert(vk.key_prefix.clone(), vk.id);
        }
        drop(index);
        drop(keys);
        Ok(())
    }

    /// O(1) lookup of a virtual key ID by the prefix of an incoming plaintext key.
    /// Extracts the first 16 chars (matching [`VirtualKey::key_prefix`]) and
    /// checks the in-memory index. Returns `None` if no key with that prefix
    /// exists. This only identifies which key the prefix belongs to — callers
    /// must still verify the full key (e.g. via [`validate`](Self::validate))
    /// before trusting the identity, since prefixes are not secret.
    pub async fn find_by_prefix(&self, key: &str) -> Option<Uuid> {
        let prefix: &str = key.get(..16).unwrap_or(key);
        self.prefix_index.read().get(prefix).copied()
    }
}

impl Default for VirtualKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Return the lowercase hex SHA-256 digest of the input.
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let bytes = hasher.finalize();
    // Manual hex encode to avoid pulling in an extra dependency.
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

/// Resolve the canonical virtual-key persistence path under the user's
/// config dir. Falls back to a relative path if `dirs` cannot resolve.
pub fn persistence_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("modelswitch")
        .join("virtual_keys.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_generates_key_with_ms_vk_prefix() {
        let store = VirtualKeyStore::new();
        let (vk, plaintext) = store
            .create(
                "test".to_string(),
                Some(100),
                Some(1000),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        assert!(plaintext.starts_with("ms-vk-"), "prefix was: {plaintext}");
        assert!(plaintext.len() > "ms-vk-".len() + 8);
        assert!(!plaintext.is_empty());
        assert_eq!(vk.name, "test");
        assert_eq!(vk.daily_budget_cents, Some(100));
        assert_eq!(vk.monthly_budget_cents, Some(1000));
        assert!(vk.enabled);
        assert_eq!(vk.key_prefix.len(), 16);
    }

    #[tokio::test]
    async fn validate_rejects_wrong_key() {
        let store = VirtualKeyStore::new();
        let _ = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        let result = store.validate("ms-vk-wrongkey").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn validate_returns_key_when_correct() {
        let store = VirtualKeyStore::new();
        let (created, plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        let validated = store.validate(&plaintext).await;
        assert!(validated.is_some());
        assert_eq!(validated.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn validate_returns_none_when_disabled() {
        let store = VirtualKeyStore::new();
        let (created, plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store
            .update(
                created.id,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await;
        let validated = store.validate(&plaintext).await;
        assert!(validated.is_none());
    }

    #[tokio::test]
    async fn has_keys_reflects_state() {
        let store = VirtualKeyStore::new();
        assert!(!store.has_keys().await);
        let _ = store
            .create(
                "a".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        assert!(store.has_keys().await);
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create(
                "a".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        assert!(store.delete(vk.id).await);
        assert!(store.get(vk.id).await.is_none());
        assert!(!store.delete(vk.id).await);
    }

    #[tokio::test]
    async fn persist_and_load_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-vk-test-{}.json",
            Uuid::new_v4().simple()
        ));
        // Clean up before and after so a previous panicked run can't poison us.
        let _ = std::fs::remove_file(&path);

        let store = VirtualKeyStore::with_store_path(path.clone());
        let (vk, plaintext) = store
            .create(
                "persisted".to_string(),
                Some(10),
                Some(100),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store.accumulate_spend(vk.id, 5).await;
        store.persist().await.unwrap();
        assert!(path.exists());

        let store2 = VirtualKeyStore::with_store_path(path.clone());
        store2.load().await.unwrap();
        let keys = store2.list().await;
        assert_eq!(keys.len(), 1);
        let loaded = &keys[0];
        assert_eq!(loaded.name, "persisted");
        assert_eq!(loaded.spend.total_cents, 5);
        // Plaintext should still validate after a reload
        let v = store2.validate(&plaintext).await;
        assert!(v.is_some());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn prefix_index_finds_match() {
        let store = VirtualKeyStore::new();
        let (vk, plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        let found = store.find_by_prefix(&plaintext).await;
        assert_eq!(found, Some(vk.id));
    }

    #[tokio::test]
    async fn prefix_index_returns_none_for_unknown() {
        let store = VirtualKeyStore::new();
        let _ = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        // Different prefix — should not match
        let found = store.find_by_prefix("ms-vk-unknownkey").await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn prefix_index_removes_on_delete() {
        let store = VirtualKeyStore::new();
        let (vk, plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        assert!(store.delete(vk.id).await);
        let found = store.find_by_prefix(&plaintext).await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn prefix_index_updates_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "modelswitch-vk-prefix-reload-{}.json",
            Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_file(&path);

        // Create and persist a key with one store
        let store = VirtualKeyStore::with_store_path(path.clone());
        let (vk, plaintext) = store
            .create(
                "persisted".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store.persist().await.unwrap();

        // Fresh store loads from disk — index should be populated
        let store2 = VirtualKeyStore::with_store_path(path.clone());
        store2.load().await.unwrap();
        let found = store2.find_by_prefix(&plaintext).await;
        assert_eq!(found, Some(vk.id));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn validate_rejects_expired_key() {
        let store = VirtualKeyStore::new();
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        let (vk, plaintext) = store
            .create(
                "expired".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                Some(past),
                None,
            )
            .await;
        let _ = vk;
        let result = store.validate(&plaintext).await;
        assert!(result.is_none(), "expired key must not validate");
    }

    #[tokio::test]
    async fn validate_rejects_budget_exceeded_key() {
        let store = VirtualKeyStore::new();
        let (vk, plaintext) = store
            .create(
                "budget".to_string(),
                Some(10),
                Some(100),
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store.accumulate_spend(vk.id, 20).await; // exceeds daily 10 cents
        let result = store.validate(&plaintext).await;
        assert!(result.is_none(), "key over daily budget must not validate");
    }

    #[tokio::test]
    async fn validate_any_works_for_valid_key() {
        let store = VirtualKeyStore::new();
        let (_vk, plaintext) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        let result = store.validate_any(&plaintext).await;
        assert!(
            result.is_some(),
            "validate_any should return Some for valid key"
        );
    }

    #[tokio::test]
    async fn update_changes_rpm_limit() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store
            .update(
                vk.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(Some(100)),
                None,
                None,
                None,
            )
            .await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.rpm_limit, Some(100));
    }

    #[tokio::test]
    async fn update_changes_expires_at() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        let future = chrono::Utc::now() + chrono::Duration::days(30);
        store
            .update(
                vk.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(Some(future)),
                None,
            )
            .await;
        let fetched = store.get(vk.id).await.unwrap();
        assert!(fetched.expires_at.is_some());
    }

    #[tokio::test]
    async fn update_changes_group() {
        let store = VirtualKeyStore::new();
        let (vk, _) = store
            .create(
                "test".to_string(),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                None,
                None,
            )
            .await;
        store
            .update(
                vk.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(Some("engineering".to_string())),
            )
            .await;
        let fetched = store.get(vk.id).await.unwrap();
        assert_eq!(fetched.group.as_deref(), Some("engineering"));
    }

    #[tokio::test]
    async fn create_with_all_fields_populated() {
        let store = VirtualKeyStore::new();
        let future = chrono::Utc::now() + chrono::Duration::days(365);
        let (vk, plaintext) = store
            .create(
                "full".to_string(),
                Some(100),
                Some(1000),
                Some(vec!["gpt-4".to_string()]),
                vec!["gpt-3.5".to_string()],
                vec!["10.0.0.0/8".to_string()],
                Some(60),
                Some(10000),
                Some(future),
                Some("dev-team".to_string()),
            )
            .await;
        assert_eq!(vk.name, "full");
        assert_eq!(vk.daily_budget_cents, Some(100));
        assert_eq!(vk.rpm_limit, Some(60));
        assert_eq!(vk.tpm_limit, Some(10000));
        assert!(vk.expires_at.is_some());
        assert_eq!(vk.group.as_deref(), Some("dev-team"));
        // Verify the key validates
        assert!(store.validate(&plaintext).await.is_some());
    }

    #[test]
    fn sha256_hex_is_stable_and_lowercase() {
        let a = sha256_hex("hello");
        let b = sha256_hex("hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Known SHA-256 of "hello"
        assert_eq!(
            a,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
