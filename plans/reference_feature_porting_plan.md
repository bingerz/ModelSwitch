# Reference Projects Evaluation & Porting Plan

> **Revision 2 — 2026-06-14**: Incorporates deep-dive findings from rmcp library evaluation, codebase integration analysis, and competitive library assessment. Estimates and architecture significantly revised from original.

This document analyzes three reference projects—CLIProxyAPI, higress, and LiteLLM—evaluates their features, and outlines a prioritized porting plan to enhance ModelSwitch.

---

## 1. Current Project Status & Reference Feature Analysis

### 1.1 Already Implemented ✅

| Feature | Implementation Status | Location |
| :--- | :--- | :--- |
| **OAuth WebView Login** | ✅ Full | `src-tauri/src/webview_login.rs` |
| **Cookie/Session Extraction** | ✅ Full | `src-tauri/src/admin.rs` (`/api/auth/cookies`) |
| **Model Fallback Chains** | ✅ Full | `src-tauri/src/router/fallback.rs`, `config.rs` |
| **Circuit Breaker** | ✅ Basic | ChannelManager, `src-tauri/src/router/circuit.rs` |
| **Rate Limiting** | ✅ Full | `src-tauri/src/proxy/rate_limiter.rs` |
| **Request Caching** | ✅ Full | `src-tauri/src/proxy/cache.rs` |
| **Quota Polling & Tracking** | ✅ Full | `src-tauri/src/quota/` |
| **Health Checks** | ✅ Full | `src-tauri/src/health/` |
| **Session Affinity** | ✅ Full | `src-tauri/src/router/affinity.rs` |
| **Multiple Routing Strategies** | ✅ Full | `src-tauri/src/router/strategy.rs` |
| **Cost Tracking** | ✅ Full | `src-tauri/src/quota/mod.rs` (`accumulate_usage`) |
| **Smart Model Mapping** | ✅ Full | `config.rs` (`model_mapping` per channel) |
| **Retries & Cooldowns** | ✅ Full | `config.rs` (`max_retries`, `cooldown_minutes`) |
| **Protocol Translation** | ✅ OpenAI/Anthropic/Gemini | `src-tauri/src/proxy/` |

### 1.2 Reference Projects — Remaining High-Value Features

#### CLIProxyAPI (Go-based Local CLI Proxy)
* **Target**: Local developers using AI coding tools (Claude Code, ChatGPT/Codex, Gemini CLI).
* **Remaining value**:
  1. **Multi-Account Pool Management**: Account pool with quota restoration tracking.
  2. **Coding Assistant Protocol Routing**: `/api/provider/{provider}/v1/...` for Claude Code/Amp CLI.

#### higress (Go/Envoy Cloud-Native AI Gateway)
* **Target**: Enterprise API gatekeeping.
* **Remaining value**:
  1. **MCP Server Hosting**: Unified management of MCP tools.
  2. **Advanced Sentinel Traffic Control**: Sophisticated circuit breakers and concurrency throttling.

#### LiteLLM (Python-based Universal LLM Gateway)
* **Target**: Startups and enterprise teams integrating multiple model providers.
* **Remaining value**:
  1. **Virtual Keys & Budget Tracking**: Spend tracking with hard/soft budget limits per virtual key.
  2. **Guardrails & Privacy Masking**: PII masking, safety filters, and response validation.

---

## 2. Library Selection: `rmcp` as MCP Runtime

Based on a comparative evaluation of 7 Rust MCP libraries (2026-06-14), `rmcp` is the sole recommended choice for ModelSwitch's MCP Server Manager.

### 2.1 Decision Rationale

| Factor | `rmcp` | Closest Alternative (`rust-mcp-sdk`) |
| :--- | :--- | :--- |
| **Governance** | `modelcontextprotocol` org (official) | Community org |
| **Subprocess support** | `TokioChildProcess` with `process-wrap` 9.0 | `StdioTransport::create_with_server_launch()` |
| **Client + Server roles** | Both, separate feature flags | Both |
| **Protocol coverage** | Tools, Resources, Prompts, Tasks, Elicitation, OAuth | Similar breadth |
| **Production readiness** | Reference implementation | README: *"use at your own risk"* |
| **Downloads** | 12.5M+ (v1.7.0) | Moderate |

### 2.2 Libraries Rejected

