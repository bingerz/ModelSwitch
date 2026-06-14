//! Example: Benchmark a direct LLM endpoint (no proxy) as baseline.
//!
//! ```bash
//! cargo run --example bench_direct
//! ```

use llm_gateway_bench::client::openai::OpenAIClient;
use llm_gateway_bench::scenarios::{self, ScenarioConfig};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let client = Arc::new(OpenAIClient::new(
        "https://api.openai.com",
        &std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "dummy".into()),
        "gpt-4",
        120,
        100,
        false,
        llm_gateway_bench::client::Protocol::OpenAI,
    )?);

    let config = ScenarioConfig {
        concurrency: 10,
        duration_secs: 30,
        warmup_secs: 5,
        rps: Some(5),
        message_tokens: 10,
        max_tokens: 50,
        burst_size: 200,
        stream_ratio: 70,
    };

    println!("Benchmarking direct endpoint (baseline)...");
    let summary = scenarios::run_scenario("chat", client, config).await?;

    println!("\n=== Direct Baseline ===");
    println!(
        "P50: {}ms  P95: {}ms  P99: {}ms",
        summary.latency_p50_ms, summary.latency_p95_ms, summary.latency_p99_ms
    );
    println!(
        "RPS: {:.1}  Errors: {:.1}%",
        summary.rps, summary.error_rate
    );

    Ok(())
}
