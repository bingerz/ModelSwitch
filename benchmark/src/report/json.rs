use std::fs;
use std::path::Path;

use crate::metrics::summary::BenchmarkSummary;

/// Write a benchmark summary as formatted JSON to a file.
pub fn write_report(summary: &BenchmarkSummary, path: &str) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(summary)?;
    fs::write(Path::new(path), json)?;
    tracing::info!("JSON report written to {}", path);
    Ok(())
}

/// Serialize a summary to a JSON string.
pub fn to_json(summary: &BenchmarkSummary) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(summary)?)
}