| Library | Reason |
| :--- | :--- |
| `rust-mcp-sdk` | Maintainer's own "use at your own risk" disclaimer; 44 open issues |
| `rust-mcp-schema` | Types only — no transport, no runtime |
| `smg-mcp` | Product component of competing SMG gateway; not reusable |
| `mcpkit` | 2.3K downloads; self-described "official" is misleading; immature |
| `agenterra` | Codegen tool, not runtime; viable only as build-time OpenAPI→MCP scaffolder |
| `rmcp-actix-web` | Targets actix-web, not axum; reference material only |

### 2.3 `rmcp` Feature Flags for ModelSwitch

```toml
[dependencies]
rmcp = { version = "=1.7", default-features = false, features = [
    "client",                   # Connect to MCP server subprocesses
    "macros",                   # #[tool_router], #[tool_handler]
    "transport-child-process",  # TokioChildProcess (stdio subprocess)
    "transport-io",             # stdin/stdout transport
    # Do NOT enable "transport-streamable-http-server" — requires axum 0.8
    # (ModelSwitch is on axum 0.7; see §6.1 Phase 3.5 for upgrade plan)
] }
```

### 2.4 What `rmcp` Eliminates

The original plan called for building `mcp/protocol.rs` to implement JSON-RPC parsing/serialization. **This is no longer needed** — `rmcp` provides:
- Full JSON-RPC 2.0 protocol handling
- MCP handshake (`initialize`, `initialized`)
- `tools/list`, `tools/call` operations
- `resources/*`, `prompts/*` operations
- Cancellation, progress notifications, pagination
- `schemars 1.0` JSON Schema 2020-12 for tool parameter types

---

## 3. Porting Value & Feasibility Matrix (Revised)

| Feature | Source | Value | Feasibility | Effort (revised) | Priority |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Local Privacy Guardrails** | LiteLLM | Medium-High | High — fits existing `middleware/` layer | 3-4 days | **P1** |
| **Virtual Keys & Budgets** | LiteLLM | High | High — extend `QuotaStore`, new `middleware/virtual_key.rs` | 5-7 days | **P1** |
| **MCP Server Manager** | higress | High — product-defining | Medium — 3 blockers to resolve first | **17-20 days** | **P0** |
| **Multi-Account Pool** | CLIProxyAPI | Medium | Medium | 2-3 days | P2 |
| **Claude Code Protocol** | CLIProxyAPI | Medium | Medium | 1-2 days | P2 |
| **Advanced Circuit Breaker** | higress | Low | Medium | 2-3 days | P2 |

> **Recommended execution order**: Privacy Guardrails → Virtual Keys → MCP Server Manager.
> Start with lowest-risk, highest-immediate-value work; tackle MCP complexity last.

---

## 4. Recommended Porting Plan

### Phase 1: Local Privacy Guardrails (P1)

Secure developer codebases from accidental secrets leakage to upstream LLMs.

#### Scope
* Create `middleware/sanitizer.rs` with predefined regex patterns for common secrets:
  * AWS keys (`AKIA[0-9A-Z]{16}`)
  * Stripe keys (`sk_live_[0-9a-zA-Z]{24}`)
  * SSH private keys (`-----BEGIN RSA PRIVATE KEY-----`)
  * Database passwords in connection URLs
  * Generic high-entropy strings (entropy-based detection as supplement)
* Configurable rules in `config.rs`:
  ```toml
  [gateway.sanitizer]
  enabled = true
  redact_secrets = true
  custom_patterns = [
    { pattern = "my-secret-[0-9]+", replacement = "[REDACTED_CUSTOM]" }
  ]
  ```
* Add as tower middleware in `lib.rs` middleware stack (after `request_id`, before auth).

#### Streaming Consideration
LLM responses are streamed as SSE chunks. A secret can span two chunk boundaries. The sanitizer must:
* **Request side**: scan the full body before forwarding (safe — body is complete).
* **Response side**: buffer SSE chunks with a sliding window, scan concatenated content, and flush after the window passes the pattern boundary. Use a small buffer (256 bytes overlap) to handle cross-chunk secrets.

#### Integration Points
* Mount in existing `middleware/` directory alongside `auth.rs`, `error.rs`, `request_id.rs`.
* Add to the global middleware stack in `build_router()` (`lib.rs:487-494`).
* Config section added to `GatewayConfig` in `config.rs` (follows existing `#[serde(default)]` pattern).

#### Estimate: 3-4 days
0.5 day config schema + sanitizer core · 1 day streaming-aware response scan · 0.5 day frontend toggle · 1-2 days tests + edge cases.

