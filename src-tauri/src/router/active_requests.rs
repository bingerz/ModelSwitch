use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Tracks active request counts per channel for least-busy routing.
pub struct ActiveRequests {
    counts: Arc<Mutex<HashMap<Uuid, Arc<AtomicU32>>>>,
}

impl Clone for ActiveRequests {
    fn clone(&self) -> Self {
        Self {
            counts: Arc::clone(&self.counts),
        }
    }
}

impl ActiveRequests {
    pub fn new() -> Self {
        Self {
            counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Increment the active request count for a channel.
    pub fn increment(&self, channel_id: Uuid) {
        let mut guard = self.counts.lock().unwrap();
        guard
            .entry(channel_id)
            .or_insert_with(|| Arc::new(AtomicU32::new(0)))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the active request count for a channel.
    /// Prevents underflow past 0 using a CAS loop.
    pub fn decrement(&self, channel_id: Uuid) {
        let guard = self.counts.lock().unwrap();
        if let Some(count) = guard.get(&channel_id) {
            loop {
                let current = count.load(Ordering::Acquire);
                if current == 0 {
                    tracing::warn!(channel = %channel_id, "Active request counter underflow prevented");
                    break;
                }
                if count
                    .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break;
                }
            }
        }
    }

    /// Get the current active request count for a channel.
    pub fn get(&self, channel_id: Uuid) -> u32 {
        let guard = self.counts.lock().unwrap();
        guard
            .get(&channel_id)
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Get all active request counts as a snapshot.
    pub fn snapshot(&self) -> HashMap<Uuid, u32> {
        let guard = self.counts.lock().unwrap();
        guard
            .iter()
            .map(|(id, count)| (*id, count.load(Ordering::Relaxed)))
            .collect()
    }
}

impl Default for ActiveRequests {
    fn default() -> Self {
        Self::new()
    }
}
