pub mod audit;
pub mod auth;
pub mod channels;
pub mod features;
pub mod mcp;
pub mod portal;
pub mod provider_budgets;
pub mod system;
pub mod virtual_keys;

use serde::Deserialize;

// ─── Shared Types ──────────────────────────────────────

/// Unified API response envelope.
#[derive(serde::Serialize)]
pub struct ApiResponse<T: serde::Serialize> {
    pub ok: bool,
    pub data: T,
}

impl<T: serde::Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { ok: true, data }
    }
}

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_offset")]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_offset() -> usize {
    0
}
fn default_limit() -> usize {
    50
}

// ─── Re-exports ────────────────────────────────────────

pub use audit::*;
pub use auth::*;
pub use channels::*;
pub use features::*;
pub use mcp::*;
pub use provider_budgets::*;
pub use system::*;
pub use virtual_keys::*;

/// Paginated response wrapper with total count.
#[derive(serde::Serialize)]
pub struct PaginatedResponse<T: serde::Serialize> {
    pub data: T,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}