---

### Phase 2: Virtual Keys & Budget Control (P1)

Prevent runaway token costs when local agentic IDEs loop, and enable multi-agent budget isolation.

#### Scope
* **VirtualKey struct** in new file `middleware/virtual_key.rs` (NOT in `proxy/mod.rs` which is already 1008 lines):
  * Fields: `key_hash`, `name`, `daily_budget_cents`, `monthly_budget_cents`, `current_spend_cents`, `enabled`, `created_at`.
  * Persistence: `~/.config/modelswitch/virtual_keys.json`.
  * Key generation: cryptographically random 32-byte key, stored as SHA-256 hash (plaintext shown once at creation).
  * Key comparison: use existing `subtle` crate (constant-time) — already in `Cargo.toml`.
* **VirtualKeyStore** — mirrors `QuotaStore` pattern in `quota/`.
* **Middleware** in `middleware/virtual_key.rs`:
  * Intercept `Authorization: Bierer <virtual_key>` header.
  * Reject with 402 if budget exhausted (soft) or 403 if disabled (hard).
  * After upstream response, accumulate spend via existing `QuotaStore::accumulate_usage()` pattern — extend with `virtual_key_id` parameter.
* **Admin API** in `admin.rs`:
  * `POST /api/virtual-keys` — create key (returns plaintext once).
  * `GET /api/virtual-keys` — list keys with current spend.
  * `DELETE /api/virtual-keys/:id` — revoke.
  * `GET /api/virtual-keys/:id/usage` — detailed usage history.
* **Frontend**: "Virtual Keys" tab — create/revoke keys, view budget consumption.

#### Threat Model Note
ModelSwitch is a localhost proxy. Virtual Keys solve "multiple agents sharing one gateway instance need budget isolation" — NOT multi-user auth. Keys protect against runaway spend, not against malicious local actors (any local process can read the config file).

#### Integration Points
* New module: `middleware/virtual_key.rs` (following `auth.rs` pattern).
* New store: `quota/virtual_key_store.rs` (following `QuotaStore` pattern).
* `AppState` (`proxy/openai.rs:19-38`): add `virtual_key_store: Arc<VirtualKeyStore>`.
* Middleware stack in `build_router()`: add after `admin_auth_middleware` on admin routes, and as a standalone layer on proxy routes.
* Admin routes in `lib.rs:463-481`.

#### Estimate: 5-7 days
1 day core structs + store · 1.5 days middleware (validation + spend tracking) · 1 day admin API · 1.5 days frontend · 1-2 days tests.

---

### Phase 3: MCP Server Manager (P0)

Transform ModelSwitch from an LLM Gateway to a unified AI tool gateway using `rmcp`.

> **This is the most complex phase.** Three blockers must be resolved before feature work begins. See §5 Risk Matrix for details.

#### Phase 3.0: Research Spike (1-2 days)
* [ ] Verify `rmcp` v1.7.0 API surface against latest docs.rs (codebase analysis was based on v0.14.0).
* [ ] Write 30-line PoC: `TokioChildProcess` + `list_all_tools()` — confirm it compiles with axum 0.7.
* [ ] Verify `process-wrap 9.0` behavior on macOS + Windows.
* [ ] Pin `rmcp = "=1.7.0"` in `Cargo.toml`.

#### Phase 3.1: Infrastructure (5-6 days)

**Prerequisite: refactor `dispatch()` (2-3 days)**

`proxy/mod.rs:315-937` contains a monolithic 622-line `dispatch()` function. MCP tool injection must add branching logic inside it. Before any MCP work, decompose into:

```
dispatch()
├── cache_lookup()           // lines 348-364
├── channel_loop()           // lines 417-559 (selection + retry)
├── prepare_upstream()       // lines 444-559 (payload rules + auth injection)
├── handle_streaming()       // lines 720-848
└── handle_json()            // lines 850-908
```

This is pure code movement — no logic changes. Run full regression after.

**MCP core (3 days)**

* `McpServerConfig` struct in `config.rs`:
  ```toml
  [[mcp_servers]]
  id = "filesystem"
  name = "Local Filesystem"
  command = "npx"
  args = ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
  env = { "NODE_ENV" = "production" }
  cwd = "/Users/hanson/workspace"
  enabled = true
  expose_tools = true
  ```
  Added to `AppConfig` at line 31 as `#[serde(default)] pub mcp_servers: Vec<McpServerConfig>`.

