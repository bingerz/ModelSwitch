# ModelSwitch 参考项目对比评估报告

> 基于对 LiteLLM (Python)、New-API (Go)、TensorZero (Rust)、Higress (Envoy/Wasm) 的深度分析

## 1. 评估维度总览

| 维度 | ModelSwitch 现状 | LiteLLM | New-API | TensorZero | 改善空间 |
|------|-----------------|---------|---------|------------|----------|
| 语言/运行时 | Rust + Axum 0.8 + Tauri | Python (FastAPI) | Go (Gin) | Rust (Axum) | — |
| 代理模块行数 | ~4,780 行 | ~33,000+ 行 | ~20,000+ 行 | ~15,000+ 行 | — |
| 路由策略 | 3 种 | 6 种 + 自适应 | 优先级+加权 | 顺序回退 | ⭐⭐⭐ |
| Provider 适配 | 3 种协议入口 | 100+ | 40+ | 20+ | ⭐⭐ |
| 缓存机制 | u64 哈希 + TTL | 语义缓存 + DualCache | 内存通道缓存 | BLAKE3 + ClickHouse/Valkey | ⭐⭐⭐ |
| 流式处理 | SSE keepalive | CustomStreamWrapper | StreamScanner + Ping | BodyStream + 防取消 | ⭐⭐ |
| 计费/配额 | 事后统计 | 预扣+对账 | 预扣+批量DB | nano-dollar精度 | ⭐⭐⭐ |
| 限流 | 单层RPM/TPM | 3层(Redis Lua) | 多层(模型/用户/IP) | 令牌桶(秒/分/时/日/月) | ⭐⭐ |
| 密钥管理 | 单密钥/通道 | — | 多密钥(随机/轮询) | — | ⭐ |
| 客户端断连 | 无处理 | — | 检测+清理 | 防取消保证副作用完成 | ⭐⭐⭐ |

---

## 2. 架构对比

### 2.1 请求调度流程

**ModelSwitch**:
```
Client → virtual_key_auth → sanitizer → dispatch()
  → select_channel(strategy, healthy_channels)
  → try_channel_attempt() → HTTP call → handle_streaming/json_success
  → fallback on failure
```

**LiteLLM**:
```
Client → user_api_key_auth (DualCache) → hooks (budget, rate_limit, cache_check)
  → Router.acompletion() → select deployment → transform_request
  → httpx call → transform_response → cost calc → async spend log
```

**New-API**:
```
Client → auth middleware → rate-limit → Relay()
  → validate → estimate tokens → pre-consume billing → retry loop:
    → channel select (priority+weight) → adaptor.ConvertRequest
    → DoRequest → DoResponse → post-consume billing + log
    → on error: auto-ban channel, check retry, loop
  → refund on failure
```

**TensorZero**:
```
Client → auth → possibly_prevent_request_cancellation
  → ModelConfig::infer() → sequential provider fallback
  → BLAKE3 cache lookup → provider HTTP call
  → non-blocking cache write → rate limit accounting
```

**评估**: ModelSwitch 的调度流程与其他项目基本一致，但缺少两个关键环节：
1. **预扣费** — New-API 和 LiteLLM 都在请求前预扣 quota，防止超额
2. **请求取消保护** — TensorZero 确保客户端断连后副作用（缓存写入、配额记账）仍完成

### 2.2 模块组织

| 项目 | 组织方式 | 评价 |
|------|----------|------|
| ModelSwitch | 按领域拆分 (proxy/, router/, admin/, mcp/) | ✅ 良好 |
| LiteLLM | 单体 proxy_server.py (15K行) + 按端点拆分 | ⚠️ 过大 |
| New-API | 分层架构 (controller → service → model → relay) | ✅ 良好 |
| TensorZero | workspace 40+ crates, thin gateway over thick core | ✅ 优秀 |

**评估**: ModelSwitch 的模块组织已经优于 LiteLLM，与 New-API 相当。TensorZero 的 workspace 多 crate 拆分是最清晰的，但对 ModelSwitch 来说当前的单 crate 模块化已足够。

---

