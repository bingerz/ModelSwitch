use serde::Serialize;
use std::time::Duration;

use super::histogram::LatencyHistogram;

/// Error categories for failed requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Client4xx,
    Server5xx,
    RateLimited429,
    Timeout,
    Network,
    None,
}

/// Result of a single benchmark request (non-streaming).
#[derive(Clone)]
pub struct RequestResult {
    pub status: u16,
    pub success: bool,
    pub latency: Duration,
    pub conn_latency: Duration,
    pub error_category: ErrorCategory,
}

/// Result of a single streaming benchmark request.
#[derive(Clone)]
pub struct StreamResult {
    pub status: u16,
    pub success: bool,
    pub latency: Duration,
    pub ttfb: Option<Duration>,
    pub chunk_intervals: Vec<Duration>,
    pub total_chunks: usize,
    pub error_category: ErrorCategory,
}

/// Aggregated benchmark statistics.
#[derive(Debug, Serialize)]
pub struct BenchmarkSummary {
    pub scenario: String,
    pub total_requests: u64,
    pub successful: u64,
    pub failed: u64,
    pub errors_4xx: u64,
    pub errors_5xx: u64,
    pub errors_timeout: u64,
    pub errors_network: u64,
    pub duration_secs: f64,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub latency_p99_ms: u64,
    pub latency_mean_ms: f64,
    pub latency_min_ms: u64,
    pub latency_max_ms: u64,
    pub conn_est_p50_ms: u64,
    pub conn_est_p95_ms: u64,
    pub rps: f64,
    pub error_rate: f64,
    // Streaming-specific
    pub ttfb_p50_ms: u64,
    pub ttfb_p95_ms: u64,
    pub ttfb_mean_ms: f64,
    pub chunk_interval_p50_ms: u64,
    pub chunk_interval_p95_ms: u64,
    pub avg_chunks_per_request: f64,
}

/// Builder that collects request results and computes the final summary.
pub struct SummaryBuilder {
    results: Vec<RequestResult>,
    stream_results: Vec<StreamResult>,
    duration: Duration,
    scenario: String,
}

impl SummaryBuilder {
    pub fn new(duration: Duration) -> Self {
        Self {
            results: Vec::new(),
            stream_results: Vec::new(),
            duration,
            scenario: "chat".to_string(),
        }
    }

    pub fn with_scenario(mut self, scenario: &str) -> Self {
        self.scenario = scenario.to_string();
        self
    }

    pub fn add_result(&mut self, result: RequestResult) {
        self.results.push(result);
    }

    pub fn add_stream_result(&mut self, result: StreamResult) {
        self.stream_results.push(result);
    }

    pub fn build(self) -> BenchmarkSummary {
        let total = (self.results.len() + self.stream_results.len()) as u64;

        let all_success: u64 = self.results.iter().filter(|r| r.success).count() as u64
            + self.stream_results.iter().filter(|r| r.success).count() as u64;
        let failed = total - all_success;

        let mut errors_4xx = 0u64;
        let mut errors_5xx = 0u64;
        let mut errors_timeout = 0u64;
        let mut errors_network = 0u64;

        let mut lat_hist = LatencyHistogram::new();
        let mut conn_hist = LatencyHistogram::new();

        for r in &self.results {
            lat_hist.record(r.latency);
            if r.conn_latency > Duration::ZERO {
                conn_hist.record(r.conn_latency);
            }
            match r.error_category {
                ErrorCategory::Client4xx | ErrorCategory::RateLimited429 => errors_4xx += 1,
                ErrorCategory::Server5xx => errors_5xx += 1,
                ErrorCategory::Network => errors_network += 1,
                ErrorCategory::Timeout => errors_timeout += 1,
                ErrorCategory::None => {}
            }
        }

        // Streaming metrics
        let mut ttfb_hist = LatencyHistogram::new();
        let mut interval_hist = LatencyHistogram::new();
        let mut total_chunks = 0usize;
        let mut stream_count = 0u64;

        for r in &self.stream_results {
            lat_hist.record(r.latency);
            if let Some(t) = r.ttfb {
                ttfb_hist.record(t);
            }
            for interval in &r.chunk_intervals {
                interval_hist.record(*interval);
            }
            total_chunks += r.total_chunks;
            stream_count += 1;

            match r.error_category {
                ErrorCategory::Client4xx | ErrorCategory::RateLimited429 => errors_4xx += 1,
                ErrorCategory::Server5xx => errors_5xx += 1,
                ErrorCategory::Network => errors_network += 1,
                ErrorCategory::Timeout => errors_timeout += 1,
                ErrorCategory::None => {}
            }
        }

        let duration_secs = self.duration.as_secs_f64();
        let rps = if duration_secs > 0.0 {
            all_success as f64 / duration_secs
        } else {
            0.0
        };

        BenchmarkSummary {
            scenario: self.scenario,
            total_requests: total,
            successful: all_success,
            failed,
            errors_4xx,
            errors_5xx,
            errors_timeout,
            errors_network,
            duration_secs,
            latency_p50_ms: lat_hist.percentile_ms(50.0),
            latency_p95_ms: lat_hist.percentile_ms(95.0),
            latency_p99_ms: lat_hist.percentile_ms(99.0),
            latency_mean_ms: lat_hist.mean_ms(),
            latency_min_ms: lat_hist.min_ms(),
            latency_max_ms: lat_hist.max_ms(),
            conn_est_p50_ms: conn_hist.percentile_ms(50.0),
            conn_est_p95_ms: conn_hist.percentile_ms(95.0),
            rps,
            error_rate: if total > 0 {
                failed as f64 / total as f64 * 100.0
            } else {
                0.0
            },
            ttfb_p50_ms: ttfb_hist.percentile_ms(50.0),
            ttfb_p95_ms: ttfb_hist.percentile_ms(95.0),
            ttfb_mean_ms: ttfb_hist.mean_ms(),
            chunk_interval_p50_ms: interval_hist.percentile_ms(50.0),
            chunk_interval_p95_ms: interval_hist.percentile_ms(95.0),
            avg_chunks_per_request: if stream_count > 0 {
                total_chunks as f64 / stream_count as f64
            } else {
                0.0
            },
        }
    }
}
