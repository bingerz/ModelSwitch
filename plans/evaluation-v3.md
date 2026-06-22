# ModelSwitch Architecture Evaluation V3

> Evaluation date: 2026-06-22
> Codebase: ~21,760 lines across ~89 Rust source files, 387 tests
> Based on analysis of the complete Rust gateway backend at `src-tauri/src/`

---

## Executive Summary

This report documents a codebase that has undergone extensive improvement since the V2 evaluation (prior Phases A-K). The following previously-identified issues have been **confirmed fixed**:

- **BUG-1** (streaming billing double-charge) -- FIXED. `response.rs` now uses `reconcile_spend()` when a reservation exists, `accumulate_spend()` otherwise.
- **BUG-2** (`stream_options` cache poisoning) -- FIXED. `canonical_key_material()` now excludes `stream_options`.
- **BUG-3** (config hot-reload race) -- FIXED. `replace_all()` atomically swaps the channel list with a single write-lock.
- **PERF-1** (HTTP/2 connection pool stalling) -- FIXED. `HttpPool` with least-busy selection and RAII guards.
- **PERF-2** (streaming `read_timeout`) -- FIXED. 120s per-chunk timeout in `stream.rs`.
- **PERF-3** (rate limiter O(n) trimming) -- FIXED. `BucketedWindow` with fixed-size buckets, O(num_buckets) per operation.
- **ARCH-3** (Prometheus metrics) -- FIXED. Full `/metrics` endpoint with labeled counters, histograms, and gauges.
- **FEAT-1** (multi-key channels) -- FIXED. Round-robin rotation in `ChannelManager::get_credential()`.
- **FEAT-2** (lowest cost routing) -- FIXED. `LowestCostStrategy` in `router/strategy.rs`.
- **FEAT-4** (tag-based routing via `X-Account-Group`) -- FIXED.
- **FEAT-5** (channel-level model mapping) -- already existed.

The codebase has improved significantly. However, 8 new issues were identified, including one **P0** dead-code problem and two **P1** issues.

---

## Module Structure Map

### proxy/ (5,756 lines, 121 tests)
| File | Lines | Responsibility | API Surface | Dependencies |
|------|-------|----------------|-------------|--------------|
| `openai.rs` | 349 | AppState struct, chat completions handler, model/tool listing | `AppState`, `handle_chat_completions`, `handle_list_models`, `handle_list_tools`, `health_check` + 6 sub-structs | channel, log, mcp, proxy::{cache,mcp_tools,payload_rules,rate_limiter,stream,dispatch,provider}, quota, router, virtual_key |
| `dispatch.rs` | 769 | Core dispatch loop: channel selection, retry, fallback chains, budget check | `dispatch()` + `select_channel_for_attempt()`, `check_request_cache()`, `resolve_model_retry_config()` | channel, proxy::{attempt,cache,stream,provider,request_meta}, router, virtual_key |
| `attempt.rs` | 568 | Single-channel attempt: body mutation, rate check, HTTP call, success/failure handling | `try_channel_attempt()`, `AttemptOutcome`, `is_context_window_error()` | channel, proxy::{response,provider,stream} |
| `response.rs` | 553 | Streaming and JSON success handlers, passthrough headers | `handle_streaming_success()`, `handle_json_success()`, `extract_passthrough_headers()` | channel, proxy::{stream,provider,usage} |
| `stream.rs` | 443 | SSE streaming, keepalive, Gemini translation | `sse_stream_response_with_telemetry()`, `json_response()`, `keepalive_stream()`, `detect_sse_error()` (dead), `sse_error_event()` (dead) | proxy::translate |
| `provider.rs` | 448 | ProviderAdaptor trait + 3 implementations | `ProviderAdaptor`, `OpenAIAdaptor`, `AnthropicAdaptor`, `GeminiAdaptor`, `sanitize_model_for_url()` | proxy::translate |
| `cache.rs` | 497 | RequestCache, cache modes, in-flight coalescing | `RequestCache`, `InFlightRequests`, `CacheMode`, `canonical_key_material()` | blake3 |
| `rate_limiter.rs` | 314 | Per-channel RPM/TPM + global TPM via bucketed sliding window | `RateLimiter`, `BucketedWindow` | -- |
| `mcp_tools.rs` | 420 | MCP tool injection into chat requests | `inject_mcp_tools()`, `detect_mcp_tool_calls()`, `execute_mcp_tool_calls()` | mcp |
| `translate.rs` | 244 | OpenAI <-> Gemini protocol translation | `openai_to_gemini()`, `gemini_to_openai()`, `gemini_stream_to_openai()` | -- |
| `embeddings.rs` | 375 | Embeddings API handler | `handle_embeddings()` | proxy::{dispatch,provider} |
| `mod.rs` | 278 | Module coordinator, `make_log()`, `validate_chat_request()`, `FailureReason`, `estimate_tokens()` | public re-exports + utility functions | log |
| `anthropic.rs` | 21 | Anthropic request handler | `handle_messages()` | proxy::dispatch |
| `gemini.rs` | 26 | Gemini request handler | `handle_gemini()` | proxy::dispatch |
| `payload_rules.rs` | 153 | Per-channel request field defaults/overrides/strip | `ChannelPayloadRules`, `PayloadRules` | -- |
| `request_meta.rs` | 99 | Request metadata extraction (session, affinity, account_group) | `extract_request_meta()`, `RequestMeta` | proxy::openai |
| `usage.rs` | 199 | Token usage extraction from responses | `extract_usage()`, `extract_usage_from_stream()` | -- |
| `dispatch_tests.rs` | 914 | Integration tests for the dispatch pipeline | N/A (test-only) | -- |