## 3. 功能差距分析

### 3.1 路由策略 (高影响)

**现状**: ModelSwitch 有 3 种策略 — 加权随机、延迟优先、最少繁忙

**差距**:

| 策略 | LiteLLM | New-API | ModelSwitch | 建议 |
|------|---------|---------|-------------|------|
| 加权随机 | ✅ | ✅ (优先级分层) | ✅ | — |
| 最少繁忙 | ✅ | ❌ | ✅ | — |
| 延迟优先 | ✅ (滑动窗口10样本) | ❌ | ✅ | 改进: 用滑动窗口替代单值 |
| 用量优先 | ✅ (TPM/RPM最低) | ❌ | ❌ | **添加** |
| 成本优先 | ✅ (最便宜) | ❌ | ❌ | **添加** |
| 自适应/Bandit | ✅ (Thompson Sampling) | ❌ | ❌ | 考虑: 长期方向 |
| 优先级分层 | ❌ | ✅ | ❌ | **添加** (影响大) |
| 通道亲和性 | ❌ | ✅ (LRU+Redis) | ✅ (基础版) | — |

**建议优先实现**:
1. **用量优先路由** — 基于 TPM/RPM 实时用量选择空闲通道
2. **优先级分层** — 高优先级通道先用完，再降级到低优先级
3. **延迟滑动窗口** — 替代当前单值延迟追踪，用最近 N 次请求的滑动平均

### 3.2 缓存系统 (高影响)

**现状**: ModelSwitch 使用 `DefaultHasher` (u64) + `HashMap<u64, String>` + TTL

**差距**:

| 特性 | ModelSwitch | LiteLLM | TensorZero | New-API | 建议 |
|------|-------------|---------|------------|---------|------|
| 哈希算法 | DefaultHasher (u64) | SHA256 | BLAKE3 | — | **升级** |
| 碰撞检测 | key_material 验证 | ✅ | 前缀无关编码 | — | ✅ 已有 |
| 语义缓存 | ❌ | ✅ (embedding 相似度) | ❌ | ❌ | 考虑 |
| 缓存模式 | 单一 On/Off | On/Off/ReadOnly/WriteOnly | 4种 | — | **添加** |
| 非阻塞写入 | ❌ (同步) | ✅ | ✅ (spawn_ignoring_shutdown) | — | **添加** |
| 流式缓存 | ❌ | ✅ (chunk replay) | ✅ | — | **添加** |
| 分布式缓存 | ❌ (本地) | ✅ (Redis DualCache) | ✅ (ClickHouse/Valkey) | ✅ (Redis) | 考虑 |

**建议优先实现**:
1. **BLAKE3 哈希** — 替代 DefaultHasher，128-bit 抗碰撞，Rust 原生支持
2. **非阻塞缓存写入** — `tokio::spawn` 写入，不阻塞响应路径
3. **缓存模式** — 添加 ReadOnly / WriteOnly 模式
4. **流式响应缓存** — 存储 SSE chunks 并支持 replay

### 3.3 流式处理 (中影响)

**现状**: ModelSwitch 有 SSE keepalive，但缺少客户端断连检测和精确的 usage 追踪

**差距**:

| 特性 | ModelSwitch | LiteLLM | New-API | TensorZero |
|------|-------------|---------|---------|------------|
| SSE keepalive | ✅ | ✅ | ✅ (Ping) | ✅ |
| 客户端断连检测 | ❌ | — | ✅ (Context.Done) | ✅ |
| 副作用保证 | ❌ | — | — | ✅ (防取消) |
| TTFT + 总超时 | 单一超时 | — | — | ✅ (双层) |
| stream_options | ❌ | — | ✅ (force include_usage) | — |
| 缓冲区管理 | 默认 | — | 64KB初始/64MB最大 | — |

**建议优先实现**:
1. **双层流式超时** — TTFT (首 token 超时) + 总超时，解决 LLM 首字延迟 vs 总生成时间的不同 SLA
2. **客户端断连检测** — 监听连接关闭事件，停止上游读取
3. **stream_options.include_usage 注入** — 强制上游在流式响应中返回 token 计数

