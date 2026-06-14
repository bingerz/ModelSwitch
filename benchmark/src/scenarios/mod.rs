pub mod burst;
pub mod chat;
pub mod mixed;
pub mod streaming;
pub mod sustained;

use crate::client::openai::OpenAIClient;
use crate::metrics::summary::{BenchmarkSummary, RequestResult, StreamResult, SummaryBuilder};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Common configuration shared across all scenarios.
#[derive(Clone)]
pub struct ScenarioConfig {
    pub concurrency: usize,
    pub duration_secs: u64,
    pub warmup_secs: u64,
    pub rps: Option<u32>,
    pub message_tokens: usize,
    pub max_tokens: usize,
    pub burst_size: usize,
    pub stream_ratio: u8,
}

impl Default for ScenarioConfig {
    fn default() -> Self {
        Self {
            concurrency: 10,
            duration_secs: 30,
            warmup_secs: 5,
            rps: None,
            message_tokens: 10,
            max_tokens: 100,
            burst_size: 200,
            stream_ratio: 70,
        }
    }
}

/// Compute effective measurement duration: actual elapsed minus warmup.
fn effective_duration(elapsed: Duration, warmup_secs: u64) -> Duration {
    elapsed.saturating_sub(Duration::from_secs(warmup_secs))
}

/// Dispatch to the appropriate scenario runner by name.
pub async fn run_scenario(
    name: &str,
    client: Arc<OpenAIClient>,
    config: ScenarioConfig,
) -> anyhow::Result<BenchmarkSummary> {
    let warmup_secs = config.warmup_secs;

    match name {
        "streaming" | "stream" => {
            let start = Instant::now();
            let results: Vec<StreamResult> = streaming::run(client, config).await?;
            let duration = effective_duration(start.elapsed(), warmup_secs);
            let mut builder = SummaryBuilder::new(duration).with_scenario(name);
            for r in results {
                builder.add_stream_result(r);
            }
            Ok(builder.build())
        }
        "chat" => {
            let start = Instant::now();
            let results: Vec<RequestResult> = chat::run(client, config).await?;
            let duration = effective_duration(start.elapsed(), warmup_secs);
            let mut builder = SummaryBuilder::new(duration).with_scenario(name);
            for r in results {
                builder.add_result(r);
            }
            Ok(builder.build())
        }
        "burst" => {
            let outcome = burst::run_burst(
                client,
                burst::BurstConfig {
                    burst_size: config.burst_size,
                    message_tokens: config.message_tokens,
                    max_tokens: config.max_tokens,
                },
            )
            .await?;
            let mut builder = SummaryBuilder::new(outcome.elapsed).with_scenario(name);
            for r in outcome.results {
                builder.add_result(r);
            }
            Ok(builder.build())
        }
        "sustained" => {
            let start = Instant::now();
            let results = sustained::run_sustained(
                client,
                sustained::SustainedConfig {
                    rps: config.rps.unwrap_or(10),
                    concurrency: config.concurrency,
                    duration_secs: config.duration_secs,
                    warmup_secs: config.warmup_secs,
                    max_tokens: config.max_tokens,
                },
            )
            .await?;
            let duration = effective_duration(start.elapsed(), warmup_secs);
            let mut builder = SummaryBuilder::new(duration).with_scenario(name);
            for r in results {
                builder.add_result(r);
            }
            Ok(builder.build())
        }
        "mixed" => {
            let start = Instant::now();
            let outcome = mixed::run(client, config.clone(), config.stream_ratio).await?;
            let duration = effective_duration(start.elapsed(), warmup_secs);
            let mut builder = SummaryBuilder::new(duration).with_scenario(name);
            for r in outcome.chat_results {
                builder.add_result(r);
            }
            for r in outcome.stream_results {
                builder.add_stream_result(r);
            }
            Ok(builder.build())
        }
        other => anyhow::bail!("unknown scenario: '{}'", other),
    }
}
