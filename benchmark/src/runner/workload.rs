use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::client::openai::OpenAIClient;
use crate::client::Protocol;
use crate::metrics::summary::BenchmarkSummary;
use crate::scenarios::{self, ScenarioConfig};

/// Interval between RSS samples taken by the background monitor.
const RSS_SAMPLE_INTERVAL_SECS: u64 = 5;

/// Sentinel used when RSS cannot be read.
const RSS_UNAVAILABLE: u64 = 0;

/// Returns the current process RSS in kilobytes, or `None` if it cannot be read.
///
/// Uses `ps -o rss= -p PID`, which works on both macOS and Linux without
/// requiring any external crate dependency.
fn current_rss_kb() -> Option<u64> {
    let pid = std::process::id();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let rss_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if rss_str.is_empty() {
        return None;
    }
    rss_str.parse::<u64>().ok()
}

/// Snapshot of RSS values collected during a benchmark run.
#[derive(Clone, Copy)]
struct RssStats {
    baseline_kb: u64,
    peak_kb: u64,
}

impl RssStats {
    /// Delta between peak and baseline in KB. Zero when either value is
    /// unavailable or no growth occurred.
    fn delta_kb(&self) -> u64 {
        if self.baseline_kb == RSS_UNAVAILABLE || self.peak_kb == RSS_UNAVAILABLE {
            return 0;
        }
        self.peak_kb.saturating_sub(self.baseline_kb)
    }

    fn is_available(&self) -> bool {
        self.baseline_kb != RSS_UNAVAILABLE && self.peak_kb != RSS_UNAVAILABLE
    }
}

/// Handle returned by [`start_rss_monitor`]; call [`RssMonitor::finalize`] to
/// stop sampling and read the peak/baseline snapshot.
struct RssMonitor {
    baseline_kb: u64,
    peak_slot: Arc<AtomicU64>,
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl RssMonitor {
    /// Stop the background sampler and return the final RSS stats.
    fn finalize(mut self) -> RssStats {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        // Take one final sample so the peak reflects the very end of the run.
        if let Some(sample) = current_rss_kb() {
            // fetch_max keeps the largest value seen so far.
            self.peak_slot.fetch_max(sample, Ordering::Relaxed);
        }
        let peak_kb = self.peak_slot.load(Ordering::Relaxed);
        RssStats {
            baseline_kb: self.baseline_kb,
            peak_kb,
        }
    }
}

/// Spawn a background task that samples RSS every
/// [`RSS_SAMPLE_INTERVAL_SECS`] seconds, tracking the peak value via a shared
/// atomic slot. Returns a handle whose [`RssMonitor::finalize`] stops the task
/// and reads the peak.
fn start_rss_monitor() -> RssMonitor {
    let baseline = current_rss_kb().unwrap_or(RSS_UNAVAILABLE);
    let baseline_kb = baseline;
    let peak_slot = Arc::new(AtomicU64::new(baseline));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let peak_clone = Arc::clone(&peak_slot);
    let mut stop_rx = stop_rx;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                _ = tokio::time::sleep(Duration::from_secs(RSS_SAMPLE_INTERVAL_SECS)) => {
                    if let Some(sample) = current_rss_kb() {
                        peak_clone.fetch_max(sample, Ordering::Relaxed);
                    }
                }
            }
        }
    });

    RssMonitor {
        baseline_kb,
        peak_slot,
        stop_tx: Some(stop_tx),
    }
}

/// Configuration for a benchmark workload run.
pub struct WorkloadConfig {
    pub target: String,
    pub api_key: String,
    pub model: String,
    pub scenario: String,
    pub concurrency: usize,
    pub duration_secs: u64,
    pub warmup_secs: u64,
    pub rps: Option<u32>,
    pub message_tokens: usize,
    pub max_tokens: usize,
    pub timeout_secs: u64,
    pub pool_max_idle: usize,
    pub tls_skip_verify: bool,
    pub burst_size: usize,
    pub runs: usize,
    pub stream_ratio: u8,
    pub protocol: Protocol,
}

