use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::ScenarioConfig;
use crate::client::openai::{OpenAIClient, RequestTiming};
use crate::metrics::summary::{ErrorCategory, RequestResult};

/// Abort the benchmark loop after this many consecutive failures.
const MAX_CONSECUTIVE_FAILURES: u32 = 200;

/// Run the non-streaming chat scenario.
pub async fn run(
    client: Arc<OpenAIClient>,
    config: ScenarioConfig,
) -> anyhow::Result<Vec<RequestResult>> {
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let results = Arc::new(tokio::sync::Mutex::new(Vec::<RequestResult>::new()));

    let rate_limiter = config.rps.map(|rps| {
        use governor::{Quota, RateLimiter};
        let quota = Quota::per_second(std::num::NonZeroU32::new(rps).expect("rps > 0"));
        Arc::new(RateLimiter::direct(quota))
    });

    let deadline = Instant::now() + Duration::from_secs(config.duration_secs);
    let warmup_deadline = Instant::now() + Duration::from_secs(config.warmup_secs);

    let consecutive_failures = Arc::new(AtomicU32::new(0));
    let mut tasks = Vec::new();

    while Instant::now() < deadline {
        if consecutive_failures.load(Ordering::Relaxed) >= MAX_CONSECUTIVE_FAILURES {
            tracing::warn!(
                "Chat: aborting early after {} consecutive failures (circuit breaker?)",
                MAX_CONSECUTIVE_FAILURES
            );
            break;
        }

        if let Some(ref limiter) = rate_limiter {
            while limiter.check().is_err() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }

        let permit = semaphore.clone().acquire_owned().await?;
        let client = Arc::clone(&client);
        let results = Arc::clone(&results);
        let fail_counter = Arc::clone(&consecutive_failures);
        let max_tokens = config.max_tokens;
        let message_tokens = config.message_tokens;

        tasks.push(tokio::spawn(async move {
            let msg_id = Uuid::new_v4();
            let content = format!("{} {}", "word ".repeat(message_tokens), msg_id);

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

            if timing.success {
                fail_counter.store(0, Ordering::Relaxed);
            } else {
                fail_counter.fetch_add(1, Ordering::Relaxed);
            }

            if Instant::now() > warmup_deadline {
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

    let results = Arc::try_unwrap(results)
        .map_err(|_| anyhow::anyhow!("results still shared"))?
        .into_inner();

    Ok(results)
}
