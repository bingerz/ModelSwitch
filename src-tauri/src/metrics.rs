//! Prometheus metrics for gateway observability.
//!
//! Metrics are exposed via the `/metrics` endpoint in Prometheus text format.
//! All metrics use lazy `OnceLock` initialization so they are zero-cost until
//! first accessed.

use prometheus::{
    Encoder, Gauge, IntCounter, IntCounterVec, HistogramVec, Opts, Registry, TextEncoder,
};
use std::sync::OnceLock;

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Returns the global Prometheus registry.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::new)
}

static REQUESTS_TOTAL: OnceLock<IntCounterVec> = OnceLock::new();
static REQUEST_DURATION: OnceLock<HistogramVec> = OnceLock::new();
static CACHE_HITS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static CACHE_MISSES_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static ACTIVE_REQUESTS: OnceLock<Gauge> = OnceLock::new();
static CIRCUIT_BREAKER_OPEN: OnceLock<prometheus::GaugeVec> = OnceLock::new();

/// Total requests by provider, model, and status label.
pub fn requests_total() -> &'static IntCounterVec {
    REQUESTS_TOTAL.get_or_init(|| {
        let m = IntCounterVec::new(
            Opts::new(
                "modelswitch_requests_total",
                "Total requests by provider model and status",
            ),
            &["provider", "model", "status"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Request duration histogram in seconds, labelled by provider and model.
pub fn request_duration() -> &'static HistogramVec {
    REQUEST_DURATION.get_or_init(|| {
        let m = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "modelswitch_request_duration_seconds",
                "Request duration in seconds",
            )
            .buckets(vec![
                0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0,
            ]),
            &["provider", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Cache hit counter.
pub fn cache_hits() -> &'static IntCounter {
    CACHE_HITS_TOTAL.get_or_init(|| {
        let m = IntCounter::new("modelswitch_cache_hits_total", "Cache hits")
            .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Cache miss counter.
pub fn cache_misses() -> &'static IntCounter {
    CACHE_MISSES_TOTAL.get_or_init(|| {
        let m = IntCounter::new("modelswitch_cache_misses_total", "Cache misses")
            .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Gauge for currently in-flight gateway requests.
pub fn active_requests() -> &'static Gauge {
    ACTIVE_REQUESTS.get_or_init(|| {
        let m =
            Gauge::new("modelswitch_active_requests", "Currently in-flight requests")
                .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Per-channel circuit breaker state gauge (1 = open, 0 = closed).
pub fn circuit_breaker_open() -> &'static prometheus::GaugeVec {
    CIRCUIT_BREAKER_OPEN.get_or_init(|| {
        let m = prometheus::GaugeVec::new(
            Opts::new(
                "modelswitch_circuit_breaker_open",
                "Circuit breaker open state per channel (1=open, 0=closed)",
            ),
            &["channel"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Render all registered metrics as Prometheus text format.
pub fn render() -> String {
    let mut buf = Vec::new();
    let encoder = TextEncoder::new();
    let metrics = registry().gather();
    encoder.encode(&metrics, &mut buf).expect("encode metrics");
    String::from_utf8(buf).expect("metrics are valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_provides_metric_names() {
        // Force initialization of all metrics
        requests_total();
        request_duration();
        cache_hits();
        cache_misses();
        active_requests();
        // GaugeVec requires at least one labelled observation to appear in gather output
        circuit_breaker_open().with_label_values(&["test-channel"]);

        let output = render();
        assert!(output.contains("modelswitch_requests_total"));
        assert!(output.contains("modelswitch_request_duration_seconds"));
        assert!(output.contains("modelswitch_cache_hits_total"));
        assert!(output.contains("modelswitch_cache_misses_total"));
        assert!(output.contains("modelswitch_active_requests"));
        assert!(output.contains("modelswitch_circuit_breaker_open"));
    }

    #[test]
    fn counter_increments_correctly() {
        cache_hits().inc();
        cache_hits().inc();
        let before = cache_hits().get();
        cache_hits().inc();
        assert_eq!(cache_hits().get(), before + 1);
    }

    #[test]
    fn labelled_counter_with_values() {
        requests_total()
            .with_label_values(&["openai", "gpt-4", "success"])
            .inc();
        let output = render();
        assert!(output.contains("openai"));
        assert!(output.contains("gpt-4"));
        assert!(output.contains("success"));
    }

    #[test]
    fn histogram_observations_recorded() {
        request_duration()
            .with_label_values(&["anthropic", "claude-3"])
            .observe(1.5);
        let output = render();
        assert!(output.contains("modelswitch_request_duration_seconds_bucket"));
    }
}
