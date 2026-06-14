# llm-gateway-bench

A standalone benchmark tool for LLM proxy gateways. Measures latency, throughput, streaming quality, and proxy overhead for any OpenAI/Anthropic-compatible endpoint.

## Features

- **5 test scenarios**: chat, streaming (SSE), burst, sustained RPS, mixed
- **Multi-protocol**: OpenAI (`/v1/chat/completions`) and Anthropic (`/v1/messages`)
- **Built-in mock server**: No API key needed — controllable delay, fail rate, token count, stream chunks
- **Compare mode**: Direct vs proxy overhead analysis
- **Multi-run variance**: Repeat benchmarks with P95 mean ± stddev reporting
- **Streaming metrics**: TTFB (Time To First Byte), chunk interval distribution, chunks/request
- **Dual report output**: JSON and Markdown

## Quick Start

```bash
# Benchmark with built-in mock (no API key needed)
cargo run -- --mock-upstream --mock-delay 50 --scenario streaming --duration 10 --concurrency 20

# Benchmark a real gateway
cargo run -- --target http://127.0.0.1:8080 --api-key sk-xxx --model gpt-4 --scenario mixed --duration 60

# Compare direct vs proxy overhead
cargo run -- compare --direct https://api.openai.com --proxy http://127.0.0.1:8080 --scenario streaming --duration 30

# Multiple runs with variance analysis
cargo run -- --mock-upstream --scenario chat --duration 10 --runs 5 --report json:results.json
```

## CLI Reference

### Global Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `--target <URL>` | `""` | Target gateway URL (required unless `--mock-upstream`) |
| `--scenario <NAME>` | `chat` | Test scenario: `chat` / `streaming` / `mixed` / `burst` / `sustained` |
| `--concurrency <N>` | `10` | Concurrent connections |
| `--duration <SECS>` | `30` | Test duration in seconds |
| `--warmup <SECS>` | `5` | Warmup time (not counted in stats) |
| `--rps <N>` | unlimited | Target RPS (rate-limited mode with token bucket) |
| `--api-key <KEY>` | `dummy` | API key for the target gateway |
| `--model <NAME>` | `gpt-4` | Model name in requests |
| `--timeout <SECS>` | `120` | Request timeout |
| `--protocol <NAME>` | `openai` | Protocol format: `openai` / `anthropic` |
| `--message-tokens <N>` | `10` | Generated message length (word count) |
| `--burst-size <N>` | `200` | Burst size (for burst scenario) |
| `--mix-ratio <PCT>` | `70` | Mixed scenario stream ratio percentage (0-100) |
| `--runs <N>` | `1` | Number of repeated runs for variance analysis |
| `--pool-max-idle <N>` | `100` | Connection pool max idle per host |
| `--tls-skip-verify` | false | Skip TLS certificate verification |
| `--stream` | false | Force streaming for all scenarios |
| `--report <FMT:PATH>` | none | Report output: `json:path.json` or `markdown:path.md` (comma-sep for multiple) |

### Mock Server Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `--mock-upstream` | false | Start built-in mock LLM server as upstream |
| `--mock-port <PORT>` | `0` | Mock server port (0 = random) |
| `--mock-delay <MS>` | `200` | Mock LLM processing delay |
| `--mock-fail-rate <PCT>` | `0` | Mock 429 fail rate percentage |
| `--mock-tokens <N>` | `100` | Mock response token count |
| `--mock-stream-chunks <N>` | `0` | Mock streaming chunk count (0 = derive from tokens) |

### Compare Subcommand

```bash
gateway-bench compare --direct <URL> --proxy <URL> [options]
```

| Argument | Default | Description |
|----------|---------|-------------|
| `--direct <URL>` | required | Direct upstream URL |
| `--proxy <URL>` | required | Proxy gateway URL |
| `--scenario <NAME>` | `chat` | Test scenario |
| `--concurrency <N>` | `10` | Concurrent connections |
| `--duration <SECS>` | `30` | Duration per phase |
| `--warmup <SECS>` | `5` | Warmup seconds |
| `--mock-tokens <N>` | `100` | Mock response tokens |
| `--report <PATH>` | none | Report output path (.json or .md) |

## Scenarios

| Scenario | Description | Key Metrics |
|----------|-------------|-------------|
| **chat** | Non-streaming `/v1/chat/completions` | P50/P95/P99 latency, RPS, error rate |
| **streaming** | SSE streaming with `stream=true` | TTFB, chunk intervals, total chunks |
| **burst** | N concurrent requests fired instantly | Peak latency, burst RPS |
| **sustained** | Constant RPS over duration | Steady-state latency, dropped requests |
| **mixed** | Combined streaming + non-streaming | Both latency and streaming metrics |

## Output Metrics

### Latency
- P50, P95, P99, Mean, Min, Max

### Streaming
- **TTFB** (Time To First Byte): Request sent → first SSE data chunk
- **Chunk Interval**: Time between consecutive SSE data chunks
- **Avg Chunks/Request**: Total data chunks received per request

### Errors
- 4xx errors (including 429 rate limits)
- 5xx server errors
- Timeouts
- Network errors (connection failures)

### Compare Mode
- Latency overhead (P50/P95/P99 delta)
- TTFB overhead
- RPS loss percentage
- Error rate delta

## Examples

```bash
# Examples are in benchmark/examples/

cargo run --example bench_modelswitch    # Benchmark a ModelSwitch gateway
cargo run --example bench_direct         # Direct baseline measurement
cargo run --example compare_overhead     # Auto compare direct vs proxy
```

## Self-Benchmarks

```bash
# Verify the tool itself isn't a bottleneck
cargo bench
```

Measures histogram recording, summary building, JSON serialization, and SSE parsing overhead.

## Architecture

```
gateway-bench
├── client/     HTTP client (OpenAI + Anthropic protocol, SSE streaming)
├── scenarios/  5 load generation scenarios
├── mock/       Built-in mock LLM server (axum)
├── metrics/    hdrhistogram latency + summary aggregation
├── report/     Markdown + JSON + compare report generators
└── runner/     Workload orchestration with multi-run support
```

## License

MIT