### virtual_key/ (901 lines, 28 tests)
- `mod.rs` -- VirtualKey types, VirtualKeyStore with PersistedStore, reserve/reconcile/accumulate spend, SHA-256 hashing with constant-time comparison, model whitelists
- Public API: `VirtualKeyStore`, `VirtualKey`, `ReserveResult`, `SharedVirtualKeyStore`

### log.rs (572 lines, 2 tests)
- `DispatchLogger` with VecDeque ring buffer, ndjson file persistence, file rotation (10MB threshold), query methods
- Public API: `DispatchLogger`, `DispatchLog`

### server.rs (597 lines, 0 tests)
- `start_gateway_services()` -- 328-line bootstrap that builds the entire gateway state and spawns 8+ background tasks
- `build_router()` -- Axum router construction with all proxy and admin routes, CORS, compression, tracing layers
- `start_gateway()` -- TCP listener, graceful shutdown, PID file management
- Public API: `start_gateway_services()`, `build_router()`, `start_gateway()`, `metrics_handler()`

### config/ (604 lines, 11 tests)
- `config.rs` -- `AppConfig`, `GatewayConfig` with 30+ fields, `ChannelConfig`, `SanitizerConfig`, TOML serialization
- `watcher.rs` -- `start_config_watcher()` with 2s polling, `apply_config_reload()` for atomic channel updates

### admin/ (1,462 lines, 7 tests)
- `system.rs` (351) -- Health, stats, gateway info endpoints
- `channels.rs` (335) -- CRUD for channels, payload rules, ping, circuit breaker reset
- `mcp.rs` (329) -- MCP server management
- `virtual_keys.rs` (171) -- Virtual key CRUD
- `provider_budgets.rs` (146) -- Provider budget management
- `mod.rs` (56) -- Module declaration, re-exports

### router/ (1,388 lines, 40 tests)
- `strategy.rs` (521) -- 5 routing strategies: WeightedRandom, LatencyBased, LeastBusy, LowestCost, UsageBased
- `fallback.rs` (212) -- Fallback chain resolution with wildcard/date-suffix support
- `latency_tracker.rs` (194) -- Per-channel latency and per-token latency tracking
- `affinity.rs` (144) -- Session affinity with TTL expiry
- `active_requests.rs` (~80) -- Per-channel active request tracking
- `weighted.rs` (~80) -- Weighted random selection
- `mod.rs` (151) -- `select_channel()` entry point, `RoutingContext`

