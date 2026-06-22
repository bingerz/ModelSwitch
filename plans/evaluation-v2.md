# ModelSwitch 架构评估报告 V2（参考项目对比）

> 评估日期: 2026-06-16  
> 基于代码库: 79 个 Rust 源文件, 15,539 行, 185 个测试  
> 参考项目: LiteLLM (Python), New-API (Go), TensorZero (Rust), Higress (Envoy/Wasm), CLIProxyAPI (Go)

---

## 背景

本轮评估在第一轮重构（26 项改进全部完成）和参考项目对比实施（13 项改进 Phases A-E 完成）之后进行。目标是识别当前代码库中仍然存在的架构问题、功能缺口和性能瓶颈。

---

## 一、正确性问题（Bug）

### BUG-1: 流式响应费用未对账 ⚠️ 严重

**位置**: `proxy/dispatch.rs:134-161` + `proxy/response.rs:196-199`

**问题**: dispatch 在请求前调用 `reserve_spend()` 预扣费用。对于非流式响应，成功后通过 `reconcile_spend()` 调整差额。但流式响应路径中，`handle_streaming_success` 调用的是 `accumulate_spend()` 而非 `reconcile_spend()`。这意味着：
1. 预扣金额保留在计数器中
2. `accumulate_spend()` 再次添加实际费用
3. 用户被双重收费（预扣 + 实际）

**修复**: 流式响应完成后的后台遥测任务需要知道预扣金额并调用 `reconcile_spend()` 而非 `accumulate_spend()`。需要将 `reserved_cents` 传递到流式响应处理路径。

### BUG-2: stream_options 导致缓存投毒

**位置**: `proxy/cache.rs:175-183` + `proxy/attempt.rs:157-176`

**问题**: 缓存键计算时排除 `stream` 字段但未排除 `stream_options`。而 `attempt.rs` 在发送上游请求前会注入 `stream_options: {include_usage: true}`。两个相同 body 的请求（一个流式一个非流式）可能因 `stream_options` 不同而收到不同的上游响应，却共享缓存键。

**修复**: 在 `canonical_key_material()` 中同时排除 `stream` 和 `stream_options` 字段。

### BUG-3: 配置热更新竞态

**位置**: `config/watcher.rs:60-133`

**问题**: 配置观察者读取 `channel_mgr.list()` 然后逐个修改通道。如果调度在此期间读取通道列表，会看到部分更新的状态。不是原子的。

**影响**: 低概率但在高并发下可能导致不一致的路由决策。

---

## 二、性能改进空间

### PERF-1: HTTP/2 连接池停滞 🔴 高影响

**参考**: TensorZero (`reference/tensorzero/crates/tensorzero-http/src/lib.rs`)

**问题**: `reqwest`/`hyper` 对每个 HTTP/2 host:port 只开一个 TCP 连接。如果并发请求超过 `max_concurrent_streams`（默认 100），请求会排队等待。ModelSwitch 使用单个 `reqwest::Client` 实例处理所有上游请求。

**TensorZero 的解决方案**: 维护一个 `OnceCell<LimitedClient>` 数组（最多 1024 个），每个客户端跟踪并发请求数（最大 100）。当现有客户端都满载时，惰性创建新客户端。使用 RAII ticket（`Drop` 时递减计数器）确保正确的清理。

**建议**: 实现 `HttpClientPool` — 多 `reqwest::Client` 实例 + 并发跟踪。对 Anthropic 和 Google（HTTP/2 提供商）影响最大。

### PERF-2: 流式响应 read_timeout 缺失

**参考**: TensorZero

**问题**: 当前的 TTFT 超时只覆盖首字节到达前的等待。如果流开始后中途停止发送数据（stalled stream），没有任何超时机制会触发。后台遥测轮询循环（200ms 间隔，最多 150 轮 = 30s）最终会结束，但客户端在此期间一直处于等待状态。

**建议**: 在 `reqwest::Client::builder()` 上添加 `.read_timeout()`，或者在 SSE 流上使用 `tokio::time::timeout` 包装每个 chunk 的接收。

