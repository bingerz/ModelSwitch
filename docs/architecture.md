# Architecture

This document describes the request flow, key design decisions, and module dependency graph for ModelSwitch.

## Request Flow

```
Client Request
  │
  ▼
┌─────────────────────────────────────────────┐
│  Axum Router (build_router in lib.rs)       │
│  Routes:                                    │
│    /v1/chat/completions  (OpenAI)           │
│    /v1/messages          (Anthropic)        │
│    /v1beta/models/{path} (Gemini)           │
│    /api/provider/{p}/v1/... (Claude Code)   │
│    /mcp                  (MCP Gateway)      │
│    /api/*                (Admin REST)       │
└─────────────┬───────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────┐
│  Middleware Stack (outer → inner)           │
│  1. request_id  – assigns X-Request-ID      │
│  2. sanitizer   – redacts secrets from body │
│  3. virtual_key – validates ms-vk- keys     │
│     (pass-through when no keys configured)  │
│  4. admin_auth  – Bearer token for /api/*   │
└─────────────┬───────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────┐
│  Provider Handler                          │
│  (openai.rs / anthropic.rs / gemini.rs)     │
│                                             │
│  Validates request body (model, messages).  │
│  If MCP auto-inject is enabled: aggregates  │
│  tools from running MCP servers, appends    │
│  them to body.tools, then enters the MCP    │
│  tool-call loop after dispatch.             │
└─────────────┬───────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────┐
│  dispatch()  (proxy/dispatch.rs)            │
│                                             │
│  1. Extract request metadata (model,        │
│     stream flag, session ID, affinity).     │
│  2. Check request cache + in-flight         │
│     coalescing (non-stream only).           │
│  3. Resolve fallback chain:                 │
│     [original, fallback1, fallback2, ...]   │
│  4. For each model in the chain:            │
│       select_channel_for_attempt()          │
│         → session affinity first            │
│         → router::select_channel() fallback │
│       try_channel_attempt()                 │
│         → rate limit check                  │
│         → payload rules (override/strip)    │
│         → HTTP dispatch to upstream         │
│         → AttemptOutcome::Respond | Retry   │
│  5. All exhausted → 429 response + log.     │
└─────────────┬───────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────┐
│  attempt.rs – Single Channel Attempt        │
│                                             │
│  Builds upstream URL, injects auth headers  │
│  (OpenAI Bearer / Anthropic x-api-key /     │
│  Gemini URL key / Cookie), forwards body.   │
│                                             │
│  Response handling:                         │
│    Stream (SSE): keepalive stream +         │
│      background telemetry extraction        │
│    JSON: parse usage, calculate cost,       │
│      cache response                         │
│                                             │
│  On success: log + bill + update affinity   │
│  On failure: categorize (FailureReason),    │
│    record circuit-breaker window failure,   │
│    return Retry                             │
└─────────────┬───────────────────────────────┘
              │
              ▼
┌─────────────────────────────────────────────┐
│  Billing & Telemetry (background)           │
│                                             │
│  • QuotaStore::accumulate_usage()           │
│    – adds input/output/cache tokens         │
│  • VirtualKeyStore::accumulate_spend()      │
│    – adds estimated cost in cents           │
│  • DispatchLogger::log()                    │
│    – writes NDJSON dispatch log entry       │
│  • Channel::recover_to_healthy()            │
│    – promotes HalfOpen → Healthy            │
└─────────────────────────────────────────────┘
```

### MCP Tool Auto-Injection Loop

When `mcp_auto_inject = true` and MCP servers are running, the OpenAI handler enters an iterative loop:

```
1. Aggregate tools from all running MCP servers
   (mcp_tools::aggregate_tools → McpManager::list_all_tools)
2. Append tools to body.tools as mcp__{server}__{tool}
3. Dispatch request upstream (non-streaming internally)
4. If response contains tool_calls targeting mcp__ tools:
   a. Execute each tool call via McpManager::call_tool
   b. Append tool results to body.messages
   c. Re-dispatch (goto step 3)
5. Return final response (text or remaining non-MCP tool_calls)
   Loop bounded by mcp_max_iterations (default: 5).
```

