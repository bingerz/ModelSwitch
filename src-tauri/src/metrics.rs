//! Prometheus metrics for gateway observability.
//!
//! Metrics are exposed via the `/metrics` endpoint in Prometheus text format.
//! All metrics use lazy `OnceLock` initialization so they are zero-cost until
//! first accessed.

use prometheus::{
    Encoder, Gauge, HistogramVec, IntCounter, IntCounterVec, Opts, Registry, TextEncoder,
};
use std::sync::OnceLock;
use std::time::Duration;

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Returns the global Prometheus registry.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::new)
}

static REQUESTS_TOTAL: OnceLock<IntCounterVec> = OnceLock::new();
static REQUEST_DURATION: OnceLock<HistogramVec> = OnceLock::new();
static REQUEST_LATENCY: OnceLock<HistogramVec> = OnceLock::new();
static CACHE_HITS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static CACHE_MISSES_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static CACHE_EVICTIONS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static ACTIVE_REQUESTS: OnceLock<Gauge> = OnceLock::new();
static CIRCUIT_BREAKER_OPEN: OnceLock<prometheus::GaugeVec> = OnceLock::new();
static RETRIES_TOTAL: OnceLock<IntCounterVec> = OnceLock::new();
static TTFT_SECONDS: OnceLock<HistogramVec> = OnceLock::new();
static INPUT_TOKENS_TOTAL: OnceLock<IntCounterVec> = OnceLock::new();
static OUTPUT_TOKENS_TOTAL: OnceLock<IntCounterVec> = OnceLock::new();
static TOKEN_USAGE: OnceLock<HistogramVec> = OnceLock::new();
static REQUEST_COST_USD: OnceLock<HistogramVec> = OnceLock::new();

/// Latency histogram buckets covering fast API calls through to slow LLM generations.
const LATENCY_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];

/// Cost histogram buckets ranging from a fraction of a cent up to multi-dollar requests.
const COST_BUCKETS: &[f64] = &[0.0001, 0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0];

