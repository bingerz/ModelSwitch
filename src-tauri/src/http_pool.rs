use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Maximum concurrent HTTP/2 streams per client connection.
/// HTTP/2 servers typically limit this to 100. When the pool client
/// is at capacity, hyper queues the request internally, adding latency.
/// Pool capacity = pool_size × HTTP2_MAX_CONCURRENT_STREAMS.
#[allow(dead_code)] // Documentation constant for pool capacity calculations
const HTTP2_MAX_CONCURRENT_STREAMS: u8 = 100;

/// A pool of `reqwest::Client` instances to work around HTTP/2 single-connection-per-host limits.
///
/// Each client maintains its own TCP connection per host. HTTP/2 multiplexes
/// requests over each connection but caps concurrent streams at ~100. Pool
/// capacity = pool_size × 100 concurrent upstream requests. Use `get()` for
/// least-busy selection across the pool.
///
/// Uses least-busy selection: each client tracks its active request count via an
/// `Arc<AtomicU8>`. `get()` returns an owned `PooledClient` guard that increments
/// the count on creation and decrements on drop, ensuring the count accurately
/// reflects in-flight requests. The guard can be moved into async tasks and
/// spawned futures, so the count stays accurate even for long-lived streaming
/// responses whose bodies continue flowing after the dispatch function returns.
/// Selection picks the client with the lowest active count.
pub struct HttpPool {
    clients: Vec<ClientEntry>,
}

struct ClientEntry {
    client: reqwest::Client,
    active: Arc<AtomicU8>,
}

impl Clone for ClientEntry {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            active: Arc::clone(&self.active),
        }
    }
}

/// Owned RAII guard that tracks an in-flight request on a pooled client.
/// Increments `active` on creation, decrements on drop.
/// Derefs to `reqwest::Client` for transparent use.
///
/// Because the guard is owned (not borrowed), it can be moved into async
/// tasks and spawned futures — the active count is decremented only when
/// the guard is actually dropped, i.e. when the response body/stream is
/// fully consumed.
pub struct PooledClient {
    client: reqwest::Client,
    active: Arc<AtomicU8>,
}

impl std::ops::Deref for PooledClient {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl Drop for PooledClient {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Release);
    }
}

impl Clone for HttpPool {
    fn clone(&self) -> Self {
        Self {
            clients: self.clients.clone(),
        }
    }
}

impl HttpPool {
    /// Create a pool of `size` identical reqwest clients built from the same builder configuration.
    pub fn new<F>(size: usize, build_fn: F) -> reqwest::Result<Self>
    where
        F: Fn() -> reqwest::ClientBuilder,
    {
        let clients = (0..size)
            .map(|_| {
                Ok(ClientEntry {
                    client: build_fn().build()?,
                    active: Arc::new(AtomicU8::new(0)),
                })
            })
            .collect::<reqwest::Result<Vec<_>>>()?;
        Ok(Self { clients })
    }

    /// Get the least-busy client with an owned RAII guard.
    /// The active count is decremented when the guard is dropped.
    pub fn get(&self) -> PooledClient {
        let idx = self.least_busy_index();
        self.clients[idx].active.fetch_add(1, Ordering::AcqRel);
        PooledClient {
            client: self.clients[idx].client.clone(),
            active: Arc::clone(&self.clients[idx].active),
        }
    }

    /// Return the first client, suitable for low-frequency background tasks.
    pub fn first(&self) -> PooledClient {
        self.clients[0].active.fetch_add(1, Ordering::AcqRel);
        PooledClient {
            client: self.clients[0].client.clone(),
            active: Arc::clone(&self.clients[0].active),
        }
    }

    fn least_busy_index(&self) -> usize {
        let mut best_idx = 0;
        let mut best_count = u8::MAX;
        for (i, entry) in self.clients.iter().enumerate() {
            let count = entry.active.load(Ordering::Acquire);
            if count < best_count {
                best_count = count;
                best_idx = i;
            }
        }
        best_idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_returns_least_busy_client() {
        let pool = HttpPool::new(3, reqwest::Client::builder).unwrap();
        // All start at 0 — first get picks index 0
        let _c1 = pool.get();
        // Index 0 has 1 active, pick index 1
        let c2 = pool.get();
        // Index 0,1 have 1, pick index 2
        let c3 = pool.get();
        // All have 1, pick 0 again
        let _c4 = pool.get();

        // _c4 should be from index 0 (all at 1, first wins)
        // Drop c2 and c3 to free their slots
        drop(c2);
        drop(c3);
        // Now index 1 and 2 have 0 active
        let _c5 = pool.get();
        // Should pick index 1 (first with 0)

        // Verify by checking active counts
        assert_eq!(
            pool.clients[0].active.load(Ordering::Acquire),
            2,
            "index 0 has _c1+_c4"
        );
        assert_eq!(
            pool.clients[1].active.load(Ordering::Acquire),
            1,
            "index 1 has _c5"
        );
        assert_eq!(
            pool.clients[2].active.load(Ordering::Acquire),
            0,
            "index 2 is free"
        );
    }

    #[test]
    fn pool_drops_decrement_active() {
        let pool = HttpPool::new(2, reqwest::Client::builder).unwrap();
        {
            let _guard = pool.get();
            assert_eq!(pool.clients[0].active.load(Ordering::Acquire), 1);
            // Client 0 has 1 active, next get picks client 1
            let _c2 = pool.get();
            assert_eq!(pool.clients[1].active.load(Ordering::Acquire), 1);
        }
        // After drop, all clients should be at 0 again
        assert_eq!(
            pool.clients[0].active.load(Ordering::Acquire),
            0,
            "guard drop should decrement index 0"
        );
        assert_eq!(
            pool.clients[1].active.load(Ordering::Acquire),
            0,
            "guard drop should decrement index 1"
        );
    }

    #[test]
    fn pool_single_client_works() {
        let pool = HttpPool::new(1, reqwest::Client::builder).unwrap();
        let _c1 = pool.get();
        let _c2 = pool.get();
        assert_eq!(
            pool.clients[0].active.load(Ordering::Acquire),
            2,
            "both guards on same client"
        );
    }

    #[test]
    fn pool_first_increments_active() {
        let pool = HttpPool::new(3, reqwest::Client::builder).unwrap();
        let _first = pool.first();
        assert_eq!(pool.clients[0].active.load(Ordering::Acquire), 1);
        // get() should pick index 1 (0 has active=1)
        let _other = pool.get();
        assert_eq!(pool.clients[1].active.load(Ordering::Acquire), 1);
    }

    #[test]
    fn pool_clone_preserves_active_state() {
        let pool = HttpPool::new(2, reqwest::Client::builder).unwrap();
        let _guard = pool.get();
        let cloned = pool.clone();
        // Clone shares Arc<AtomicU8>, so sees active=1
        assert_eq!(
            cloned.clients[0].active.load(Ordering::Acquire),
            1,
            "clone shares active count via Arc"
        );
        // get() on clone picks index 1
        let _c = cloned.get();
        assert_eq!(
            cloned.clients[1].active.load(Ordering::Acquire),
            1,
            "clone's get() should pick least-busy (index 1)"
        );
    }
}
