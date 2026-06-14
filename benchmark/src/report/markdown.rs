use std::fs;
use std::path::Path;

use crate::metrics::summary::BenchmarkSummary;

/// Format duration for display — uses ms for sub-second, seconds otherwise.
fn fmt_duration(secs: f64) -> String {
    if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else {
        format!("{:.1}s", secs)
    }
}

/// Format a benchmark summary as a markdown table string.
pub fn format_summary(summary: &BenchmarkSummary) -> String {
    let mut out = format!(
        r#"# LLM Gateway Benchmark Report

**Scenario:** {} · **Duration:** {}

## Summary

| Metric                  | Value          |
|-------------------------|----------------|
| Total Requests          | {}             |
| Successful              | {}             |
| Failed                  | {}             |
| RPS (Throughput)        | {:.1}          |
| Error Rate              | {:.2}%         |

## Latency Distribution

| Percentile | Latency (ms)   |
|------------|----------------|
| P50        | {}             |
| P95        | {}             |
| P99        | {}             |
| Mean       | {:.1}          |
| Min / Max  | {} / {}        |
"#,
        summary.scenario,
        fmt_duration(summary.duration_secs),
        summary.total_requests,
        summary.successful,
        summary.failed,
        summary.rps,
        summary.error_rate,
        summary.latency_p50_ms,
        summary.latency_p95_ms,
        summary.latency_p99_ms,
        summary.latency_mean_ms,
        summary.latency_min_ms,
        summary.latency_max_ms,
    );

    // Streaming-specific metrics
    if summary.ttfb_p50_ms > 0 || summary.avg_chunks_per_request > 0.0 {
        out.push_str(&format!(
            r#"
## Streaming Metrics

| Metric                    | Value     |
|---------------------------|-----------|
| TTFB P50                  | {} ms     |
| TTFB P95                  | {} ms     |
| TTFB Mean                 | {:.1} ms  |
| Chunk Interval P50        | {} ms     |
| Chunk Interval P95        | {} ms     |
| Avg Chunks / Request      | {:.1}     |
"#,
            summary.ttfb_p50_ms,
            summary.ttfb_p95_ms,
            summary.ttfb_mean_ms,
            summary.chunk_interval_p50_ms,
            summary.chunk_interval_p95_ms,
            summary.avg_chunks_per_request,
        ));
    }

    // Error breakdown (skip if no errors)
    if summary.failed > 0 {
        out.push_str(&format!(
            r#"
## Error Breakdown

| Category       | Count |
|----------------|-------|
| 4xx Errors     | {}    |
| 5xx Errors     | {}    |
| Timeouts       | {}    |
| Network Errors | {}    |
"#,
            summary.errors_4xx, summary.errors_5xx, summary.errors_timeout, summary.errors_network,
        ));
    }

    out
}

/// Write a benchmark summary as markdown to a file.
pub fn write_report(summary: &BenchmarkSummary, path: &str) -> anyhow::Result<()> {
    let content = format_summary(summary);
    fs::write(Path::new(path), content)?;
    tracing::info!("Markdown report written to {}", path);
    Ok(())
}