/// Token usage histogram buckets ranging from a single token to millions.
const TOKEN_BUCKETS: &[f64] = &[1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0];

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
            .buckets(vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0]),
            &["provider", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Latency histogram (`request_latency_seconds`) labelled by model and provider.
/// Uses buckets tuned for API latency (50ms to 60s).
pub fn request_latency() -> &'static HistogramVec {
    REQUEST_LATENCY.get_or_init(|| {
        let m = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "modelswitch_request_latency_seconds",
                "End-to-end request latency in seconds",
            )
            .buckets(LATENCY_BUCKETS.to_vec()),
            &["model", "provider"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Token usage histogram (`token_usage_total`) labelled by `type` (prompt or
/// completion) and `model`. Each request records two observations — one for
/// prompt tokens, one for completion tokens.
pub fn token_usage() -> &'static HistogramVec {
    TOKEN_USAGE.get_or_init(|| {
        let m = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "modelswitch_token_usage_total",
                "Token usage per request by type (prompt/completion) and model",
            )
            .buckets(TOKEN_BUCKETS.to_vec()),
            &["type", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Per-request cost histogram (`request_cost_usd`) labelled by model.
pub fn request_cost_usd() -> &'static HistogramVec {
    REQUEST_COST_USD.get_or_init(|| {
        let m = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "modelswitch_request_cost_usd",
                "Estimated per-request cost in USD",
            )
            .buckets(COST_BUCKETS.to_vec()),
            &["model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Cache hit counter.
pub fn cache_hits() -> &'static IntCounter {
    CACHE_HITS_TOTAL.get_or_init(|| {
        let m = IntCounter::new("modelswitch_cache_hits_total", "Cache hits").expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Cache miss counter.
pub fn cache_misses() -> &'static IntCounter {
    CACHE_MISSES_TOTAL.get_or_init(|| {
        let m =
            IntCounter::new("modelswitch_cache_misses_total", "Cache misses").expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Cache entries evicted by TTL expiry or capacity overflow.
pub fn cache_evictions() -> &'static IntCounter {
    CACHE_EVICTIONS_TOTAL.get_or_init(|| {
        let m = IntCounter::new("modelswitch_cache_evictions_total", "Cache entries evicted")
            .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Gauge for currently in-flight gateway requests.
pub fn active_requests() -> &'static Gauge {
    ACTIVE_REQUESTS.get_or_init(|| {
        let m = Gauge::new(
            "modelswitch_active_requests",
            "Currently in-flight requests",
        )
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

/// Total retries by provider and model.
pub fn retries_total() -> &'static IntCounterVec {
    RETRIES_TOTAL.get_or_init(|| {
        let m = IntCounterVec::new(
            Opts::new(
                "modelswitch_retries_total",
                "Total dispatch retries by provider and model",
            ),
            &["provider", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// TTFT histogram in seconds, labelled by provider and model.
/// Uses the same buckets as [`request_latency`] so the two can be compared directly.
pub fn ttft_seconds() -> &'static HistogramVec {
    TTFT_SECONDS.get_or_init(|| {
        let m = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "modelswitch_ttft_seconds",
                "Time to first token in seconds",
            )
            .buckets(LATENCY_BUCKETS.to_vec()),
            &["provider", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Total input tokens consumed, labelled by provider and model.
pub fn input_tokens_total() -> &'static IntCounterVec {
    INPUT_TOKENS_TOTAL.get_or_init(|| {
        let m = IntCounterVec::new(
            Opts::new(
                "modelswitch_input_tokens_total",
                "Total input tokens consumed",
            ),
            &["provider", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Total output tokens consumed, labelled by provider and model.
pub fn output_tokens_total() -> &'static IntCounterVec {
    OUTPUT_TOKENS_TOTAL.get_or_init(|| {
        let m = IntCounterVec::new(
            Opts::new(
                "modelswitch_output_tokens_total",
                "Total output tokens consumed",
            ),
            &["provider", "model"],
        )
        .expect("valid opts");
        registry().register(Box::new(m.clone())).ok();
        m
    })
}

/// Record end-to-end request latency. `provider` is the upstream channel
/// provider (e.g. "openai"); `model` is the resolved model name.
pub fn record_latency(duration: Duration, model: &str, provider: &str) {
    request_latency()
        .with_label_values(&[model, provider])
        .observe(duration.as_secs_f64());
}

/// Record per-request token usage. Emits two histogram observations — one
/// labelled `prompt`, one labelled `completion` — so both can be queried via
/// `modelswitch_token_usage_total{type="prompt"}` / `{type="completion"}`.
pub fn record_tokens(prompt: u64, completion: u64, model: &str) {
    token_usage()
        .with_label_values(&["prompt", model])
        .observe(prompt as f64);
    token_usage()
        .with_label_values(&["completion", model])
        .observe(completion as f64);
}

/// Record estimated per-request cost in USD.
pub fn record_cost(cost: f64, model: &str) {
    // Guard against negative / NaN costs that would corrupt histogram sums.
    if cost.is_finite() && cost >= 0.0 {
        request_cost_usd()
            .with_label_values(&[model])
            .observe(cost);
    }
}

/// Record time-to-first-token for a streaming request.
pub fn record_ttft(duration: Duration, model: &str, provider: &str) {
    ttft_seconds()
        .with_label_values(&[provider, model])
        .observe(duration.as_secs_f64());
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
        request_latency();
        cache_hits();
        cache_misses();
        cache_evictions();
        active_requests();
        // GaugeVec requires at least one labelled observation to appear in gather output
        circuit_breaker_open().with_label_values(&["test-channel"]);
        retries_total().with_label_values(&["test", "test-model"]);
        ttft_seconds()
            .with_label_values(&["test", "test-model"])
            .observe(0.1);
        input_tokens_total().with_label_values(&["test", "test-model"]);
        output_tokens_total().with_label_values(&["test", "test-model"]);
        token_usage()
            .with_label_values(&["prompt", "test-model"])
            .observe(100.0);
        request_cost_usd()
            .with_label_values(&["test-model"])
            .observe(0.01);

        let output = render();
        assert!(output.contains("modelswitch_requests_total"));
        assert!(output.contains("modelswitch_request_duration_seconds"));
        assert!(output.contains("modelswitch_request_latency_seconds"));
        assert!(output.contains("modelswitch_cache_hits_total"));
        assert!(output.contains("modelswitch_cache_misses_total"));
        assert!(output.contains("modelswitch_cache_evictions_total"));
        assert!(output.contains("modelswitch_active_requests"));
        assert!(output.contains("modelswitch_circuit_breaker_open"));
        assert!(output.contains("modelswitch_retries_total"));
        assert!(output.contains("modelswitch_ttft_seconds"));
        assert!(output.contains("modelswitch_input_tokens_total"));
        assert!(output.contains("modelswitch_output_tokens_total"));
        assert!(output.contains("modelswitch_token_usage_total"));
        assert!(output.contains("modelswitch_request_cost_usd"));
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

    #[test]
    fn record_latency_observations_appear_under_model_provider_labels() {
        record_latency(Duration::from_millis(750), "gpt-4o", "openai");
        let output = render();
        assert!(output.contains("modelswitch_request_latency_seconds_bucket"));
        assert!(output.contains("gpt-4o"));
        assert!(output.contains("openai"));
    }

    #[test]
    fn record_tokens_emits_prompt_and_completion_observations() {
        record_tokens(1200, 450, "claude-3-5-sonnet");
        let output = render();
        assert!(output.contains("modelswitch_token_usage_total_bucket"));
        assert!(output.contains("prompt"));
        assert!(output.contains("completion"));
        assert!(output.contains("claude-3-5-sonnet"));
    }

    #[test]
    fn record_cost_observations_appear_under_model_label() {
        record_cost(0.0234, "gpt-4o");
        let output = render();
        assert!(output.contains("modelswitch_request_cost_usd_bucket"));
        assert!(output.contains("gpt-4o"));
    }

    #[test]
    fn record_cost_ignores_non_finite_values() {
        // NaN / negative values must be silently dropped to protect histogram sums.
        record_cost(f64::NAN, "gpt-4o");
        record_cost(-1.0, "gpt-4o");
        // Finite, non-negative values should still be accepted.
        record_cost(0.5, "gpt-4o");
        let output = render();
        assert!(output.contains("modelswitch_request_cost_usd_bucket"));
    }

    #[test]
    fn record_ttft_observations_appear_under_provider_model_labels() {
        record_ttft(Duration::from_millis(120), "claude-3", "anthropic");
        let output = render();
        assert!(output.contains("modelswitch_ttft_seconds_bucket"));
        assert!(output.contains("anthropic"));
        assert!(output.contains("claude-3"));
    }

    #[test]
    fn latency_buckets_match_spec() {
        // Sanity-check the latency bucket boundaries so dashboards can rely on them.
        request_latency()
            .with_label_values(&["test-model", "test-provider"])
            .observe(0.075);
        let output = render();
        for needle in &[
            "modelswitch_request_latency_seconds_bucket",
            r#"le="0.05""#,
            r#"le="60""#,
        ] {
            assert!(output.contains(needle), "expected {needle} in render output");
        }
    }
}