### quota/ (3,341 lines, 65 tests)
- `mod.rs` (426) -- `QuotaStore`, `QuotaState` with PersistedStore, TOML persistence
- `poller.rs` (145) -- Background quota polling
- `registry.rs` -- Provider registry for quota collectors
- `webview_scrape.rs` (408) -- WebView-based quota scraping
- `collectors/` -- 12+ collector implementations:
  - `webview.rs` (274), `jsonpath_generic.rs` (251), `deepseek.rs` (195)
  - `response_header.rs` (161), `openai_compat.rs` (156), `zhipu.rs` (155)
  - `minimax.rs` (143), `openrouter.rs` (117), `moonshot.rs` (114)
  - `novita.rs`, `siliconflow.rs`, `stepfun.rs`, `shengsuanyun.rs`
  - Plus `webview_scripts/` for headless browser automation

### mcp/ (1,167 lines, 37 tests)
- `manager.rs` (500) -- MCP subprocess lifecycle management
- `translator.rs` (330) -- MCP tool-to-OpenAI/Anthropic function format conversion
- `gateway.rs` (166) -- Streamable HTTP MCP gateway
- `aggregator.rs` (157) -- Aggregated tool listing across servers

### middleware/ (806 lines, 24 tests)
- `sanitizer.rs` (576) -- PII detection and redaction in request bodies
- `auth.rs` (~70) -- Admin Bearer token authentication
- `virtual_key.rs` (~70) -- Virtual key middleware
- `request_id.rs` (30) -- Request ID header injection
- `error.rs` (41) -- `ApiError` response helper (used by admin endpoints)

### health/ (710 lines, 22 tests)
- `mod.rs` (333) -- `start_health_checker()`, health check logic
- `probe.rs` (377) -- Per-channel TCP/HTTP probe

### channel/ (674 lines, 7 tests)
- `manager.rs` (398) -- ChannelManager with CRUD, circuit breaker, multi-key rotation
- `mod.rs` (276) -- `Channel`, `ChannelStatus`, `Provider`, `Credential` types

### Other
| File | Lines | Purpose |
|------|-------|---------|
| `error.rs` | 170 | `GatewayError` enum with `IntoResponse` -- UNUSED (dead code) |
| `metrics.rs` | 260 | Prometheus metric definitions and `/metrics` render |
| `http_pool.rs` | 234 | Multi-client HTTP connection pool |
| `provider_budget.rs` | 504 | Per-provider budget tracking |
| `persisted_store.rs` | 184 | Generic JSON file persistence |
| `lib.rs` | 176 | Module declarations, Tauri entry point, `spawn_bg()` |
| `tauri_cmds.rs` | 279 | Tauri IPC command handlers |
| `credential/` | ~200 | Credential store trait + file-based implementation |
| `shutdown.rs` | ~50 | Signal-based graceful shutdown |

---

## Issues Found

### P0: GatewayError is entirely dead code (UNCHANGED from previous hidden state)

- **Severity**: P0 -- Critical
- **Location**: `error.rs:1-170`, referenced nowhere outside the file
- **Description**: `GatewayError` is a 170-line typed error enum with 15 variants, `safe_message()` (for information disclosure prevention), `IntoResponse` implementation mapping each variant to HTTP status codes, and `From` conversions for `anyhow::Error` and `serde_json::Error`. It is declared `pub mod error` in `lib.rs:5` but never imported or used by any other module. The proxy handlers return raw `axum::response::Response` via `json_response()`. Admin handlers use `middleware::error::ApiError` which is a separate, simpler type. Every `From` impl, every match arm, every status code mapping is wasted.
- **Recommendation**: Either (a) delete `error.rs` entirely (and its `pub mod error` declaration in `lib.rs`), or (b) migrate all proxy and admin handlers to use `GatewayError` as their error return type, removing the ad-hoc `json_response()` pattern. Option (b) would improve consistency but requires significant refactoring of every handler. Option (a) is the practical fix.

