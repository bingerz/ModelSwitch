use std::sync::atomic::{AtomicU8, Ordering};

/// A pool of `reqwest::Client` instances to work around HTTP/2 single-connection-per-host limits.
///
/// Uses least-busy selection: each client tracks its active request count via an
/// `AtomicU8`. `get()` returns a `PooledClient` guard that increments the count
/// on creation and decrements on drop, ensuring the count accurately reflects
/// in-flight requests. Selection picks the client with the lowest active count.
pub struct HttpPool {
    clients: Vec<ClientEntry>,
}

struct ClientEntry {
    client: reqwest::Client,
    active: AtomicU8,
}

impl Clone for ClientEntry {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            active: AtomicU8::new(self.active.load(Ordering::Relaxed)),
        }
    }
}

/// RAII guard that tracks an in-flight request on a pooled client.
/// Increments `active` on creation, decrements on drop.
/// Derefs to `reqwest::Client` for transparent use.
pub struct PooledClient<'a> {
    entry: &'a ClientEntry,
}

impl std::ops::Deref for PooledClient<'_> {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target {
        &self.entry.client
    }
}

impl Drop for PooledClient<'_> {
    fn drop(&mut self) {
        self.entry.active.fetch_sub(1, Ordering::Release);
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
                    active: AtomicU8::new(0),
                })
            })
            .collect::<reqwest::Result<Vec<_>>>()?;
        Ok(Self { clients })
    }

    /// Get the least-busy client with an RAII guard.
    /// The active count is decremented when the guard is dropped.
    pub fn get(&self) -> PooledClient<'_> {
        let idx = self.least_busy_index();
        self.clients[idx].active.fetch_add(1, Ordering::AcqRel);
        PooledClient {
            entry: &self.clients[idx],
        }
    }

    /// Return the first client, suitable for low-frequency background tasks.
    pub fn first(&self) -> PooledClient<'_> {
        self.clients[0].active.fetch_add(1, Ordering::AcqRel);
        PooledClient {
            entry: &self.clients[0],
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
        // All clients start at 0 active -- first call returns index 0
        let c1 = pool.get();
        // Client 0 now has 1 active, clients 1 and 2 have 0 -- next call picks 1
        let c2 = pool.get();
        // Client 0 has 1, client 1 has 1, client 2 has 0 -- picks 2
        let c3 = pool.get();
        // All have 1 active -- picks 0 (first with min count)
        let c4 = pool.get();

        // c1 and c4 should be the same client (both index 0)
        assert!(std::ptr::eq(&*c1, &*c4), "c1 and c4 should be same client");
        assert!(!std::ptr::eq(&*c1, &*c2), "c1 and c2 should differ");
        assert!(!std::ptr::eq(&*c1, &*c3), "c1 and c3 should differ");
    }

    #[test]
    fn pool_drops_decrement_active() {
        let pool = HttpPool::new(2, reqwest::Client::builder).unwrap();

        {
            let _guard = pool.get();
            // Client 0 has 1 active, next get picks client 1
            let _c2 = pool.get();
        }
        // After drop, all clients should be at 0 again
        // Next get should pick index 0
        let c_final = pool.get();
        assert!(std::ptr::eq(&*c_final, &pool.clients[0].client));
    }

    #[test]
    fn pool_single_client_works() {
        let pool = HttpPool::new(1, reqwest::Client::builder).unwrap();
        let c1 = pool.get();
        let c2 = pool.get();
        assert!(
            std::ptr::eq(&*c1, &*c2),
            "single-client pool always returns same client"
        );
    }

    #[test]
    fn pool_first_returns_first_client() {
        let pool = HttpPool::new(3, reqwest::Client::builder).unwrap();
        let first_guard = pool.first();
        let get_guard = pool.get();
        // first() took index 0 (active=1), get() picks index 1 (active=0)
        assert!(
            !std::ptr::eq(&*first_guard, &*get_guard),
            "get() should pick least-busy (index 1) not first (index 0 with active=1)"
        );
    }

    #[test]
    fn pool_clone_preserves_state() {
        let pool = HttpPool::new(2, reqwest::Client::builder).unwrap();
        let _guard = pool.get();
        let cloned = pool.clone();
        // Clone should see updated active counts since Vec<ClientEntry> is cloned
        // (AtomicU8 values are copied)
        // After clone, index 0 has 1 active in the clone too
        let c = cloned.get();
        // Should pick index 1 (0 active) over index 0 (1 active)
        assert!(std::ptr::eq(&*c, &cloned.clients[1].client));
    }
}