### PERF-3: RateLimiter 每次 O(n) 裁剪

**位置**: `proxy/rate_limiter.rs:44-47`

**问题**: `current_total()` 在 `check()` 和 `record()` 中都被调用，每次调用都会裁剪滑动窗口（`retain()` 扫描）。对于一个有 N 个通道的系统，每次调度尝试触发 2N 次裁剪操作。

**影响**: 在高吞吐 + 多通道场景下可能成为瓶颈。

**建议**: 改为惰性裁剪 — 只在 `record()` 中裁剪对应通道的窗口，而非每次 `check()` 都全量扫描。或者使用 `TokenBucket` 替代滑动窗口。

### PERF-4: DispatchLogger 读锁阻塞写入

**位置**: `log.rs:175-232`

**问题**: `usage_history()` 和 `cost_stats()` 持有读锁遍历全部日志。如果日志条目很多（接近 1000 条上限），这会阻塞写入操作。

**建议**: 使用快照模式 — 在读锁内 clone 出需要的聚合数据，或预计算 hourly bucket 聚合。

---

## 三、架构改进

### ARCH-1: Provider Adaptor Trait

**参考**: New-API (`reference/new-api/relay/channel/adapter.go`)

**问题**: ModelSwitch 的三个提供商模块（`proxy/openai.rs`, `proxy/anthropic.rs`, `proxy/gemini.rs`）各自独立实现请求处理，没有共同的 trait 接口。新提供商需要修改多个文件。

**New-API 的方案**: `Adaptor` 接口定义了 `GetRequestURL`, `SetupRequestHeader`, `ConvertOpenAIRequest`, `ConvertClaudeRequest`, `DoRequest`, `DoResponse` 等方法。

**建议**: 定义 `ProviderAdapter` trait 用于统一提供商接口。

### ARCH-2: 协议翻译注册表

**参考**: CLIProxyAPI (`reference/CLIProxyAPI/internal/translator/translator/translator.go`)

**问题**: 当前 `proxy/translate.rs` 硬编码了 OpenAI ↔ Gemini 翻译。添加新协议对需要修改翻译模块。

**建议**: 实现翻译注册表，支持动态注册协议对。

### ARCH-3: 可观测性 — Prometheus 指标

**问题**: 无任何指标导出。无 `/metrics` 端点。

**建议**:
1. 添加 `prometheus` crate
2. 暴露 `/metrics` 端点
3. 记录: 请求计数、延迟直方图、缓存命中率、并发请求数、熔断器状态

---

## 四、功能增强

### FEAT-1: 多密钥通道 (Multi-Key Channels)

**参考**: New-API

**问题**: 每个通道只支持一个 API 密钥。需要更高配额时，用户必须创建多个通道。

**建议**: 在 `Channel` 结构体中添加 `api_keys: Vec<String>` 字段，调度时轮换选择。

### FEAT-2: 最低成本路由策略

**参考**: LiteLLM (`lowest_cost.py`)

**建议**: 添加 `LowestCostStrategy` — 基于模型定价表选择最低成本的通道。

### FEAT-3: 前缀缓存感知路由

**参考**: Higress (`ai-load-balancer/prefix_cache/`)

**建议**: 长期方向。路由到已经缓存了最长匹配前缀的端点。

### FEAT-4: 基于标签的路由

**参考**: LiteLLM (`tag_based_routing.py`)

**建议**: 支持请求 metadata tags，按 tag 匹配通道组。

### FEAT-5: 通道级别模型映射

**参考**: New-API

**建议**: 在 `ChannelConfig` 中增加 `model_mapping: HashMap<String, String>` 字段。

---

## 五、代码质量

### QUAL-1: 未测试模块

以下关键模块零测试覆盖：
- `config.rs` / `config/watcher.rs`
- `channel/manager.rs`
- `health/`
- `admin/*.rs`
- `server.rs`
- `quota/collectors/*.rs`

### QUAL-2: 死代码