### P1: `server.rs::start_gateway_services` is a 328-line god function

- **Severity**: P1 -- High
- **Location**: `server.rs:42-369`
- **Description**: `start_gateway_services()` loads config, creates the HTTP pool, initializes the channel manager, builds every shared component, constructs `AppState`, spawns 8+ background tasks (log loading, quota persistence, virtual key persistence, provider budget persistence, health checker, quota poller, session affinity cleanup, cache sweep, config watcher), and returns `GatewayHandles`. This single function understands the entire system's startup wiring, making it hard to test, hard to override individual components, and fragile.
- **Previous state**: Not identified in V2 evaluation. New finding.
- **Recommendation**: Decompose into focused builder functions:
  - `build_http_pool(config) -> HttpPool`
  - `init_persistence_tasks(state)` -- spawn all periodic persist/load tasks
  - `init_background_services(state, config)` -- health checker, quota poller, etc.
  - `init_config_watcher(state, config)`

### P1: Zero test coverage on `server.rs`, thin coverage on `admin/`

- **Severity**: P1 -- High
- **Location**: `server.rs` (0 tests, 597 lines), `admin/` module (7 tests, 1,462 lines)
- **Description**: `server.rs` has zero tests. The gateway startup, `build_router()`, and graceful shutdown are never tested in isolation. The admin module has only 7 tests across 1,462 lines. These are among the most critical surfaces -- startup wiring bugs and admin API regressions would only be caught at runtime.
- **Previous state**: Noted in V2 as "key modules zero coverage" (QUAL-1), but not explicitly tied to `server.rs`.
- **Recommendation**: Add integration tests for `build_router()` (test route registration), tests for `start_gateway_services()` with a test config, and tests for graceful shutdown behavior.

### P1: Aggressive `unwrap()` usage in production code

- **Severity**: P1 -- High
- **Location**: `middleware/sanitizer.rs:60-127` (10+ Regex::new().unwrap()), `middleware/request_id.rs:23,28` (HeaderValue::from_str().unwrap()), `proxy/cache.rs:81,90` (try_into().unwrap()), `proxy/stream.rs:218,229,242,269` (Response::builder().body().unwrap())
- **Description**: Static `unwrap()` calls in production (non-test) code for `Regex::new()`, `HeaderValue::from_str()`, and `Response::builder().body()`. While these are effectively infallible in practice (known-good regexes, hardcoded header values), a future change could introduce panics. The `reqwest` header variant `HeaderValue::from_maybe_shared()` or `.expect("...")` would be safer.
- **Previous state**: Not identified as a standalone issue. New finding.
- **Recommendation**: Replace `.unwrap()` with `.expect("...")` to document the invariant, or propagate errors.

### P2: Code duplication in Gemini stream translation (stream.rs)

- **Severity**: P2 -- Medium
- **Location**: `proxy/stream.rs:83-114` and `proxy/stream.rs:152-188`
- **Description**: The Gemini SSE-to-OpenAI translation logic is duplicated verbatim between the "connected client" forward path (lines 83-114) and the "disconnected drain" path (lines 152-188). Approximately 35 lines of near-identical code (split-loop over SSE lines, parse each `data:` line, call `gemini_stream_to_openai()`, reassemble). A comment on line 151 explicitly acknowledges this ("Fixes a pre-existing bug where the drain path accumulated raw upstream bytes instead of translated output for Gemini streams"), indicating this was a post-hoc fix that introduced duplication.
- **Previous state**: New finding.
- **Recommendation**: Extract the Gemini line-by-line translation into a shared function:
  ```rust
  fn translate_gemini_chunk(raw_bytes: &[u8], model: &str) -> Bytes;
  ```
  Then call it from both paths.

### P2: `routing_strategy` is stringly-typed