## Key Design Decisions

### 1. Shared `dispatch()` Pattern

All three provider handlers (OpenAI, Anthropic, Gemini) converge on a single `dispatch()` function in `proxy/dispatch.rs`. Each handler constructs a `ProxyConfig` struct (default model, upstream path, auth style) and passes it to `dispatch()` along with the shared `AppState`. This eliminates duplicated retry, caching, and fallback logic across providers.

```rust
pub(crate) struct ProxyConfig {
    pub default_model: &'static str,
    pub upstream_path: &'static str,
    pub auth_style: AuthStyle,  // OpenAI | Anthropic | Cookie | GeminiUrl
}
```

### 2. `RoutingStrategy` Trait

Channel selection is abstracted behind a trait, making routing strategies extensible without modifying the dispatch loop:

```rust
pub trait RoutingStrategy: Send + Sync {
    fn select(&self, candidates: &[Channel]) -> Option<Channel>;
}
```

Three implementations ship built-in:
- **`WeightedRandomStrategy`** - Default; random selection weighted by channel `weight` field
- **`LatencyBasedStrategy`** - Picks from the top-K channels by lowest `avg_latency_ms`; falls back to weighted random when no latency data exists
- **`LeastBusyStrategy`** - Selects the channel with the fewest in-flight requests (tracked by `ActiveRequests`); falls back to weighted random on ties

### 3. `AppState` Nested Substructs

The shared application state is decomposed into domain-specific substructs to keep concerns isolated and field access readable:

```rust
pub struct AppState {
    pub channel_mgr: Arc<ChannelManager>,
    pub credential_store: SharedCredentialStore,
    pub logger: Arc<DispatchLogger>,
    pub http_client: reqwest::Client,
    pub gateway: GatewayParams,      // timeouts, retries, fallbacks, strategy
    pub router: RouterState,         // affinity, active requests
    pub cache: CacheState,           // request cache, in-flight coalescing
    pub limits: LimitsState,         // rate limiter, payload rules
    pub billing: BillingState,       // quota store, virtual key store
    pub mcp: McpState,               // MCP manager, iteration config
    pub security: SecurityState,     // admin token, sanitizer config
    pub started_at: std::time::Instant,
}
```

Handlers access state via `state.cache.request_cache`, `state.billing.quota_store`, etc., making the domain boundary explicit at every call site.

### 4. `PersistedStore<K, V>` for JSON-Persisted Stores

Stores that need to survive restarts (quota data, virtual keys) build on a generic `PersistedStore<K, V>` that wraps `RwLock<HashMap<K, V>>` with async and sync persistence methods:

```rust
pub struct PersistedStore<K, V>
where
    K: Hash + Eq + Clone + Serialize + DeserializeOwned,
    V: Clone + Serialize + DeserializeOwned,
{
    data: RwLock<HashMap<K, V>>,
    store_path: PathBuf,
}
```

- `persist()` / `load()` - async save/load to JSON file with 0600 permissions on Unix
- `persist_sync()` - synchronous save for shutdown paths where no async runtime is available
- `load()` merges into existing entries (does not overwrite), so the poller refreshes data without losing accumulated counters

### 5. `GatewayError` Unified Error Type

All domain errors funnel through a single `thiserror`-derived enum with context-preserving `From` impls for `anyhow::Error`, `serde_json::Error`, and `QuotaError`:

```rust
#[derive(Debug, Error)]
pub enum GatewayError {
    ChannelNotFound(String),
    NoHealthyChannel(String),
    AllChannelsExhausted(String),
    Credential(String),
    RateLimited,
    Upstream { status: u16, body: String },
    Connection(String),
    Timeout(u64),
    Quota(#[from] QuotaError),
    VirtualKey(String),
    // ...
}
```

