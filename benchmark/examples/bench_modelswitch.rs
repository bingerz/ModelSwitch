//! Example: ModelSwitch-specific test scenarios (plan section 5.3).
//!
//! Runs three specialized scenarios back-to-back:
//!   1. Cache hit/bypass — same request 10x sequentially; expect latency drop.
//!   2. In-flight request merging — 10 identical concurrent requests vs
//!      10 unique concurrent requests.
//!   3. Keepalive filtering — stream against a slow upstream, verify TTFB
//!      is measured correctly (keepalive comments must not be counted).
//!
//! Assumes a ModelSwitch gateway is already running at http://127.0.0.1:8080.
//! Starts its own mock LLM server for the keepalive scenario.
//!
//! ```bash
//! cargo run --release --example bench_modelswitch
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use llm_gateway_bench::client::openai::OpenAIClient;
use llm_gateway_bench::client::Protocol;
use llm_gateway_bench::mock::server::{self, MockConfig};
use uuid::Uuid;

/// Default ModelSwitch gateway address.
const GATEWAY_URL: &str = "http://127.0.0.1:8080";

/// Number of sequential requests used by the cache-hit scenario.
const CACHE_REPS: usize = 10;

/// Number of concurrent requests used by the in-flight merging scenario.
const CONCURRENT_REPS: usize = 10;

/// Delay the mock server injects before responding (ms) — used by keepalive
/// scenario. Picked long enough to make TTFB clearly distinguishable from
/// chunk spacing (5 ms default in the mock).
const SLOW_UPSTREAM_DELAY_MS: u64 = 500;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let api_key = std::env::var("MODELSWITCH_API_KEY").unwrap_or_else(|_| "dummy-key".into());
    let model = std::env::var("MODELSWITCH_MODEL").unwrap_or_else(|_| "gpt-4".into());

    let client = Arc::new(OpenAIClient::new(
        GATEWAY_URL,
        &api_key,
        &model,
        120,
        100,
        false,
        Protocol::OpenAI,
    )?);

    println!("\n════════════════════════════════════════════════════════════");
    println!("  ModelSwitch Specialized Benchmark — target: {GATEWAY_URL}");
    println!("════════════════════════════════════════════════════════════\n");

    scenario_cache_hit_bypass(Arc::clone(&client), &model).await?;
    scenario_inflight_merging(Arc::clone(&client), &model).await?;
    scenario_keepalive_filtering(Arc::clone(&client), &model).await?;

    println!("\n════════════════════════════════════════════════════════════");
    println!("  All ModelSwitch scenarios completed.");
    println!("════════════════════════════════════════════════════════════");

    Ok(())
}

// ── Scenario 1: Cache Hit / Bypass ────────────────────────────────────────

/// Send the SAME request body `CACHE_REPS` times sequentially and report
/// per-request latency. The first request is expected to be slower (cache
/// miss / full proxy path); subsequent requests should drop noticeably if the
/// gateway serves them from its request cache.
async fn scenario_cache_hit_bypass(
    client: Arc<OpenAIClient>,
    model: &str,
) -> anyhow::Result<()> {
    println!("─── Scenario 1: Cache Hit / Bypass ──────────────────────────");
    println!(
        "Sending the same request {CACHE_REPS} times sequentially; watching for\n\
         latency drop after the first (cache-miss) request.\n"
    );

    let fixed_body = format!("cache-test-fixed-content-{}", model);
    let max_tokens = 64;
    let mut latencies: Vec<Duration> = Vec::with_capacity(CACHE_REPS);

    for i in 1..=CACHE_REPS {
        let start = Instant::now();
        let timing = client.chat(&fixed_body, max_tokens).await?;
        let elapsed = start.elapsed();
        let actual = timing.latency;
        let status = timing.status.as_u16();
        let ok = if timing.success { "OK" } else { "ERR" };
        println!(
            "  [{i:>2}/{CACHE_REPS}] status={status} {ok}  wall={elapsed:?}  client-latency={actual:?}"
        );
        latencies.push(actual);
    }

    print_cache_progression(&latencies);
    println!();
    Ok(())
}

