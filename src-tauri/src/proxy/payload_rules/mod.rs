//! Payload rules for modifying outgoing request payloads per channel.
//!
//! This module provides:
//! - JSON path utilities (`get_path`, `set_path`, `delete_path`, `path_exists`)
//! - Channel-level rules (`PayloadRules`) applied to every request
//! - Per-model rules (`ModelPayloadRule`) with optional protocol/model matching
//! - A thread-safe registry (`ChannelPayloadRules`) for storing rules keyed by
//!   channel id, with combined application via `apply_for_model`.

mod path;
mod store;
mod types;

pub use store::{ChannelPayloadRules, ChannelRules};
pub use types::{ModelPayloadRule, PayloadRules};
