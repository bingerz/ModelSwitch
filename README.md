# ModelSwitch

A Rust-based LLM intelligent gateway with multi-provider proxy, routing strategies, circuit breaker, quota tracking, virtual keys, and MCP gateway capabilities.

Built on Tauri + Axum 0.8, runs as a desktop app (Tauri) or headless CLI.

## Features

- **Multi-provider proxy** - OpenAI, Anthropic, Gemini, DeepSeek, OpenRouter, and custom providers behind a unified OpenAI/Anthropic-compatible API surface
- **Routing strategies** - Weighted random, latency-based, least-busy, and session affinity channel selection
- **Circuit breaker** - Sliding-window failure detection with Healthy / CircuitOpen / HalfOpen state machine and automatic recovery probing
- **Model fallback chains** - Configure fallback model sequences so requests retry on alternatives when the primary model fails
- **Quota tracking** - Passive token accumulation from API responses plus active quota polling from provider dashboards
- **Virtual API keys** - Issue per-agent keys with daily and monthly spend caps; constant-time validation via SHA-256 hash comparison
- **MCP tool auto-injection** - Aggregate tools from running MCP servers, inject them into chat requests, execute tool calls in an iterative loop, and expose a `/mcp` gateway endpoint
- **Privacy sanitizer** - Redact secrets (API keys, tokens, emails) from request payloads before forwarding upstream
- **SSE streaming** - Full streaming support with keepalive and telemetry extraction (token counts from stream chunks)
- **Request coalescing cache** - Deduplicate identical in-flight non-streaming requests and serve subsequent callers from cache
- **Hot-reload config** - TOML config file watched for changes; channels and MCP servers update without restart
- **Admin REST API** - Full CRUD for channels, MCP servers, virtual keys, payload rules, plus logs, stats, and cost endpoints

## Architecture Overview

```
src-tauri/src/
├── proxy/            Core proxy engine
│   ├── dispatch.rs     Shared dispatch() entry point (retry, fallback, cache)
│   ├── attempt.rs      Single-channel attempt (streaming/JSON, telemetry)
│   ├── openai.rs       OpenAI handler + AppState definition
│   ├── anthropic.rs    Anthropic /v1/messages handler
│   ├── gemini.rs       Gemini v1beta handler
│   ├── stream.rs       SSE streaming + JSON response helpers
│   ├── cache.rs        Request cache + in-flight coalescing
│   ├── rate_limiter.rs Per-channel RPM/TPM limiting
│   ├── payload_rules.rs  Model parameter overrides and field stripping
│   ├── translate.rs    Gemini-to-OpenAI payload translation
│   ├── mcp_tools.rs    MCP tool aggregation and injection
│   └── usage.rs        Token usage extraction from responses/streams
├── channel/          Channel management with circuit breaker state machine
├── router/           Channel selection strategies (weighted, latency, least-busy, affinity)
├── quota/            Quota polling, provider registry, passive token tracking
├── virtual_key/      Virtual API keys with daily/monthly budget caps
├── mcp/              MCP server lifecycle, tool aggregation, translation, /mcp gateway
├── config/           TOML config loading + hot-reload watcher
├── middleware/       Auth, sanitizer, virtual key validation, request ID
├── credential/       Credential store (system keyring or file-based)
├── health/           Background health checker
├── admin.rs          Admin REST API (channels, logs, stats, MCP, virtual keys)
├── error.rs          Unified GatewayError type
├── persisted_store.rs  Generic JSON-persisted K-V store
└── log.rs            Dispatch logger with NDJSON persistence
```

## Prerequisites

- **Rust toolchain** - Install via [rustup](https://rustup.rs/) (stable channel)
- **Node.js + pnpm** - Required for the frontend (Tauri mode only)
- **Tauri prerequisites** - See [Tauri v2 setup guide](https://v2.tauri.app/start/prerequisites/) for platform-specific system dependencies

## Build and Run

### CLI Mode (headless gateway)

Runs the Axum HTTP gateway without the Tauri desktop shell:

```bash
cargo build --no-default-features
cargo run --no-default-features
```

The CLI binary is `modelswitch-cli` (defined at `src-tauri/src/bin/cli.rs`).

### Tauri Mode (desktop app)

Runs the gateway inside the Tauri desktop application with the React frontend:

```bash
# Install frontend dependencies
pnpm install

# Development mode (hot-reload frontend + gateway)
pnpm tauri dev

# Production build
cargo build --features tauri
```

### Frontend Only

```bash
pnpm install
pnpm dev
```

## Configuration

ModelSwitch reads a TOML config file. The default location is the platform config directory:

| Platform | Path |
|----------|------|
| macOS    | `~/Library/Application Support/modelswitch/config.toml` |
| Linux    | `~/.config/modelswitch/config.toml` |
| Windows  | `%APPDATA%\modelswitch\config.toml` |

Override the path with `--config /path/to/config.toml`.

A fully commented example is at `config.example.toml`. The config contains:

- **`[gateway]`** - Host, port, routing strategy, retries, circuit breaker cooldown, health checks, MCP settings, sanitizer rules, admin token
- **`[gateway.model_fallbacks]`** - Model-to-fallback-chain mapping (e.g. `"gpt-4o" = ["gpt-4o-mini", "gpt-3.5-turbo"]`)
- **`[[channels]]`** - Channel definitions (id, provider, priority, weight, credentials, base URL, model mapping, rate limits, cost rates, payload rules)
- **`[[mcp_servers]]`** - MCP server subprocess definitions (command, args, env, enabled, expose_tools)

Config changes are hot-reloaded: editing the file updates channels and MCP servers without restarting the gateway.

## Testing

```bash
# Unit tests only (fast)
cargo test --lib

# All tests (unit + doc)
cargo test
```

Tests are co-located in each module under `#[cfg(test)] mod tests` blocks.

## License

<!-- TODO: Add license information -->
