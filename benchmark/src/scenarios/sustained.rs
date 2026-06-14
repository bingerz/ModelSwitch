use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::client::openai::{OpenAIClient, RequestTiming};
use crate::metrics::summary::{ErrorCategory, RequestResult};

/// Config for sustained scenario.
pub struct SustainedConfig {
    pub rps: u32,
    pub concurrency: usize,
    pub duration_secs: u64,
    pub warmup_secs: u64,
    pub max_tokens: usize,
}

/// Run the sustained scenario — constant RPS over the full duration.
pub async fn run_sustained(
    client: Arc<OpenAIClient>,
    config: SustainedConfig,
) -> anyhow::Result<Vec<RequestResult>> {
    let rps = config.rps;
    let interval = Duration::from_secs_f64(1.0 / rps as f64);
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let results = Arc::new(tokio::sync::Mutex::new(Vec::<RequestResult>::new()));

    let deadline = Instant::now() + Duration::from_secs(config.duration_secs);
    let warmup_deadline = Instant::now() + Duration::from_secs(config.warmup_secs);

    let mut ticker = tokio::time::interval(interval);
    // Don't accumulate missed ticks
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut tasks = Vec::new();
    let mut dropped = 0u64;

    loop {
        ticker.tick().await;
        if Instant::now() >= deadline {
            break;
        }

        // Use try_acquire to detect backpressure, but log it
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                dropped += 1;
                if dropped % 100 == 1 {
                    tracing::warn!(
                        "Sustained: concurrency exhausted ({} requests dropped so far)",
                        dropped
                    );
                }
                continue;
            }
        };

        let client = Arc::clone(&client);
        let results = Arc::clone(&results);
        let max_tokens = config.max_tokens;
        let warmup_dl = warmup_deadline;

        tasks.push(tokio::spawn(async move {
            let content = format!("sustained {}", Uuid::new_v4());

            let timing: RequestTiming =
                client
                    .chat(&content, max_tokens)
                    .await
                    .unwrap_or(RequestTiming {
                        status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                        success: false,
                        latency: Duration::ZERO,
                        conn_latency: Duration::ZERO,
                        error_category: ErrorCategory::Network,
                    });

            if Instant::now() > warmup_dl {
                results.lock().await.push(RequestResult {
                    status: timing.status.as_u16(),
                    success: timing.success,
                    latency: timing.latency,
                    conn_latency: timing.conn_latency,
                    error_category: timing.error_category,
                });
            }

            drop(permit);
        }));
    }

    for t in tasks {
        let _ = t.await;
    }

    if dropped > 0 {
        tracing::warn!(
            "Sustained: {} total requests dropped due to concurrency limit",
            dropped
        );
    }

    let results = Arc::try_unwrap(results)
        .map_err(|_| anyhow::anyhow!("results still shared"))?
        .into_inner();

    Ok(results)
}