/// Run benchmark(s) and return all run summaries (1 if runs=1).
pub async fn run_benchmark(config: WorkloadConfig) -> anyhow::Result<Vec<BenchmarkSummary>> {
    let client = Arc::new(OpenAIClient::new(
        &config.target,
        &config.api_key,
        &config.model,
        config.timeout_secs,
        config.pool_max_idle,
        config.tls_skip_verify,
        config.protocol,
    )?);

    let scenario_config = ScenarioConfig {
        concurrency: config.concurrency,
        duration_secs: config.duration_secs,
        warmup_secs: config.warmup_secs,
        rps: config.rps,
        message_tokens: config.message_tokens,
        max_tokens: config.max_tokens,
        burst_size: config.burst_size,
        stream_ratio: config.stream_ratio,
    };

    let rss_monitor = start_rss_monitor();
    if let Some(baseline) = current_rss_kb() {
        tracing::info!("RSS baseline: {} KB ({} MB)", baseline, baseline / 1024);
    } else {
        tracing::warn!("RSS sampling unavailable on this platform; memory stats disabled");
    }

    let runs = config.runs.max(1);
    let mut summaries = Vec::with_capacity(runs);

    for i in 1..=runs {
        if runs > 1 {
            tracing::info!("═══ Run {}/{} ═══", i, runs);
        }

        tracing::info!(
            "Starting: scenario={}, target={}, concurrency={}, duration={}s",
            config.scenario,
            config.target,
            scenario_config.concurrency,
            scenario_config.duration_secs
        );

        let summary = scenarios::run_scenario(
            &config.scenario,
            Arc::clone(&client),
            ScenarioConfig {
                concurrency: scenario_config.concurrency,
                duration_secs: scenario_config.duration_secs,
                warmup_secs: scenario_config.warmup_secs,
                rps: scenario_config.rps,
                message_tokens: scenario_config.message_tokens,
                max_tokens: scenario_config.max_tokens,
                burst_size: scenario_config.burst_size,
                stream_ratio: scenario_config.stream_ratio,
            },
        )
        .await?;

        tracing::info!(
            "Run {}/{} done: {} reqs, {:.1} RPS, P50={}ms, P95={}ms, P99={}ms, err={:.1}%",
            i,
            runs,
            summary.total_requests,
            summary.rps,
            summary.latency_p50_ms,
            summary.latency_p95_ms,
            summary.latency_p99_ms,
            summary.error_rate
        );

        summaries.push(summary);

        if i < runs {
            tracing::info!("Cooldown 3s before next run...");
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }

    let rss_stats = rss_monitor.finalize();

    if rss_stats.is_available() {
        tracing::info!(
            "RSS peak: {} KB ({} MB) | delta: {} KB ({} MB)",
            rss_stats.peak_kb,
            rss_stats.peak_kb / 1024,
            rss_stats.delta_kb(),
            rss_stats.delta_kb() / 1024,
        );
    }

    if summaries.len() > 1 {
        log_variance(&summaries, rss_stats);
    } else if rss_stats.is_available() {
        tracing::info!(
            "═══ Memory (1 run) ═══ baseline={} MB peak={} MB delta={} MB",
            rss_stats.baseline_kb / 1024,
            rss_stats.peak_kb / 1024,
            rss_stats.delta_kb() / 1024,
        );
    }

    Ok(summaries)
}

/// Log variance statistics across multiple runs.
fn log_variance(summaries: &[BenchmarkSummary], rss: RssStats) {
    let p95s: Vec<f64> = summaries.iter().map(|s| s.latency_p95_ms as f64).collect();
    let p99s: Vec<f64> = summaries.iter().map(|s| s.latency_p99_ms as f64).collect();
    let rps_vals: Vec<f64> = summaries.iter().map(|s| s.rps).collect();

    let p95_mean = mean(&p95s);
    let p95_stddev = stddev(&p95s, p95_mean);
    let p99_mean = mean(&p99s);
    let rps_mean = mean(&rps_vals);

    let mut header = format!(
        "═══ Variance Analysis ({} runs) ═══\n\
         P95: mean={:.1}ms ±{:.1}ms\n\
         P99: mean={:.1}ms\n\
         RPS: mean={:.1}",
        summaries.len(),
        p95_mean,
        p95_stddev,
        p99_mean,
        rps_mean,
    );

    if rss.is_available() {
        header.push_str(&format!(
            "\nRSS: baseline={} MB, peak={} MB, delta={} MB",
            rss.baseline_kb / 1024,
            rss.peak_kb / 1024,
            rss.delta_kb() / 1024,
        ));
    }

    tracing::info!("{}", header);
}

fn mean(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    vals.iter().sum::<f64>() / vals.len() as f64
}

fn stddev(vals: &[f64], mean: f64) -> f64 {
    if vals.len() < 2 {
        return 0.0;
    }
    let variance =
        vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (vals.len() - 1) as f64;
    variance.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_stats_delta_handles_unavailable() {
        let stats = RssStats {
            baseline_kb: RSS_UNAVAILABLE,
            peak_kb: 10_000,
        };
        assert_eq!(stats.delta_kb(), 0);
        assert!(!stats.is_available());
    }

    #[test]
    fn rss_stats_delta_normal_case() {
        let stats = RssStats {
            baseline_kb: 5_000,
            peak_kb: 8_000,
        };
        assert_eq!(stats.delta_kb(), 3_000);
        assert!(stats.is_available());
    }

    #[test]
    fn rss_stats_delta_clamps_to_zero_when_peak_below_baseline() {
        let stats = RssStats {
            baseline_kb: 10_000,
            peak_kb: 7_000,
        };
        assert_eq!(stats.delta_kb(), 0);
    }

    #[test]
    fn current_rss_kb_returns_some_on_unix() {
        // ps exists on macOS and Linux CI runners.
        let rss = current_rss_kb();
        assert!(rss.is_some(), "expected ps-based RSS reading to work");
        assert!(rss.unwrap() > 0);
    }

    #[tokio::test]
    async fn rss_monitor_records_peak_via_sampling() {
        // We can't easily instrument `ps`, but the monitor should still produce
        // a non-empty baseline/peak snapshot when finalized immediately.
        let monitor = start_rss_monitor();
        // Yield once so the spawned task can initialize.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let stats = monitor.finalize();
        assert!(stats.is_available());
        assert!(stats.peak_kb > 0);
    }
}
