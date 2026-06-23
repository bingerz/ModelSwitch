use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Tracks active request counts per channel for least-busy routing.
pub struct ActiveRequests {
    counts: Mutex<HashMap<Uuid, u32>>,
}

impl ActiveRequests {
    pub fn new() -> Self {
        Self {
            counts: Mutex::new(HashMap::new()),
        }
    }

    /// Increment the active request count for a channel.
    pub fn increment(&self, channel_id: Uuid) {
        let mut guard = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        *guard.entry(channel_id).or_insert(0) += 1;
    }

    /// Decrement the active request count for a channel.
    pub fn decrement(&self, channel_id: Uuid) {
        let mut guard = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = guard.get_mut(&channel_id) {
            if *count > 0 {
                *count -= 1;
            } else {
                tracing::warn!(channel = %channel_id, "Active request counter underflow prevented");
            }
        }
    }

    /// Increment the active request count and return a guard that decrements on drop.
    /// Call this instead of `increment()` + manual `decrement()`.
    pub fn acquire(self: &Arc<Self>, channel_id: Uuid) -> ActiveRequestGuard {
        self.increment(channel_id);
        ActiveRequestGuard {
            tracker: Arc::clone(self),
            channel_id,
        }
    }

    /// Get the current active request count for a channel.
    pub fn get(&self, channel_id: Uuid) -> u32 {
        let guard = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(&channel_id).copied().unwrap_or(0)
    }

    /// Get all active request counts as a snapshot.
    pub fn snapshot(&self) -> HashMap<Uuid, u32> {
        let guard = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    /// Total active requests across all channels.
    pub fn total(&self) -> u64 {
        let guard = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        guard.values().map(|v| *v as u64).sum()
    }
}

impl Default for ActiveRequests {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard that decrements the active request count on drop.
/// Eliminates manual decrement call sites that can drift.
pub struct ActiveRequestGuard {
    tracker: Arc<ActiveRequests>,
    channel_id: Uuid,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.tracker.decrement(self.channel_id);
    }
}