### 3.4 计费与配额 (高影响)

**现状**: ModelSwitch 使用事后统计，`QuotaStore` 持久化

**差距**:

| 特性 | ModelSwitch | LiteLLM | New-API | TensorZero |
|------|-------------|---------|---------|------------|
| 预扣费 | ❌ | ✅ (估算+预扣) | ✅ (信任配额优化) | — |
| 对账 | ❌ | ✅ (refund/charge delta) | ✅ (post-consume reconcile) | — |
| 精度 | f64 | f64 | decimal | nano-dollar (i128) |
| 批量写入 | 单条 | ✅ (Redis 批量) | ✅ (DB 批量) | — |
| 分层定价 | ❌ | ❌ | ✅ (表达式引擎) | — |
| 缓存计费 | ❌ | ❌ | ✅ (Claude 5m/1h tier) | — |
| 提供商预算 | ❌ | ✅ ($/period) | — | ✅ |

**建议优先实现**:
1. **预扣费 + 对账** — 请求前估算并预扣 quota，完成后对账退还差额
2. **提供商预算限制** — 按提供商设置日/月预算上限
3. **批量配额写入** — 积累多次请求的配额变更后批量写入

### 3.5 限流 (中影响)

**现状**: ModelSwitch 单层 RPM/TPM 限流

**差距**:

| 特性 | ModelSwitch | LiteLLM | New-API | TensorZero |
|------|-------------|---------|---------|------------|
| 算法 | 固定窗口 | 滑动窗口 (Redis Lua) | 滑动窗口 | 令牌桶 |
| 层级 | 通道级 | 通道+用户+团队+模型 | 用户+模型+IP | 标签+API key |
| 原子性 | Mutex (单进程) | TOCTOU-safe (Lua) | Redis/in-memory | — |
| 并发限制 | ❌ | ✅ (max_parallel) | — | — |

**建议**:
1. **并发请求限制** — 添加 per-channel / per-key 最大并发请求数限制
2. **多维度限流** — 从仅通道级扩展到通道+虚拟密钥+模型级
3. 令牌桶算法替代固定窗口（更平滑的限流曲线）

### 3.6 密钥管理 (低影响)

**现状**: ModelSwitch 单密钥/通道

**差距**: New-API 支持多密钥/通道（换行分隔或 JSON 数组），支持随机/轮询模式选择密钥，单个密钥可独立禁用。

**建议**: 后续可考虑支持通道级多密钥轮换，但优先级较低。

---

## 4. 性能优化机会

### 4.1 HTTP 连接池调优 (高影响，低成本)

**现状**: 
```rust
let http_client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(config.gateway.http_timeout_secs))
    .build()
```

**问题**: 只配置了总超时，未调优连接池参数。

**参考**: 
- TensorZero 使用 `TensorzeroHttpClient` 封装 reqwest
- LiteLLM 使用 custom httpx transport with TCP keepalive
- New-API 配置 Go HTTP transport 的 MaxIdleConns/MaxIdleConnsPerHost

**建议**:
```rust
let http_client = reqwest::Client::builder()
    .timeout(Duration::from_secs(timeout))
    .connect_timeout(Duration::from_secs(10))      // 连接建立超时
    .pool_idle_timeout(Duration::from_secs(90))     // 空闲连接保活
    .pool_max_idle_per_host(20)                     // 每主机最大空闲连接
    .tcp_keepalive(Duration::from_secs(60))         // TCP 层 keepalive
    .tcp_nodelay(true)                              // 禁用 Nagle 算法
    .build()
```

**预期效果**: 降低连接建立开销 30-50%（取决于上游 API 的 TLS 握手成本）。

### 4.2 缓存写入非阻塞化 (中影响，低成本)

**现状**: 响应缓存和配额存储在请求路径中同步写入。

**建议**: 使用 `tokio::spawn` 将缓存写入异步化，参考 TensorZero 的 `spawn_ignoring_shutdown` 模式。

### 4.3 客户端断连处理 (高影响，中成本)

**现状**: 客户端断连后，上游 HTTP 请求可能继续运行，浪费配额和连接。