* `mcp/manager.rs` — `McpManager` struct:
  * Mirrors `ChannelManager` pattern (`channel/manager.rs`).
  * Holds `HashMap<String, McpServerEntry>` where each entry has config + `Option<RunningService<RoleClient, ClientHandler>>`.
  * `start_server()`: build `tokio::process::Command`, wrap in `TokioChildProcess`, call `().serve(transport)`.
  * `stop_server()`: call `client.cancel()`, then `TokioChildProcess::graceful_shutdown()`.
  * `stop_all()`: iterate all entries; called from gateway shutdown path.

* `AppState` (`proxy/openai.rs:19`): add `pub mcp_manager: Arc<McpManager>`.
* Gateway lifecycle (`lib.rs`): initialize in `start_gateway_services()`, shutdown in drain path.
* Config watcher (`config/watcher.rs`): add MCP server hot-reload parallel to channel reload.

#### Phase 3.2: Tool Exposure (4-5 days)

* `mcp/aggregator.rs` — multi-server tool aggregation:
  * Iterate all running MCP servers, call `client.list_all_tools()`.
  * Namespace each tool: `mcp__{server_id}__{original_name}`.
  * Return `Vec<AggregatedTool>` with server_id + namespaced name + original `Tool`.

* `mcp/translator.rs` — format conversion:
  * `to_openai_function(tool: &McpTool) -> Value` — wrap in `{"type":"function","function":{...}}`.
  * `to_anthropic_tool(tool: &McpTool) -> Value` — `{"name","description","input_schema"}`.
  * `sanitize_schema(schema: &Value) -> Value` — inline `$ref`, strip `$schema`/`title`, ensure `type: "object"` at root. Handles schemars output that some LLM providers reject.
  * `from_openai_tool_call(name: &str, args: &Value) -> CallToolRequestParam` — strip `mcp__` prefix, extract original tool name.
  * `flatten_result(result: &CallToolResult) -> String` — concatenate text content blocks, drop/encode non-text.

* Admin API endpoints in `admin.rs`:
  * `GET /api/mcp/servers` — list configured servers with status.
  * `POST /api/mcp/servers` — add server config.
  * `PUT /api/mcp/servers/:id` — update config.
  * `DELETE /api/mcp/servers/:id` — remove + stop.
  * `POST /api/mcp/servers/:id/start` — spawn subprocess.
  * `POST /api/mcp/servers/:id/stop` — terminate.
  * `GET /api/mcp/servers/:id/tools` — list tools from this server.
  * `GET /api/mcp/tools` — aggregated tools across all servers.

* Proxy endpoint: `GET /v1/tools` in `proxy/` — expose aggregated tools in OpenAI format for client discovery.

#### Phase 3.3: Tool Call Closed Loop (4-5 days)

> **Critical**: The proxy currently has zero tool-call handling. The `tools`, `tool_calls`, and `function_call` fields pass through as opaque JSON. This phase builds the interception layer from scratch.