### 6. `FailureReason` Typed Enum

Instead of stringly-typed failure categories, dispatch diagnostics use a typed enum:

```rust
pub(super) enum FailureReason {
    RateLimited,      // 429 from upstream
    ServerError,      // 5xx from upstream
    ConnectionError,  // network/timeout
    NoCredential,     // missing or expired credential
    ModelFallback,    // switch to next model in fallback chain
    AllExhausted,     // no channels left to try
    ClientError(u16), // 4xx (non-429) — not retried
}
```

Each variant maps to a `log_str()` for NDJSON logging and drives retry/circuit-breaker decisions in `attempt.rs`.

### 7. Circuit Breaker State Machine

Each channel has a `ChannelStatus` that transitions through a state machine:

```
                  failures >= 5 in 5-min window
    Healthy ──────────────────────────────────► CircuitOpen
        ▲                                            │
        │ success on probe                           │ cooldown expires
        │                                            ▼
      HalfOpen ◄─────────────────────────────────────┘
        │
        │ failure on probe
        ▼
    CircuitOpen (cooldown reset)
```

Key properties:
- **`Healthy`** - Normal operation; eligible for all routing strategies
- **`CircuitOpen`** - Removed from candidate pool; `circuit_open_until` timestamp set to `now + cooldown_minutes`
- **`HalfOpen`** - Transitioned from CircuitOpen after cooldown expires; eligible for limited probing (router prefers Healthy over HalfOpen to limit probe traffic)
- **Success in HalfOpen** - `recover_to_healthy()` resets failure counters and sliding window
- **Failure in HalfOpen** - Back to CircuitOpen with a fresh cooldown

The sliding window (`record_window_failure`) tracks failures in a 5-minute window. Five failures within the window trigger the circuit open transition.

### 8. MCP Tool Auto-Injection Loop

The gateway can act as an MCP-aware agent proxy. When enabled, the OpenAI chat completions handler transparently:

1. **Aggregates** tools from all running MCP server subprocesses (via `rmcp` client connections)
2. **Injects** them into the request `tools` array under namespaced IDs (`mcp__{server}__{tool}`)
3. **Dispatches** the request upstream
4. **Intercepts** tool_calls targeting MCP tools, executes them via `McpManager::call_tool`, and appends results to the conversation
5. **Re-dispatches** with the expanded conversation, looping up to `mcp_max_iterations` times

This lets any OpenAI-compatible client use MCP tools without MCP protocol awareness. The gateway also exposes a `/mcp` endpoint (Streamable HTTP transport) for direct MCP client connections.

## Module Dependency Graph