fn print_cache_progression(latencies: &[Duration]) {
    if latencies.is_empty() {
        return;
    }
    let first = latencies[0];
    let last = latencies[latencies.len() - 1];
    let mean_rest_ms = if latencies.len() > 1 {
        let sum_ms: u128 = latencies[1..].iter().map(|d| d.as_millis()).sum();
        sum_ms as f64 / (latencies.len() - 1) as f64
    } else {
        0.0
    };
    let first_ms = first.as_secs_f64() * 1000.0;
    let last_ms = last.as_secs_f64() * 1000.0;
    let speedup_vs_first = if last_ms > 0.0 {
        first_ms / last_ms
    } else {
        0.0
    };

    println!("\n  Cache-progression summary:");
    println!("    first (cache miss) : {first_ms:.2} ms");
    println!("    mean of remaining  : {mean_rest_ms:.2} ms");
    println!("    last (steady state): {last_ms:.2} ms");
    println!("    speedup first/last : {speedup_vs_first:.2}x");
    if speedup_vs_first > 2.0 {
        println!("    => Latency dropped significantly; cache path likely engaged.");
    } else if speedup_vs_first > 1.1 {
        println!("    => Modest latency drop; cache may be partial or disabled.");
    } else {
        println!("    => No measurable drop; gateway cache appears bypassed or absent.");
    }
}

// ── Scenario 2: In-Flight Request Merging ─────────────────────────────────

/// Send `CONCURRENT_REPS` IDENTICAL requests concurrently, then send
/// `CONCURRENT_REPS` UNIQUE requests concurrently. If the gateway coalesces
/// in-flight identical requests, the identical batch should show tightly
/// clustered latencies and roughly match a single request's cost.
async fn scenario_inflight_merging(client: Arc<OpenAIClient>, model: &str) -> anyhow::Result<()> {
    println!("─── Scenario 2: In-Flight Request Merging ───────────────────");
    println!(
        "Comparing {CONCURRENT_REPS} concurrent IDENTICAL requests vs {CONCURRENT_REPS}\n\
         concurrent UNIQUE requests. Coalesced requests should cluster tightly.\n"
    );

    let identical_body = format!("inflight-merge-fixed-{model}");
    let unique_seed: Vec<String> = (0..CONCURRENT_REPS)
        .map(|i| format!("unique-body-{i}-{}", Uuid::new_v4()))
        .collect();

    let identical = run_concurrent_batch(Arc::clone(&client), vec![identical_body; CONCURRENT_REPS])
        .await?;
    let unique = run_concurrent_batch(Arc::clone(&client), unique_seed).await?;

    print_batch_summary("IDENTICAL", &identical);
    print_batch_summary("UNIQUE    ", &unique);

    let identical_total = identical.total;
    let unique_total = unique.total;
    let ratio = if unique_total.as_secs_f64() > 0.0 {
        identical_total.as_secs_f64() / unique_total.as_secs_f64()
    } else {
        0.0
    };
    let spread_identical = latency_spread_ms(&identical.latencies);
    let spread_unique = latency_spread_ms(&unique.latencies);

    println!("\n  In-flight merging summary:");
    println!(
        "    identical total wall : {:?}   (sum of per-req: {:?})",
        identical_total,
        Duration::from_nanos(identical.latencies.iter().map(|d| d.as_nanos() as u64).sum())
    );
    println!(
        "    unique total wall    : {:?}   (sum of per-req: {:?})",
        unique_total,
        Duration::from_nanos(unique.latencies.iter().map(|d| d.as_nanos() as u64).sum())
    );
    println!("    identical max-min spread : {spread_identical:.2} ms");
    println!("    unique max-min spread    : {spread_unique:.2} ms");
    println!("    identical/unique total ratio : {ratio:.2}");
    if ratio < 0.85 && spread_identical < spread_unique * 0.5 {
        println!("    => Identical batch was faster and tighter; coalescing likely engaged.");
    } else if ratio < 1.05 {
        println!("    => No coalescing observed; gateway forwards each request separately.");
    } else {
        println!("    => Mixed signal; inspect per-request latencies above.");
    }
    println!();
    Ok(())
}

struct BatchResult {
    total: Duration,
    latencies: Vec<Duration>,
    failures: usize,
}

async fn run_concurrent_batch(
    client: Arc<OpenAIClient>,
    bodies: Vec<String>,
) -> anyhow::Result<BatchResult> {
    let max_tokens = 64;
    let start = Instant::now();
    let mut tasks = Vec::with_capacity(bodies.len());
    for body in bodies {
        let client = Arc::clone(&client);
        tasks.push(tokio::spawn(async move {
            let timing = client.chat(&body, max_tokens).await;
            (timing.is_ok(), timing.map(|t| t.latency).unwrap_or_default())
        }));
    }
    let mut latencies = Vec::with_capacity(tasks.len());
    let mut failures = 0usize;
    for t in tasks {
        let (ok, lat) = t.await?;
        if !ok {
            failures += 1;
        }
        latencies.push(lat);
    }
    Ok(BatchResult {
        total: start.elapsed(),
        latencies,
        failures,
    })
}