* `proxy/mcp_tools.rs` — request/response interception:

  **Request side** (inject into `dispatch()`, after payload rules ~line 447):
  1. Check if `body.tools` array exists.
  2. Append MCP tool definitions (from aggregator, translated to the request's protocol format).
  3. Forward modified body to upstream.

  **Response side** (intercept after `handle_json()` ~line 850, and in streaming telemetry):
  1. Parse `choices[0].message.tool_calls` (OpenAI) or `content[].type == "tool_use"` (Anthropic).
  2. For any tool call with `mcp__` prefix:
     - Route to the correct MCP server via `McpManager`.
     - Execute `client.call_tool(CallToolRequestParam)`.
     - Flatten result to string.
  3. Append tool result as a new `tool` role message.
  4. Re-submit the conversation to the upstream LLM with tool results included.
  5. Return final response to client.

  **Streaming side** (in `proxy/stream.rs`):
  - SSE chunks may split a tool call across boundaries.
  - Buffer tool_call arguments until the `finish_reason: "tool_calls"` chunk arrives, then process the complete call.

#### Phase 3.4: Frontend + Tauri Commands (2-3 days)

* Tauri commands (5 new, in `lib.rs:603-612`):
  * `mcp_list_servers` — `tauri::State<GatewayManager>` → `Vec<McpServerStatus>`.
  * `mcp_spawn_server` — `(manager, server_id: String)` → `Result<(), String>`.
  * `mcp_kill_server` — `(manager, server_id: String)` → `Result<(), String>`.
  * `mcp_list_tools` — `(manager, server_id: String)` → `Vec<McpTool>`.
  * `mcp_call_tool` — `(manager, server_id, tool_name, args)` → `Value`.

* Frontend `McpServersPanel.tsx` (`src/components/`):
  * Server list with status indicators (running/stopped/error).
  * Add/edit/delete server config form.
  * Start/stop buttons.
  * Tool browser — list tools per server, test-call UI.
  * Add `"mcp"` to `TabId` union type in `App.tsx:12`.

#### Phase 3.5: MCP Gateway Mode — Optional (3-4 days, deferred)

Expose ModelSwitch itself as an MCP server over HTTP, allowing Claude Desktop / Cursor / other MCP clients to connect directly.

* **Prerequisite**: Upgrade axum 0.7 → 0.8 (2-3 days):
  * Route path syntax: `:id` → `{id}` in all `.route()` calls.
  * Middleware composition API changes.
  * Test all existing endpoints after upgrade.
* Enable `transport-streamable-http-server` feature in `rmcp`.
* Implement `ServerHandler` for a `McpGatewayHandler` that aggregates tools from all managed servers and routes `tools/call` requests.

#### Phase 3 Total Estimate: 17-20 days (without 3.5)

| Sub-phase | Days | Cumulative |
| :--- | :--- | :--- |
| 3.0 Research spike | 1-2 | 2 |
| 3.1 Infrastructure (incl. dispatch refactor) | 5-6 | 8 |
| 3.2 Tool exposure | 4-5 | 12 |
| 3.3 Tool call closed loop | 4-5 | 17 |
| 3.4 Frontend + Tauri | 2-3 | **20** |
| 3.5 Gateway mode (optional) | 3-4 | 23 |

---

### Phase 4: Enhancements & Polish (P2)

* **Multi-Account Pool Enhancements** (2-3 days):
  * Account pool rotation when quota exhausted.
  * Track quota restoration times, auto re-enable.
  * "Account sets" for grouping related accounts.

* **Claude Code Protocol Compatibility** (1-2 days):
  * Route handlers for `/api/provider/{provider}/v1/...`.
  * Map Claude Code-specific parameters to standard format.

* **Advanced Circuit Breaker** (2-3 days, optional):
  * Half-open state with periodic probing.
  * Sliding window failure counting.

---

## 5. Risk Matrix

### Phase 3 (MCP) Risks

| Risk | Level | Mitigation |
| :--- | :--- | :--- |
| **axum 0.7/0.8 conflict** | 🔴 Critical | Phase 3.1-3.4 uses stdio only (no axum 0.8 dep). HTTP gateway deferred to 3.5. |
| **`dispatch()` refactor regression** | 🔴 Critical | Pure code movement, no logic changes. Full regression suite before/after. |
| **Zero existing tool-call handling** | 🔴 Critical | Phase 3.3 builds the entire interception layer from scratch. Budget 4-5 days, not 1-2. |
| **Subprocess zombies on Tauri force-quit** | 🟡 High | Hook `RunEvent::ExitRequested` → `mcp_manager.stop_all()`. `process-wrap` provides `ChildWithCleanup` Drop impl. |
| **JSON Schema edge cases** | 🟡 High | `sanitize_schema()` inlines `$ref`, strips `$schema`/`title`. Test with real MCP servers. |
| **rmcp API drift (v0.14 → v1.7)** | 🟡 High | Phase 3.0 validates against current docs.rs. Pin `=1.7.0`. |
| **Cross-platform `npx`/`node` PATH** | 🟡 High | Document prerequisites. Test in Tauri bundle on macOS + Windows. |
| **Tool name collisions** | 🟢 Medium | Namespace: `mcp__{server_id}__{tool_name}`. |
| **3-second force-kill interrupts in-flight calls** | 🟢 Medium | Track in-flight calls; wait before shutdown. |
| **No WebSocket transport in rmcp** | 🟢 Medium | Document limitation. Implement custom `Transport` if needed. |

### Phase 1-2 Risks

| Risk | Level | Mitigation |
| :--- | :--- | :--- |
| **Streaming sanitizer misses cross-chunk secrets** | 🟡 High | Sliding window buffer (256-byte overlap) in response scan. |
| **Virtual key stored in plaintext JSON** | 🟡 Medium | Store SHA-256 hash only; plaintext shown once at creation. `subtle` for constant-time comparison. |

---

## 6. Implementation Roadmap

| Phase | Feature | Effort | Dependencies | Recommended Order |
| :--- | :--- | :--- | :--- | :--- |
| 1 | Privacy Guardrails | 3-4 days | None | **First** (lowest risk, immediate value) |
| 2 | Virtual Keys & Budgets | 5-7 days | None | **Second** |
| 3.0 | MCP Research Spike | 1-2 days | None | Third |
| 3.1 | MCP Infrastructure + dispatch refactor | 5-6 days | 3.0 | |
| 3.2 | MCP Tool Exposure | 4-5 days | 3.1 | |
| 3.3 | MCP Tool Call Closed Loop | 4-5 days | 3.1, 3.2 | |
| 3.4 | MCP Frontend + Tauri | 2-3 days | 3.2 | |
| 3.5 | MCP Gateway Mode (optional) | 3-4 days | 3.1-3.4, axum upgrade | Deferred |
| 4 | Multi-Account Pool | 2-3 days | Phase 2 | After Phase 3 |
| 4 | Claude Code Protocol | 1-2 days | None | Anytime |

**Total Phase 1-3**: ~28-35 working days (6-7 weeks).

---

## 7. Technical Notes

### 7.1 Files Modified vs Created (Phase 3)

**Modified (8 files)**:

| File | Change |
| :--- | :--- |
| `src-tauri/Cargo.toml` | Add `rmcp` dependency with selected features |
| `src-tauri/src/config.rs` | Add `McpServerConfig` struct + `mcp_servers` field on `AppConfig` |
| `src-tauri/src/lib.rs` | MCP routes in `build_router()`, Tauri commands in `invoke_handler`, MCP init in `start_gateway_services()` |
| `src-tauri/src/proxy/openai.rs` | Add `mcp_manager: Arc<McpManager>` to `AppState` |
| `src-tauri/src/proxy/mod.rs` | Refactor `dispatch()` into sub-functions; add MCP tool injection point |
| `src-tauri/src/admin.rs` | Add MCP server CRUD handler functions |
| `src/App.tsx` | Add `"mcp"` tab |
| `src/lib/api.ts` | Add MCP API methods + TypeScript interfaces |

**Created (7-8 files)**:

| File | Purpose |
| :--- | :--- |
| `src-tauri/src/mcp/mod.rs` | Module root, `McpManager` struct |
| `src-tauri/src/mcp/manager.rs` | Subprocess lifecycle, CRUD (mirrors `channel/manager.rs`) |
| `src-tauri/src/mcp/aggregator.rs` | Multi-server tool aggregation + namespacing |
| `src-tauri/src/mcp/translator.rs` | MCP ↔ OpenAI/Anthropic format conversion + schema sanitization |
| `src-tauri/src/proxy/mcp_tools.rs` | Request injection + response tool_call interception |
| `src-tauri/src/config/watcher.rs` | *(modify)* MCP server hot-reload |
| `src/components/McpServersPanel.tsx` | Frontend panel |
| `src/hooks/useMcp.ts` *(optional)* | React hook for MCP state |

### 7.2 Data Persistence

| Data | Location | Format |
| :--- | :--- | :--- |
| MCP server config | Main `config.toml` | `[[mcp_servers]]` TOML section |
| Virtual keys | `~/.config/modelswitch/virtual_keys.json` | JSON (hashed keys) |
| Sanitizer rules | Main `config.toml` | `[gateway.sanitizer]` section |

### 7.3 Config Compatibility

New config sections (`mcp_servers`, `virtual_keys`, `sanitizer`) all use `#[serde(default)]`, so existing config files without these sections will continue to work. No migration needed — new features activate only when the user adds the corresponding config section.

### 7.4 Testing Strategy

* **Unit tests**: aggregator namespacing, translator format conversion, schema sanitization, virtual key budget math, sanitizer regex matching.
* **Integration tests**: MCP tool injection into request body, tool_call interception from response, budget enforcement per virtual key, streaming sanitizer with split chunks.
* **Manual testing**: real MCP server (`@modelcontextprotocol/server-filesystem`), real LLM provider tool-call round-trip, Tauri app lifecycle (start/stop/force-quit) for subprocess cleanup.

### 7.5 Security Review Triggers

Per project security rules, the following require `security-reviewer` review before merge:
* Virtual Keys (authentication code, key storage).
* MCP subprocess spawning (OS process management, injection risk in command args).
* Sanitizer (must not log redacted content).
