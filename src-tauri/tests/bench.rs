//! Gateway dispatch pipeline benchmark.
//!
//! Measures raw dispatch throughput and latency against a wiremock upstream.
//! Run with:
//!
//! ```sh
//! cargo test --release --test bench -- --ignored --nocapture
//! ```
//!
//! `#[ignore]` keeps it out of normal CI — these are long-running, representative
//! only in release mode.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use model_switch_lib::proxy::AppState;
use model_switch_lib::test_helpers::{build_test_state, channel_config, dispatch_openai_chat};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WARMUP_REQS: usize = 50;
const SEQ_REQS: usize = 200;
const CONCURRENT_REQS: usize = 200;
const CONCURRENCY: usize = 10;

fn chat_body(i: usize) -> Value {
    json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": format!("benchmark-{i}")}],
        "max_tokens": 50
    })
}

fn chat_completion_response() -> String {
    json!({
        "id": "chatcmpl-bench",
        "object": "chat.completion",
        "created": 1234567890_u64,
        "model": "gpt-4",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    })
    .to_string()
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[tokio::test]
#[ignore]
async fn bench_dispatch_throughput() {
    // ── Setup ──────────────────────────────────────────────────────────────

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(chat_completion_response()),
        )
        .mount(&mock_server)
        .await;

    let state: Arc<AppState> = build_test_state(vec![channel_config(
        "00000000-0000-0000-0000-000000000001",
        "bench-channel",
        &mock_server.uri(),
        1,
    )]);

    // Default rate limiter caps at 60 RPM — raise it so the benchmark isn't
    // measuring the limiter.
    let ch_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    state
        .limits
        .rate_limiter
        .set_channel_rpm_limit(ch_id, 1_000_000);

    let headers = HeaderMap::new();

    // ── Warmup ─────────────────────────────────────────────────────────────

    for i in 0..WARMUP_REQS {
        let body = chat_body(i);
        let _ = dispatch_openai_chat(&state, &headers, &body).await;
    }

    // ── Sequential ─────────────────────────────────────────────────────────

    let mut seq_latencies = Vec::with_capacity(SEQ_REQS);
    let seq_start = Instant::now();

    for i in 0..SEQ_REQS {
        let body = chat_body(i);
        let t0 = Instant::now();
        let resp = dispatch_openai_chat(&state, &headers, &body).await;
        seq_latencies.push(t0.elapsed());
        assert_eq!(
            resp.status().as_u16(),
            200,
            "sequential request {} returned non-200",
            i
        );
    }

    let seq_total = seq_start.elapsed();
    let seq_avg: Duration =
        seq_latencies.iter().sum::<Duration>() / seq_latencies.len() as u32;
    let seq_qps = SEQ_REQS as f64 / seq_total.as_secs_f64();

    // ── Concurrent ─────────────────────────────────────────────────────────

    let conc_start = Instant::now();

    // Process CONCURRENT_REQS in batches of CONCURRENCY to bound parallelism.
    let mut conc_latencies: Vec<Duration> = Vec::with_capacity(CONCURRENT_REQS);
    let mut batch = Vec::with_capacity(CONCURRENCY);

    for i in 0..CONCURRENT_REQS {
        let st = Arc::clone(&state);
        let hdrs = headers.clone();
        let body = chat_body(i + SEQ_REQS); // unique bodies to avoid cache
        batch.push(tokio::spawn(async move {
            let t0 = Instant::now();
            let resp = dispatch_openai_chat(&st, &hdrs, &body).await;
            let lat = t0.elapsed();
            assert_eq!(resp.status().as_u16(), 200, "concurrent request {i} failed");
            lat
        }));

        if batch.len() == CONCURRENCY {
            let results = futures::future::join_all(batch.drain(..)).await;
            for r in results {
                conc_latencies.push(r.unwrap());
            }
        }
    }
    // Drain remaining
    if !batch.is_empty() {
        let results = futures::future::join_all(batch.drain(..)).await;
        for r in results {
            conc_latencies.push(r.unwrap());
        }
    }

    let conc_total = conc_start.elapsed();
    conc_latencies.sort();

    let conc_avg: Duration = conc_latencies.iter().sum::<Duration>() / conc_latencies.len() as u32;
    let conc_p50 = percentile(&conc_latencies, 0.50);
    let conc_p95 = percentile(&conc_latencies, 0.95);
    let conc_p99 = percentile(&conc_latencies, 0.99);
    let conc_qps = CONCURRENT_REQS as f64 / conc_total.as_secs_f64();

    // ── Report ─────────────────────────────────────────────────────────────

    println!();
    println!("=== ModelSwitch Gateway Benchmark ===");
    println!();
    println!("Sequential ({SEQ_REQS} requests):");
    println!("  Total: {:.2?}", seq_total);
    println!("  Avg latency: {:.2?}", seq_avg);
    println!("  QPS: {:.1}", seq_qps);
    println!();
    println!(
        "Concurrent ({CONCURRENT_REQS} requests, {CONCURRENCY} workers):"
    );
    println!("  Total: {:.2?}", conc_total);
    println!("  Avg latency: {:.2?}", conc_avg);
    println!("  p50: {:.2?}", conc_p50);
    println!("  p95: {:.2?}", conc_p95);
    println!("  p99: {:.2?}", conc_p99);
    println!("  QPS: {:.1}", conc_qps);
}