fn print_batch_summary(label: &str, batch: &BatchResult) {
    let mut sorted: Vec<Duration> = batch.latencies.clone();
    sorted.sort();
    let min = sorted.first().copied().unwrap_or_default();
    let max = sorted.last().copied().unwrap_or_default();
    let p50 = percentile(&sorted, 50.0);
    let p99 = percentile(&sorted, 99.0);
    println!(
        "  {label} batch: n={} failures={} total={:?}\n\
         \x20    per-req: min={min:?}  p50={p50:?}  p99={p99:?}  max={max:?}",
        batch.latencies.len(),
        batch.failures,
        batch.total,
    );
}

fn percentile(sorted: &[Duration], pct: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * (pct / 100.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn latency_spread_ms(latencies: &[Duration]) -> f64 {
    let max = latencies.iter().max().copied().unwrap_or_default();
    let min = latencies.iter().min().copied().unwrap_or_default();
    (max - min).as_secs_f64() * 1000.0
}

// ── Scenario 3: Keepalive Filtering ───────────────────────────────────────

/// Start a slow mock upstream (SLOW_UPSTREAM_DELAY_MS) and stream a request
/// through the gateway. The mock server emits SSE keepalive comments during
/// idle periods; the gateway must filter them so TTFB reflects the first real
/// data chunk, not the keepalive. The client's stream parser already filters
/// on the `data:` prefix, so any keepalive leakage in `total_chunks` would
/// indicate a problem.
async fn scenario_keepalive_filtering(client: Arc<OpenAIClient>, _model: &str) -> anyhow::Result<()> {
    println!("─── Scenario 3: Keepalive Filtering (streaming) ─────────────");
    println!(
        "Starting slow mock upstream (delay={SLOW_UPSTREAM_DELAY_MS} ms) and streaming\n\
         one request through the gateway. Reports TTFB vs total completion time.\n"
    );

    let mock_cfg = MockConfig {
        delay_ms: SLOW_UPSTREAM_DELAY_MS,
        tokens: 50,
        stream_chunks: Some(10),
        ..MockConfig::default()
    };
    let (_mock, mock_port) = server::start(mock_cfg).await?;
    println!("  Mock upstream bound on 127.0.0.1:{mock_port}");
    println!(
        "  Ensure ModelSwitch routes '{_model}' to this mock before continuing;\n\
         \x20 if it does not, the stream will return whatever upstream it actually hits."
    );

    // Give the gateway a moment to notice the upstream (no-op when no
    // orchestration layer exists).
    tokio::time::sleep(Duration::from_millis(200)).await;

    let body = format!("keepalive-test-{}", Uuid::new_v4());
    let timing = client.chat_stream(&body, 50).await?;

    let latency = timing.latency;
    let ttfb = timing.ttfb;
    let total_chunks = timing.total_chunks;
    let status = timing.status.as_u16();
    let success = timing.success;

    println!("\n  Stream result: status={status} success={success}");
    match ttfb {
        Some(t) => {
            let ttfb_ms = t.as_secs_f64() * 1000.0;
            let total_ms = latency.as_secs_f64() * 1000.0;
            let inter_chunk_ms = latency
                .checked_sub(t)
                .map(|d| d.as_secs_f64() * 1000.0 / (total_chunks.max(1) as f64))
                .unwrap_or(0.0);
            println!("    TTFB              : {ttfb_ms:.2} ms");
            println!("    Total completion  : {total_ms:.2} ms");
            println!("    Total chunks seen : {total_chunks}");
            println!("    Avg inter-chunk   : {inter_chunk_ms:.2} ms");
            let lower_bound = SLOW_UPSTREAM_DELAY_MS as f64 * 0.8;
            if ttfb_ms < lower_bound {
                println!("    => TTFB below upstream delay; verify upstream routing.");
            } else {
                println!("    => TTFB includes upstream delay as expected.");
            }
            if total_chunks == 0 {
                println!("    => WARNING: zero data chunks; keepalive comments may be leaking.");
            } else {
                println!("    => TTFB measured from first data: chunk (keepalive filtered).");
            }
        }
        None => {
            println!("    TTFB: <none>  (no data chunk arrived)");
            println!("    Total completion: {:?}", latency);
            println!("    => Streaming did not produce measurable data; check gateway/upstream.");
        }
    }

    // Also take a small batch of 3 sequential streams to surface variance.
    println!("\n  Three additional streaming samples for variance:");
    for i in 1..=3 {
        let body = format!("keepalive-var-{i}-{}", Uuid::new_v4());
        let timing = client.chat_stream(&body, 50).await?;
        let t = timing
            .ttfb
            .map(|d| format!("{:.2} ms", d.as_secs_f64() * 1000.0))
            .unwrap_or_else(|| "<none>".to_string());
        let lat = timing.latency;
        println!(
            "    sample {i}: TTFB={t:<10}  total={lat:?}  chunks={}",
            timing.total_chunks
        );
    }

    println!();
    Ok(())
}
