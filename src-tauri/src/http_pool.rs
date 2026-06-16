use std::sync::atomic::{AtomicUsize, Ordering};

/// A pool of `reqwest::Client` instances to work around HTTP/2 single-connection-per-host limits.
///
/// `reqwest`/`hyper` opens only one TCP connection per HTTP/2 host:port pair. When concurrent
/// requests exceed `max_concurrent_streams` (default 100), requests stall. Round-robin selection
/// across multiple clients spreads concurrent requests across multiple TCP connections.
pub struct HttpPool {
    clients: Vec<reqwest::Client>,
    index: AtomicUsize,
}

impl Clone for HttpPool {
    fn clone(&self) -> Self {
        Self {
            clients: self.clients.clone(),
            index: AtomicUsize::new(self.index.load(Ordering::Relaxed)),
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
            .map(|_| build_fn().build())
            .collect::<reqwest::Result<Vec<_>>>()?;
        Ok(Self {
            clients,
            index: AtomicUsize::new(0),
        })
    }

    /// Get the next client via round-robin. Thread-safe.
    pub fn get(&self) -> &reqwest::Client {
        let idx = self.index.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        &self.clients[idx]
    }

    /// Return a reference to the first client, suitable for low-frequency background tasks
    /// where round-robin distribution is unnecessary.
    pub fn first(&self) -> &reqwest::Client {
        &self.clients[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_round_robin_cycles() {
        let pool = HttpPool::new(3, reqwest::Client::builder).unwrap();
        assert_eq!(pool.clients.len(), 3);

        // Round-robin should cycle through indices 0, 1, 2, 0, 1, 2, ...
        let ptr0 = pool.get() as *const _;
        let ptr1 = pool.get() as *const _;
        let ptr2 = pool.get() as *const _;
        let ptr3 = pool.get() as *const _;

        assert_ne!(ptr0, ptr1);
        assert_ne!(ptr1, ptr2);
        assert_eq!(ptr0, ptr3, "fourth call should wrap to first client");
    }

    #[test]
    fn pool_single_client_works() {
        let pool = HttpPool::new(1, reqwest::Client::builder).unwrap();
        let c1 = pool.get() as *const _;
        let c2 = pool.get() as *const _;
        assert_eq!(c1, c2, "single-client pool always returns the same client");
    }

    #[test]
    fn pool_first_returns_first_client() {
        let pool = HttpPool::new(3, reqwest::Client::builder).unwrap();
        let first_ptr = pool.first() as *const _;
        let get_ptr = pool.get() as *const _;
        assert_eq!(first_ptr, get_ptr);
    }
}
