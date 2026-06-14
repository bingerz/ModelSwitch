//! Criterion self-benchmarks — verify the benchmark tool itself isn't a bottleneck.
//!
//! Run with: `cargo bench`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use llm_gateway_bench::metrics::histogram::LatencyHistogram;
use llm_gateway_bench::metrics::summary::{
    ErrorCategory, RequestResult, StreamResult, SummaryBuilder,
};
use serde_json::json;
use std::time::Duration;

/// Benchmark hdrhistogram recording at various batch sizes.
fn bench_histogram_recording(c: &mut Criterion) {
    let mut group = c.benchmark_group("histogram");
    group.throughput(Throughput::Elements(1));

    for batch_size in [100, 1_000, 10_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("record_{}", batch_size)),
            &batch_size,
            |b, &size| {
                b.iter(|| {
                    let mut hist = LatencyHistogram::new();
                    for i in 0..size {
                        hist.record(Duration::from_micros(i as u64));
                    }
                    black_box(hist.percentile_ms(95.0));
                });
            },
        );
    }
    group.finish();
}

/// Benchmark summary building from request results.
fn bench_summary_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("summary");

    for count in [100, 1_000, 10_000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("build_{}", count)),
            &count,
            |b, &n| {
                // Pre-build results
                let results: Vec<RequestResult> = (0..n)
                    .map(|i| RequestResult {
                        status: 200,
                        success: true,
                        latency: Duration::from_micros(1_000 + i as u64 % 50_000),
                        conn_latency: Duration::from_micros(100),
                        error_category: ErrorCategory::None,
                    })
                    .collect();

                b.iter(|| {
                    let mut builder =
                        SummaryBuilder::new(Duration::from_secs(10)).with_scenario("chat");
                    for r in &results {
                        builder.add_result(r.clone());
                    }
                    black_box(builder.build());
                });
            },
        );
    }
    group.finish();
}

/// Benchmark summary building with streaming results (includes TTFB + chunk intervals).
fn bench_summary_stream_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("summary_stream");

    for count in [100, 1_000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("stream_build_{}", count)),
            &count,
            |b, &n| {
                let results: Vec<StreamResult> = (0..n)
                    .map(|i| StreamResult {
                        status: 200,
                        success: true,
                        latency: Duration::from_millis(50 + i as u64 % 200),
                        ttfb: Some(Duration::from_millis(5 + i as u64 % 30)),
                        chunk_intervals: (0..20)
                            .map(|j| Duration::from_millis(3 + j % 10))
                            .collect(),
                        total_chunks: 20,
                        error_category: ErrorCategory::None,
                    })
                    .collect();

                b.iter(|| {
                    let mut builder =
                        SummaryBuilder::new(Duration::from_secs(10)).with_scenario("streaming");
                    for r in &results {
                        builder.add_stream_result(r.clone());
                    }
                    black_box(builder.build());
                });
            },
        );
    }
    group.finish();
}

/// Benchmark JSON request body serialization (as the client does).
fn bench_json_serialize(c: &mut Criterion) {
    c.bench_function("json_serialize_request", |b| {
        b.iter(|| {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "word word word word word test-uuid-1234"}],
                "max_tokens": 100,
                "stream": false,
            });
            black_box(serde_json::to_string(&body).unwrap());
        });
    });

    c.bench_function("json_serialize_stream_request", |b| {
        b.iter(|| {
            let body = json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "word word word word word test-uuid-1234"}],
                "max_tokens": 100,
                "stream": true,
            });
            black_box(serde_json::to_string(&body).unwrap());
        });
    });
}

/// Benchmark SSE chunk parsing (simulates the stream parsing loop).
fn bench_sse_parsing(c: &mut Criterion) {
    // Simulate a typical SSE chunk as received from the wire
    let chunk_data = "data: {\"id\":\"chatcmpl-abc\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lorem ipsum \"},\"finish_reason\":null}]}\n\n";

    c.bench_function("sse_chunk_contains_data", |b| {
        b.iter(|| {
            let text = String::from_utf8_lossy(black_box(chunk_data.as_bytes()));
            let has_data = text.contains("data:");
            black_box(has_data);
        });
    });
}

criterion_group!(
    benches,
    bench_histogram_recording,
    bench_summary_build,
    bench_summary_stream_build,
    bench_json_serialize,
    bench_sse_parsing,
);
criterion_main!(benches);
