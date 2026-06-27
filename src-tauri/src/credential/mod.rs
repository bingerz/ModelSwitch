pub mod crypto;
pub mod file_store;

use std::sync::Arc;

/// Abstraction over credential storage backends.
pub trait CredentialStore: Send + Sync {
    fn get(&self, service: &str, username: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, service: &str, username: &str, password: &str) -> anyhow::Result<()>;
    fn delete(&self, service: &str, username: &str) -> anyhow::Result<()>;
}

pub type SharedCredentialStore = Arc<dyn CredentialStore>;

/// Create the file-based credential store.
///
/// When `admin_token` is provided, credentials are encrypted at rest with
/// AES-256-GCM. When `None`, credentials are stored in plaintext (with a
/// startup warning).
pub fn create_credential_store(admin_token: Option<&str>) -> SharedCredentialStore {
    if let Some(token) = admin_token {
        tracing::info!("Using file-based credential store with AES-256-GCM encryption at rest");
        Arc::new(file_store::FileCredentialStore::with_encryption(token))
    } else {
        tracing::warn!("No admin_token set — credentials stored in plaintext without encryption");
        Arc::new(file_store::FileCredentialStore::new())
    }
}