- **Severity**: P2 -- Medium
- **Location**: `proxy/openai.rs:34` (`pub routing_strategy: String`), `router/mod.rs` (string comparison to select strategy)
- **Description**: Router strategy selection uses string comparison of `"weighted_random"`, `"latency"`, `"least_busy"`, etc. This loses the type safety of an enum -- typos, unknown strategies, and refactoring rename are all silent runtime errors. The strategy names are serialized in config TOML so an enum with serde would work seamlessly.
- **Previous state**: New finding (noted in V2 only for multi-routing features, not the stringly-typed gap).
- **Recommendation**: Define `enum RoutingStrategy { WeightedRandom, Latency, LeastBusy, LowestCost, Usage }` with `Serialize`/`Deserialize`, and use it throughout.

### P2: `drain_timeout_secs` parameter is unused (`_`-prefixed)

- **Severity**: P2 -- Medium
- **Location**: `server.rs:531` (`_drain_timeout_secs: u64`)
- **Description**: The `drain_timeout_secs` parameter in `start_gateway()` is accepted but never used. The graceful shutdown via `axum::serve(listener, app).with_graceful_shutdown(shutdown_fut)` does not apply any drain timeout -- if the shutdown signal comes, the server waits indefinitely for in-flight requests. This means a shutdown during a long-running streaming request could hang the process.
- **Previous state**: New finding.
- **Recommendation**: Implement the drain timeout by wrapping the graceful shutdown in a `tokio::time::timeout`, and force-kill the `Axum` server after the timeout expires.

### P2: `error.rs` (GatewayError) vs `middleware/error.rs` (ApiError) -- two error systems

- **Severity**: P2 -- Medium
- **Location**: `error.rs` (170 lines) vs `middleware/error.rs` (41 lines)
- **Description**: Two separate error response systems exist. `error.rs` has a full typed enum with 15 variants. `middleware/error.rs` has a simpler `ApiError` struct with a status-to-code mapping in a match statement. Both emit identical JSON shape. If `GatewayError` is kept alive (P0 resolution), it should replace `ApiError`. If `GatewayError` is deleted, `middleware/error.rs` remains as the sole error system but needs a reference in `error.rs` to be deleted.
- **Previous state**: New finding.
- **Recommendation**: Unify into one error system; delete the other.

### P3: `#[allow(dead_code)]` annotations remain on unused items

- **Severity**: P3 -- Low
- **Location**: `proxy/stream.rs:325` (`detect_sse_error`), `proxy/stream.rs:355` (`sse_error_event`), `proxy/mod.rs:184` (`FailureReason::ModelFallback`)
- **Description**: Three dead-code items with `#[allow(dead_code)]` persist:
  - `detect_sse_error()` (stream.rs:326, 27 lines) -- unused in production, only called from tests
  - `sse_error_event()` (stream.rs:356, 7 lines) -- unused in production, only called from tests
  - `FailureReason::ModelFallback` (proxy/mod.rs:185) -- marked dead, never constructed; its `log_str()` arm exists but is unreachable
- **Previous state**: Identified in V2 as QUAL-2. Not fixed.
- **Recommendation**: Remove these three items and their test cases. `FailureReason::FallbackReason` without the `ModelFallback` variant would change the match in `log_str()` to be non-exhaustive -- remove the variant and add a catch-all arm, or keep the variant if future use is anticipated.

### P3: `GrantParams` -- slight name inconsistency

- **Severity**: P3 -- Low
- **Location**: `proxy/openai.rs:23` (`GatewayParams`)
- **Description**: Minor naming issue: the struct is called `GatewayParams` but belongs inside the proxy `AppState` and holds proxy-level config (timeouts, retries, fallbacks). The name `GatewayParams` is overly broad since `AppConfig` already has `GatewayConfig`. Neither name indicates what is different between `GatewayConfig` (static config) and `GatewayParams` (runtime subset used by the proxy).
- **Previous state**: New finding.
- **Recommendation**: Rename to `ProxyParams` to clarify its role as the proxy/runtime subset of `GatewayConfig`.

### P3: Proxy handler names misleading -- `openai.rs` owns `AppState`

