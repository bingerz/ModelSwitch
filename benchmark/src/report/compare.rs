use crate::metrics::summary::BenchmarkSummary;

/// Result of comparing two benchmark runs (direct vs proxy).
#[derive(Debug, serde::Serialize)]
pub struct ComparisonResult {
    pub direct: BenchmarkSummary,
    pub proxy: BenchmarkSummary,
    pub latency_p50_overhead_ms: i64,
    pub latency_p95_overhead_ms: i64,
    pub latency_p99_overhead_ms: i64,
    pub ttfb_p50_overhead_ms: i64,
    pub ttfb_p95_overhead_ms: i64,
    pub rps_loss_pct: f64,
    pub error_rate_diff_pct: f64,
}

impl ComparisonResult {
    pub fn new(direct: BenchmarkSummary, proxy: BenchmarkSummary) -> Self {
        Self {
            latency_p50_overhead_ms: proxy.latency_p50_ms as i64 - direct.latency_p50_ms as i64,
            latency_p95_overhead_ms: proxy.latency_p95_ms as i64 - direct.latency_p95_ms as i64,
            latency_p99_overhead_ms: proxy.latency_p99_ms as i64 - direct.latency_p99_ms as i64,
            ttfb_p50_overhead_ms: proxy.ttfb_p50_ms as i64 - direct.ttfb_p50_ms as i64,
            ttfb_p95_overhead_ms: proxy.ttfb_p95_ms as i64 - direct.ttfb_p95_ms as i64,
            rps_loss_pct: if direct.rps > 0.0 {
                (direct.rps - proxy.rps) / direct.rps * 100.0
            } else {
                0.0
            },
            error_rate_diff_pct: proxy.error_rate - direct.error_rate,
            direct,
            proxy,
        }
    }

    /// Format as a markdown comparison table.
    pub fn to_markdown(&self) -> String {
        let sign = |v: i64| {
            if v >= 0 {
                format!("+{}ms", v)
            } else {
                format!("{}ms", v)
            }
        };
        let pct_sign = |v: f64| {
            if v >= 0.0 {
                format!("+{:.1}%", v)
            } else {
                format!("{:.1}%", v)
            }
        };

        format!(
            r#"# Gateway Overhead Comparison Report

## Latency Overhead (Proxy − Direct)

| Metric          | Direct     | Via Proxy  | Overhead   |
|-----------------|------------|------------|-----------|
| P50 Latency     | {}ms       | {}ms       | {}         |
| P95 Latency     | {}ms       | {}ms       | {}         |
| P99 Latency     | {}ms       | {}ms       | {}         |
| TTFB P50        | {}ms       | {}ms       | {}         |
| TTFB P95        | {}ms       | {}ms       | {}         |

## Throughput & Errors

| Metric          | Direct     | Via Proxy  | Delta     |
|-----------------|------------|------------|-----------|
| RPS             | {:.1}      | {:.1}      | {}        |
| Error Rate      | {:.2}%     | {:.2}%     | {}        |
| Total Requests  | {}         | {}         | —         |
"#,
            self.direct.latency_p50_ms,
            self.proxy.latency_p50_ms,
            sign(self.latency_p50_overhead_ms),
            self.direct.latency_p95_ms,
            self.proxy.latency_p95_ms,
            sign(self.latency_p95_overhead_ms),
            self.direct.latency_p99_ms,
            self.proxy.latency_p99_ms,
            sign(self.latency_p99_overhead_ms),
            self.direct.ttfb_p50_ms,
            self.proxy.ttfb_p50_ms,
            sign(self.ttfb_p50_overhead_ms),
            self.direct.ttfb_p95_ms,
            self.proxy.ttfb_p95_ms,
            sign(self.ttfb_p95_overhead_ms),
            self.direct.rps,
            self.proxy.rps,
            pct_sign(-self.rps_loss_pct),
            self.direct.error_rate,
            self.proxy.error_rate,
            pct_sign(self.error_rate_diff_pct),
            self.direct.total_requests,
            self.proxy.total_requests,
        )
    }
}

/// Run a direct-vs-proxy comparison benchmark.
#[allow(clippy::too_many_arguments)]
pub async fn run_comparison(
    direct_url: &str,
    proxy_url: &str,
    api_key: &str,
    model: &str,
    scenario: &str,
    concurrency: usize,
    duration_secs: u64,
    warmup_secs: u64,
    timeout_secs: u64,
    pool_max_idle: usize,
    tls_skip_verify: bool,
    mock_tokens: usize,
) -> anyhow::Result<ComparisonResult> {
    use crate::client::openai::OpenAIClient;
    use crate::scenarios::{self, ScenarioConfig};
    use std::sync::Arc;

    let make_config = || ScenarioConfig {
        concurrency,
        duration_secs,
        warmup_secs,
        rps: None,
        message_tokens: 10,
        max_tokens: mock_tokens,
        burst_size: 200,
        stream_ratio: 70,
    };

    tracing::info!("═══ Phase 1/2: Direct baseline ═══");
    let direct_client = Arc::new(OpenAIClient::new(
        direct_url,
        api_key,
        model,
        timeout_secs,
        pool_max_idle,
        tls_skip_verify,
        crate::client::Protocol::OpenAI,
    )?);
    let direct_summary = scenarios::run_scenario(scenario, direct_client, make_config()).await?;

    tracing::info!("═══ Phase 2/2: Via proxy ═══");
    let proxy_client = Arc::new(OpenAIClient::new(
        proxy_url,
        api_key,
        model,
        timeout_secs,
        pool_max_idle,
        tls_skip_verify,
        crate::client::Protocol::OpenAI,
    )?);
    let proxy_summary = scenarios::run_scenario(scenario, proxy_client, make_config()).await?;

    Ok(ComparisonResult::new(direct_summary, proxy_summary))
}
