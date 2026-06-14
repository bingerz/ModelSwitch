use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::ScenarioConfig;
use crate::client::openai::OpenAIClient;
use crate::metrics::summary::{ErrorCategory, StreamResult};

/// Abort the benchmark loop after this many consecutive failures.
const MAX_CONSECUTIVE_FAILURES: u32 = 200;

/// Run the streaming SSE scenario — measures TTFB and chunk intervals.
pub async fn run(
    client: Arc<OpenAIClient>,
    config: ScenarioConfig,
) -> anyhow::Result<Vec<StreamResult>> {
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let results = Arc::new(tokio::sync::Mutex::new(Vec::<StreamResult>::new()));

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
                "Streaming: aborting early after {} consecutive failures (circuit breaker?)",
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

        tasks.push(tokio::spawn(async move {
            let content = format!("stream test {}", Uuid::new_v4());

            let timing = client.chat_stream(&content, config.max_tokens).await;
            let is_warmup = Instant::now() <= warmup_deadline;

            let success = match &timing {
                Ok(t) => t.success,
                Err(_) => false,
            };
            if success {
                fail_counter.store(0, Ordering::Relaxed);
            } else {
                fail_counter.fetch_add(1, Ordering::Relaxed);
            }

            match timing {
                Ok(t) if !is_warmup => {
                    results.lock().await.push(StreamResult {
                        status: t.status.as_u16(),
                        success: t.success,
                        latency: t.latency,
                        ttfb: t.ttfb,
                        chunk_intervals: t.chunk_intervals,
                        total_chunks: t.total_chunks,
                        error_category: t.error_category,
                    });
                }
                Ok(_) => { /* warmup, skip */ }
                Err(_) if !is_warmup => {
                    results.lock().await.push(StreamResult {
                        status: 500,
                        success: false,
                        latency: Duration::ZERO,
                        ttfb: None,
                        chunk_intervals: vec![],
                        total_chunks: 0,
                        error_category: ErrorCategory::Network,
                    });
                }
                Err(_) => { /* warmup error, skip */ }
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