```
                          ┌──────────┐
                          │  lib.rs  │  (entry point, router assembly, lifecycle)
                          └────┬─────┘
           ┌───────────────────┼───────────────────────────────────┐
           │                   │                                   │
    ┌──────▼──────┐    ┌──────▼──────┐                     ┌──────▼──────┐
    │  config/    │    │ middleware/ │                     │   admin.rs  │
    │  (TOML +    │    │ (auth,      │                     │ (REST API)  │
    │   watcher)  │    │  sanitizer, │                     └──────┬──────┘
    └──────┬──────┘    │  vk, req_id)│                            │
           │           └──────┬──────┘                            │
           │                  │                                   │
    ┌──────▼──────────────────▼───────────────────────────────────▼──────┐
    │                         proxy/                                      │
    │  ┌───────────┐ ┌──────────┐ ┌─────────┐ ┌─────────┐ ┌───────────┐  │
    │  │ dispatch  │ │ attempt  │ │ openai  │ │anthropic│ │  gemini   │  │
    │  │ (retry,   │ │ (single  │ │(handler │ │(handler │ │ (handler  │  │
    │  │  fallback)│ │ attempt) │ │+AppState)│ │ +vk)    │ │  +trans)  │  │
    │  └─────┬─────┘ └────┬─────┘ └────┬────┘ └────┬────┘ └─────┬─────┘  │
    │        │            │            │           │             │        │
    │  ┌─────▼────┐ ┌────▼─────┐ ┌────▼────┐ ┌────▼────┐ ┌──────▼─────┐  │
    │  │  cache   │ │  stream  │ │rate_lim │ │ payload │ │ mcp_tools  │  │
    │  │(coalesce)│ │ (SSE)    │ │(RPM/TPM)│ │ _rules  │ │(aggregate) │  │
    │  └──────────┘ └──────────┘ └─────────┘ └─────────┘ └──────┬─────┘  │
    │  ┌──────────┐ ┌──────────┐                                   │        │
    │  │ usage    │ │translate │                                   │        │
    │  │(tokens)  │ │(gemini)  │                                   │        │
    │  └──────────┘ └──────────┘                                   │        │
    └───────────────────────────┬──────────────────────────────────┼────────┘
                                │                                  │
              ┌─────────────────┼──────────────┐                   │
              │                 │              │                   │
       ┌──────▼──────┐  ┌──────▼──────┐ ┌─────▼─────┐    ┌────────▼────────┐
       │  channel/   │  │  router/    │ │ health/   │    │      mcp/       │
       │ (circuit    │  │ (weighted,  │ │ (checker) │    │ manager         │
       │  breaker,   │  │  latency,   │ └───────────┘    │ aggregator     │
       │  status)    │  │  least_busy,│                  │ translator     │
       └──────┬──────┘  │  affinity)  │                  │ gateway (/mcp) │
              │         └──────┬──────┘                  └────────┬────────┘
              │                │                                  │
       ┌──────▼──────┐  ┌──────▼──────┐                   ┌───────▼────────┐
       │ credential/ │  │ active_req  │                   │ config/        │
       │ (keyring/   │  │ (in-flight  │                   │ (McpServerCfg) │
       │  file)      │  │  counters)  │                   └────────────────┘
       └─────────────┘  └─────────────┘
              │
       ┌──────▼──────────────────────────────────────┐
       │  Domain Stores (all use PersistedStore)      │
       │  ┌─────────────┐  ┌──────────────────────┐   │
       │  │  quota/     │  │  virtual_key/        │   │
       │  │ (poller,    │  │ (budget caps,        │   │
       │  │  registry,  │  │  spend tracking,     │   │
       │  │  store)     │  │  constant-time auth) │   │
       │  └─────────────┘  └──────────────────────┘   │
       └──────────────────────────────────────────────┘
              │
       ┌──────▼──────┐  ┌──────────┐  ┌─────────────────┐
       │ error.rs    │  │ log.rs   │  │ persisted_store │
       │(GatewayErr) │  │(NDJSON)  │  │ (generic K-V)   │
       └─────────────┘  └──────────┘  └─────────────────┘
```

### Dependency Summary

| Module | Depends On |
|--------|-----------|
| `lib.rs` | `config`, `proxy`, `admin`, `middleware`, `mcp`, `channel`, `credential`, `quota`, `virtual_key`, `router`, `health`, `log` |
| `proxy/` | `channel`, `router`, `config`, `mcp`, `quota`, `virtual_key`, `credential`, `log`, `error` |
| `proxy/dispatch` | `proxy/attempt`, `proxy/cache`, `router`, `channel`, `log` |
| `proxy/attempt` | `proxy/stream`, `proxy/usage`, `proxy/cache`, `channel`, `quota`, `virtual_key`, `log` |
| `router/` | `channel` |
| `channel/` | `config`, `credential` |
| `mcp/` | `config` (McpServerConfig) |
| `quota/` | `config`, `persisted_store`, `channel` |
| `virtual_key/` | `persisted_store` |
| `middleware/` | `virtual_key`, `config`, `error` |
| `admin/` | `proxy` (AppState), `channel`, `mcp`, `quota`, `virtual_key`, `config` |
| `persisted_store` | (standalone) |
| `error.rs` | `quota` (QuotaError via `From`) |