- **Severity**: P3 -- Low
- **Location**: `proxy/openai.rs`
- **Description**: The file `proxy/openai.rs` contains `AppState` (the core gateway state struct used everywhere), `GatewayParams`, `RouterState`, `CacheState`, `LimitsState`, `BillingState`, `McpState`, `SecurityState`, plus the `handle_chat_completions()` function (the primary proxy entry point). The filename suggests it contains only the OpenAI adaptor handler, but it is actually the central hub of the gateway. This is misleading for new contributors.
- **Previous state**: New finding.
- **Recommendation**: Either (a) move `AppState` and its sub-structs to a separate `state.rs` file in the proxy module, or (b) rename `openai.rs` to `handler.rs`. Option (a) is cleaner.

---

## Positive Patterns

Despite the issues above, the codebase has several well-designed abstractions that should be preserved:

### 1. ProviderAdaptor Trait (proxy/provider.rs)
A clean trait-based abstraction for provider-specific differences. Three implementations (OpenAI, Anthropic, Gemini) with sensible defaults. The trait separates URL construction, auth, request/response transformation, and stream format detection into well-defined methods. Adding a new provider requires implementing 3-5 methods.

### 2. HttpPool with RAII Guards (http_pool.rs)
Least-busy client selection across multiple `reqwest::Client` instances. The `PooledClient` guard increments an `AtomicU8` on creation and decrements on `Drop`. The guard can be moved into spawned async tasks so the count stays accurate for streaming responses whose bodies outlive the dispatch function.

### 3. RAII ActiveRequestGuard (proxy/dispatch.rs:25-38)
Simple, correct pattern: a struct that increments `active_requests` on construction and decrements on `Drop`. Ensures the gauge is always balanced regardless of return path or panic.

### 4. BucketedWindow Rate Limiter (proxy/rate_limiter.rs)
O(num_buckets) per operation, independent of request count. No `retain()` scanning. The fixed-size bucket array makes performance predictable.

### 5. CacheMode Enum (proxy/cache.rs)
Clean four-state model: On, Off, ReadOnly, WriteOnly. Each `can_read()`/`can_write()` predicate makes cache behavior explicit. The `from_str()` parser accepts multiple aliases for each mode.

### 6. InFlight Request Coalescing (proxy/cache.rs:248-289)
Uses `tokio::sync::Notify` for correct ordering-independent signaling. When multiple identical requests arrive concurrently, only one is dispatched; others wait and reuse the cached result.

### 7. Thread Safety Discipline
The codebase consistently uses `Arc<RwLock<>>` for shared mutable state, `Arc<Mutex<>>` for synchronous state, and `std::sync::atomic` for counters. No evidence of deadlock-prone multi-lock patterns. The only shared state behind a single Mutex is the rate limiter and cache.

### 8. Copy-on-Write Body Mutations (proxy/attempt.rs:86-104)
The attempt module uses `Cow<'_, Value>` to avoid cloning the request body for every channel attempt. The clone happens only when model mapping or payload rules require mutations. A well-considered optimization.

### 9. Prometheus Metrics Coverage
11 metric types covering: request counts, duration histograms, cache hits/misses/evictions, active requests, circuit breaker state, retries, TTFT, input/output tokens. The `/metrics` endpoint uses the standard Prometheus text format.

### 10. Single-Mutex Rate Limiter
All rate limiter state (per-channel windows + global TPM window) is behind a single Mutex. This avoids the deadlock risk of multiple locks and the complexity of fine-grained locking.

---

## Summary Matrix

| Priority | Count | Key Items |
|----------|-------|-----------|
| P0 | 1 | `GatewayError` dead code (170 lines, zero usage) |
| P1 | 3 | God function in `server.rs`, zero test coverage on server/admin, aggressive `unwrap()` usage |
| P2 | 4 | Gemini translation code duplication, stringly-typed routing strategy, unused drain_timeout, dual error systems |
| P3 | 3 | Dead `#[allow(dead_code)]` items, misleading naming, proxy state location |

Total: 8 new issues (beyond those already fixed from V2).
