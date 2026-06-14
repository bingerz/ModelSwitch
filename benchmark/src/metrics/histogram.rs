use std::time::Duration;

/// Latency histogram backed by hdrhistogram with nanosecond precision.
pub struct LatencyHistogram {
    hist: hdrhistogram::Histogram<u64>,
}

impl LatencyHistogram {
    pub fn new() -> Self {
        Self {
            hist: hdrhistogram::Histogram::<u64>::new_with_bounds(1, 300_000_000_000, 3)
                .expect("failed to create histogram"),
        }
    }

    /// Record a latency duration.
    pub fn record(&mut self, duration: Duration) {
        let ns = duration.as_nanos().min(u64::MAX as u128) as u64;
        let _ = self.hist.record(ns);
    }

    /// Return the percentile value in milliseconds.
    pub fn percentile_ms(&self, p: f64) -> u64 {
        self.hist.value_at_quantile(p) / 1_000_000
    }

    pub fn min_ms(&self) -> u64 {
        self.hist.min() / 1_000_000
    }

    pub fn max_ms(&self) -> u64 {
        self.hist.max() / 1_000_000
    }

    pub fn mean_ms(&self) -> f64 {
        self.hist.mean() / 1_000_000.0
    }

    pub fn count(&self) -> u64 {
        self.hist.len()
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}