**参考**: TensorZero 的 `possibly_prevent_request_cancellation` 是最有价值的设计 — 在 non-GET 请求中将处理逻辑 spawn 到独立 task，通过 mpsc channel 驱动响应流，确保即使客户端断连，缓存写入/配额记账/速率限制统计仍完整完成。

**建议**: 实现 Axum 等价的断连保护中间件。

### 4.4 请求合并优化 (已有，可改进)

**现状**: `InFlightRequests` 使用 `tokio::sync::Notify` 进行请求合并。

**评估**: 这个设计已经很好，与 LiteLLM 的 DualCache 请求合并相当。可考虑添加等待超时防止无限等待。

---

## 5. 改善路线图建议

### Phase A: 性能快赢 (1-2 天)

1. **HTTP 连接池调优** — 添加 connect_timeout/pool_idle_timeout/tcp_keepalive/tcp_nodelay
2. **BLAKE3 缓存哈希** — 替代 DefaultHasher，128-bit 抗碰撞
3. **非阻塞缓存写入** — tokio::spawn 缓存写入操作
4. **stream_options 注入** — 强制上游返回 usage

### Phase B: 路由增强 (2-3 天)

1. **用量优先路由** — 基于 TPM/RPM 实时用量选择
2. **优先级分层** — 高优先级通道先用，低优先级兜底
3. **延迟滑动窗口** — 最近 N 次请求的加权平均
4. **并发请求限制** — per-channel 最大并发数

### Phase C: 计费增强 (2-3 天)

1. **预扣费 + 对账** — 请求前估算预扣，完成后对账
2. **提供商预算限制** — 按提供商设置日/月预算
3. **批量配额写入** — 积累后批量持久化

### Phase D: 流式健壮性 (1-2 天)

1. **双层流式超时** — TTFT + 总超时
2. **客户端断连检测** — 监听连接关闭
3. **副作用保证** — 断连后仍完成缓存写入/配额记账
4. **流式响应缓存** — 存储 SSE chunks 并支持 replay

### Phase E: 高级特性 (长期)

1. **缓存模式** — ReadOnly / WriteOnly
2. **语义缓存** — 基于 embedding 相似度
3. **多密钥通道** — 通道级密钥轮换
4. **分层定价** — 表达式引擎
5. **自适应路由** — Thompson Sampling bandit

---

## 6. ModelSwitch 独特优势

与参考项目相比，ModelSwitch 有以下独有优势应保持：

1. **Rust 性能** — 零成本抽象，无 GC 停顿，内存安全
2. **Tauri 桌面模式** — 独有的桌面 GUI + 本地网关一体化
3. **MCP 网关模式** — 内置 MCP 工具自动注入循环，业界罕见
4. **隐私过滤器** — 内置 sanitizer 中间件，请求级 PII 脱敏
5. **Claude Code 协议** — provider-prefixed 路由支持 agentic 工具链
6. **虚拟密钥** — 与 LiteLLM 相当的虚拟密钥管理
7. **配置热更新** — 全组件热加载（channels, MCP, rate_limiter, payload_rules）

---

## 7. 总结

ModelSwitch 在架构组织上已经达到参考项目的水平（优于 LiteLLM 的单体结构，与 New-API 相当）。主要差距集中在：

| 领域 | 差距程度 | ROI |
|------|----------|-----|
| HTTP 连接池调优 | 小差距 | ⭐⭐⭐⭐⭐ |
| 缓存哈希升级 | 小差距 | ⭐⭐⭐⭐⭐ |
| 客户端断连保护 | 中差距 | ⭐⭐⭐⭐ |
| 预扣费+对账 | 中差距 | ⭐⭐⭐⭐ |
| 路由策略扩展 | 中差距 | ⭐⭐⭐ |
| 流式超时分层 | 小差距 | ⭐⭐⭐⭐ |
| 语义缓存 | 大差距 | ⭐⭐ |

**建议从 Phase A (性能快赢) 开始**，这 4 项改进预计 1-2 天即可完成，但能显著提升代理性能和可靠性。
