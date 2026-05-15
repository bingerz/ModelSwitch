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
pub fn create_credential_store() -> SharedCredentialStore {
    tracing::info!("Using file-based credential store");
    Arc::new(file_store::FileCredentialStore::new())
}
