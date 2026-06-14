use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::ScenarioConfig;
use crate::client::openai::OpenAIClient;
use crate::metrics::summary::{ErrorCategory, RequestResult, StreamResult};

/// Abort the benchmark loop after this many consecutive failures.
const MAX_CONSECUTIVE_FAILURES: u32 = 200;

/// Outcome of a mixed run — holds both non-streaming and streaming results.
pub struct MixedOutcome {
    pub chat_results: Vec<RequestResult>,
    pub stream_results: Vec<StreamResult>,
}

/// Run the mixed scenario — combines streaming and non-streaming traffic.
/// `stream_ratio` is the percentage of streaming requests (0-100).
pub async fn run(
    client: Arc<OpenAIClient>,
    config: ScenarioConfig,
    stream_ratio: u8,
) -> anyhow::Result<MixedOutcome> {
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let chat_results = Arc::new(tokio::sync::Mutex::new(Vec::<RequestResult>::new()));
    let stream_results = Arc::new(tokio::sync::Mutex::new(Vec::<StreamResult>::new()));

    let rate_limiter = config.rps.map(|rps| {
        use governor::{Quota, RateLimiter};
        let quota = Quota::per_second(std::num::NonZeroU32::new(rps).expect("rps > 0"));
        Arc::new(RateLimiter::direct(quota))
    });

    let deadline = Instant::now() + Duration::from_secs(config.duration_secs);
    let warmup_deadline = Instant::now() + Duration::from_secs(config.warmup_secs);

    let consecutive_failures = Arc::new(AtomicU32::new(0));
    let mut tasks = Vec::new();
    let mut request_index = 0u64;

    while Instant::now() < deadline {
        if consecutive_failures.load(Ordering::Relaxed) >= MAX_CONSECUTIVE_FAILURES {
            tracing::warn!(
                "Mixed: aborting early after {} consecutive failures (circuit breaker?)",
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
        let chat_results = Arc::clone(&chat_results);
        let stream_results = Arc::clone(&stream_results);
        let fail_counter = Arc::clone(&consecutive_failures);
        let max_tokens = config.max_tokens;
        let message_tokens = config.message_tokens;

        // Decide stream vs non-stream based on ratio
        let is_stream = (request_index % 100) < stream_ratio as u64;
        request_index += 1;

        tasks.push(tokio::spawn(async move {
            let content = format!("{} {}", "word ".repeat(message_tokens), Uuid::new_v4());
            let is_warmup = Instant::now() <= warmup_deadline;
            let mut task_success = false;

            if is_stream {
                match client.chat_stream(&content, max_tokens).await {
                    Ok(t) if !is_warmup => {
                        task_success = t.success;
                        stream_results.lock().await.push(StreamResult {
                            status: t.status.as_u16(),
                            success: t.success,
                            latency: t.latency,
                            ttfb: t.ttfb,
                            chunk_intervals: t.chunk_intervals,
                            total_chunks: t.total_chunks,
                            error_category: t.error_category,
                        });
                    }
                    Ok(t) if is_warmup => {
                        task_success = t.success;
                    }
                    _ => {}
                }
            } else {
                let timing = client.chat(&content, max_tokens).await.unwrap_or(
                    crate::client::openai::RequestTiming {
                        status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                        success: false,
                        latency: Duration::ZERO,
                        conn_latency: Duration::ZERO,
                        error_category: ErrorCategory::Network,
                    },
                );
                task_success = timing.success;

                if !is_warmup {
                    chat_results.lock().await.push(RequestResult {
                        status: timing.status.as_u16(),
                        success: timing.success,
                        latency: timing.latency,
                        conn_latency: timing.conn_latency,
                        error_category: timing.error_category,
                    });
                }
            }

            if task_success {
                fail_counter.store(0, Ordering::Relaxed);
            } else {
                fail_counter.fetch_add(1, Ordering::Relaxed);
            }

            drop(permit);
        }));
    }

    for t in tasks {
        let _ = t.await;
    }

    let chat_results = Arc::try_unwrap(chat_results)
        .map_err(|_| anyhow::anyhow!("chat_results still shared"))?
        .into_inner();
    let stream_results = Arc::try_unwrap(stream_results)
        .map_err(|_| anyhow::anyhow!("stream_results still shared"))?
        .into_inner();

    Ok(MixedOutcome {
        chat_results,
        stream_results,
    })
}