以下函数标记为 `#[allow(dead_code)]` 但从未使用：
- `FailureReason::ModelFallback`
- `detect_sse_error` / `sse_error_event`
- `to_anthropic_tool` / `parse_mcp_tool_call`

### QUAL-3: 流式遥测轮询效率

后台遥测任务每 200ms 轮询检查 chunks 是否稳定，对于长流式响应多达 150 次轮询。改为基于 stream completion 事件更高效。

---

## 六、优先级排列

### P0 — 必须修复（正确性）
| # | 问题 | 影响 | 复杂度 |
|---|------|------|--------|
| BUG-1 | 流式响应费用未对账 | 双重收费 | 中 |
| BUG-2 | stream_options 缓存投毒 | 缓存返回错误响应 | 低 |
| BUG-3 | 配置热更新竞态 | 短暂路由不一致 | 低 |

### P1 — 高影响改进
| # | 改进 | 影响 | 复杂度 | 参考 |
|---|------|------|--------|------|
| PERF-1 | HTTP/2 连接池 | 高并发下防止停滞 | 高 | TensorZero |
| PERF-2 | 流式 read_timeout | 防止 stalled stream | 低 | TensorZero |
| ARCH-3 | Prometheus 指标 | 可观测性 | 中 | 通用 |
| FEAT-1 | 多密钥通道 | 配额扩展 | 中 | New-API |

### P2 — 中等优先级
| # | 改进 | 影响 | 复杂度 | 参考 |
|---|------|------|--------|------|
| PERF-3 | RateLimiter 优化 | 高吞吐性能 | 中 | — |
| ARCH-1 | Provider Adaptor trait | 可扩展性 | 高 | New-API |
| FEAT-2 | 最低成本路由 | 成本优化 | 中 | LiteLLM |
| FEAT-5 | 通道级模型映射 | 灵活性 | 低 | New-API |
| QUAL-1 | 关键模块测试 | 质量保障 | 中 | — |

### P3 — 长期方向
| # | 改进 | 影响 | 复杂度 | 参考 |
|---|------|------|--------|------|
| ARCH-2 | 翻译注册表 | 协议扩展 | 高 | CLIProxyAPI |
| FEAT-3 | 前缀缓存路由 | 性能 | 高 | Higress |
| FEAT-4 | 标签路由 | 多租户 | 中 | LiteLLM |
| PERF-4 | 日志聚合优化 | 可观测性 | 低 | — |
| QUAL-2 | 死代码清理 | 可维护性 | 低 | — |
| QUAL-3 | 遥测轮询优化 | 效率 | 低 | — |

---

## 七、总结

### 当前状态

| 维度 | 评分 | 说明 |
|------|------|------|
| 架构设计 | 8/10 | 模块化良好，子结构分解清晰 |
| 功能完整性 | 7/10 | 核心功能完备，缺多密钥/成本路由/Prometheus |
| 性能 | 7/10 | BLAKE3/连接池调优/TTFT 已实现，HTTP/2 池是瓶颈 |
| 代码质量 | 7/10 | 185 测试/零 clippy，但关键模块零覆盖 |
| 正确性 | 7/10 | 3 个 bug 待修复 |
| 可观测性 | 4/10 | 仅 ndjson 日志，无指标导出 |

### 改进路线图建议

**Phase F: 正确性修复** (1-2 天)
- BUG-1: 流式响应 reserve/reconcile 修复
- BUG-2: 缓存键排除 stream_options
- BUG-3: 配置热更新原子化

**Phase G: 性能 + 可观测性** (2-3 天)
- PERF-1: HTTP/2 多客户端池
- PERF-2: 流式 read_timeout
- ARCH-3: Prometheus /metrics 端点

**Phase H: 功能增强** (2-3 天)
- FEAT-1: 多密钥通道
- FEAT-5: 通道级模型映射
- PERF-3: RateLimiter 优化

**Phase I: 架构改进** (长期)
- ARCH-1: Provider Adaptor trait
- FEAT-2: 最低成本路由
- QUAL-1: 关键模块测试覆盖
