use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::client::openai::{OpenAIClient, RequestTiming};
use crate::metrics::summary::{ErrorCategory, RequestResult};

/// Config for burst scenario.
pub struct BurstConfig {
    pub burst_size: usize,
    pub message_tokens: usize,
    pub max_tokens: usize,
}

/// Result of a burst run — includes actual elapsed time for correct RPS.
pub struct BurstOutcome {
    pub results: Vec<RequestResult>,
    pub elapsed: Duration,
}

/// Run the burst scenario — fire N concurrent requests instantly.
/// Returns results + actual wall-clock elapsed time for accurate RPS.
pub async fn run_burst(
    client: Arc<OpenAIClient>,
    config: BurstConfig,
) -> anyhow::Result<BurstOutcome> {
    let burst_size = config.burst_size;
    let semaphore = Arc::new(Semaphore::new(burst_size));
    let results = Arc::new(tokio::sync::Mutex::new(Vec::<RequestResult>::new()));

    tracing::info!("Burst: firing {} concurrent requests", burst_size);

    let start = Instant::now();
    let mut tasks = Vec::with_capacity(burst_size);

    for _ in 0..burst_size {
        let permit = semaphore.clone().acquire_owned().await?;
        let client = Arc::clone(&client);
        let results = Arc::clone(&results);
        let max_tokens = config.max_tokens;

        tasks.push(tokio::spawn(async move {
            let content = format!("burst {}", Uuid::new_v4());

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

            results.lock().await.push(RequestResult {
                status: timing.status.as_u16(),
                success: timing.success,
                latency: timing.latency,
                conn_latency: timing.conn_latency,
                error_category: timing.error_category,
            });

            drop(permit);
        }));
    }

    for t in tasks {
        let _ = t.await;
    }

    let elapsed = start.elapsed();
    let results = Arc::try_unwrap(results)
        .map_err(|_| anyhow::anyhow!("results still shared"))?
        .into_inner();

    tracing::info!(
        "Burst complete: {} requests in {}ms",
        results.len(),
        elapsed.as_millis()
    );

    Ok(BurstOutcome { results, elapsed })
}
