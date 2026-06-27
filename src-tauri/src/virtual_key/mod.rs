//! Virtual API keys with budget tracking for multi-agent spend isolation.
//!
//! A virtual key wraps the gateway's upstream credentials so that downstream
//! clients (agents, scripts, IDEs) can be issued per-key credentials with
//! their own daily/monthly spend caps. When at least one virtual key exists,
//! the proxy enforces that incoming requests carry a valid key; otherwise
//! the gateway behaves exactly as before (open proxy keyed on channel creds).

mod key;
mod spend;
mod store;

pub use key::{DailySpend, MonthlySpend, VirtualKey, VirtualKeySpend};
pub use spend::ReserveResult;
pub use store::{persistence_path, SharedVirtualKeyStore, VirtualKeyStore};