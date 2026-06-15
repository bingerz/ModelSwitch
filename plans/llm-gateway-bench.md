# llm-gateway-bench — 独立 LLM 网关 Benchmark 工具计划

> **版本**: v1.2 · **日期**: 2026-06-13 · **状态**: Draft

---

## 1 项目概述

### 1.1 目标

构建一个**独立的、通用的 LLM 网关性能基准测试工具**，能够对任何 OpenAI/Anthropic 兼容的 LLM 代理网关进行标准化性能评测。

**当前放置位置**：`ModelSwitch/benchmark/`，作为 ModelSwitch 项目下的独立功能模块。成熟后可直接提取为独立项目。

### 1.2 核心设计原则

1. **零耦合** — 不 import ModelSwitch 任何代码，纯 HTTP 协议层交互
2. **独立 Cargo 项目** — 自己的 `Cargo.toml`、`README.md`，不在 ModelSwitch workspace 内
3. **可直接提取** — 后续 `git filter-branch` 或直接复制 `benchmark/` 目录即为完整项目
4. **通用性** — 可指向任何 OpenAI/Anthropic 兼容端点（ModelSwitch / litellm / CLIProxyAPI / higress / 直连 vLLM）

### 1.3 参考项目调研

| 项目 | 语言/框架 | Benchmark 能力 | 借鉴要点 |
|------|----------|---------------|---------|
| **litellm** | Python (FastAPI) | 最完整：proxy-vs-provider 延迟对比、pytest-benchmark 热路径测试、内存泄漏检测 | 8ms P95 overhead @ 1k RPS；DualCache 架构；asyncio 负载生成器 |
| **CLIProxyAPI** | Go (Gin) | 无内置 benchmark | 多协议桥接、OAuth 提取、cooldown 机制 |
| **higress** | Go (Envoy Wasm) | 无 LLM 专项 benchmark（生产验证） | LLM 专项 LB 策略（prefix_cache、endpoint_metrics、LeastFirstTokenLatency） |
| **tensorzero** | Rust | Criterion 微基准 | 纯 Rust 异步、P99 < 1ms 目标 |

### 1.4 目标网关内部机制与评测影响

为了精确测试大模型网关（如 ModelSwitch），基准测试工具必须能够感知并合理触发/规避以下网关内部机制：
1. **并发合并 (In-Flight Request Merging)**：ModelSwitch 对相同的提示词请求，可能在后台合并为同一个上游请求以节省 Token。测试时需通过随机化消息内容来避免触发合并，或特意设计测试用例验证合并效果。
2. **请求缓存 (Request Cache)**：网关支持语义或完全匹配的缓存。若需要测试网关的真实转发性能（如路由、限流），需要通过参数生成不重复的请求。
3. **滑动窗口限流 (Sliding Window Rate Limiting)**：ModelSwitch 内部限流采用滑动窗口算法（基于时间戳列表），而非令牌桶算法。测试工具自身限速（`--rps`）使用令牌桶以生成平滑流量，但需注意被测网关的滑动窗口在临界点处的反应。
4. **熔断机制 (Circuit Breaker)**：在高比例 429 或 5xx 错误时，网关可能会触发熔断直接返回错误，测试工具应能记录并区分网关主动熔断与上游错误的比例。
5. **流式保活 (Keepalive Stream)**：流式响应中，ModelSwitch 在上游无响应时可能会定期发送保活 chunk（如空格或特定注释），测试工具的流式解析器需要能过滤此类无关的 Keepalive 信号。

---

## 2 目录结构

```
ModelSwitch/
├── src-tauri/              # ModelSwitch 主项目（不改动）
├── src/                    # 前端（不改动）
└── benchmark/              # ← 新建：完全独立的 benchmark 项目
    ├── Cargo.toml          # 独立 crate，不属于任何 workspace
    ├── README.md           # 项目文档
    ├── .gitignore
    ├── src/
    │   ├── main.rs         # CLI 入口 (clap)
    │   ├── lib.rs          # 库导出（可被其他项目引用）
    │   │
    │   ├── client/         # HTTP 客户端封装
    │   │   ├── mod.rs
    │   │   ├── openai.rs   # OpenAI /v1/chat/completions 格式
    │   │   ├── anthropic.rs # Anthropic /v1/messages 格式
    │   │   └── stream.rs   # SSE 流式读取与计时
    │   │
    │   ├── scenarios/      # 测试场景定义
    │   │   ├── mod.rs      # Scenario trait + 注册
    │   │   ├── chat.rs     # 非流式 chat completions
    │   │   ├── streaming.rs # 流式 SSE（TTFB + 完成时间）
    │   │   ├── mixed.rs    # 混合读写负载（流式 + 非流式）
    │   │   ├── burst.rs    # 突发并发
    │   │   └── sustained.rs # 持续 RPS 压测
    │   │
    │   ├── mock/           # 内置 Mock LLM Server
    │   │   ├── mod.rs      # 启动/管理
    │   │   └── server.rs   # axum mock：可控延迟、可控429、可控token数
    │   │
    │   ├── metrics/        # 指标采集与统计
    │   │   ├── mod.rs
    │   │   ├── histogram.rs # hdrhistogram 延迟直方图（P50/P95/P99）
    │   │   ├── recorder.rs  # 单请求计时器
    │   │   └── summary.rs   # 聚合统计
    │   │
    │   ├── report/         # 报告输出
    │   │   ├── mod.rs
    │   │   ├── json.rs     # JSON 格式
    │   │   ├── markdown.rs # Markdown 表格 + 图表
    │   │   └── compare.rs  # 多次运行对比（方差分析）
    │   │
    │   └── runner/         # 测试编排引擎
    │       ├── mod.rs
    │       ├── workload.rs  # 并发控制 + 速率限制
    │       └── phases.rs    # warmup → ramp → steady → cooldown
    │
    ├── benches/            # Criterion 微基准（测自身性能开销）
    │   └── self_bench.rs   # 确保 benchmark 工具自身不成为瓶颈
    │
    └── examples/
        ├── bench_modelswitch.rs  # 示例：压测 ModelSwitch
        ├── bench_direct.rs       # 示例：压测直连 LLM（做基线）
        └── compare_overhead.rs   # 示例：自动对比直连 vs 代理
```

---

## 3 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| 异步运行时 | `tokio` | 与 ModelSwitch 一致，高性能 |
| HTTP 客户端 | `reqwest` (async) | 连接池复用，启用 `http2` 与 `rustls-tls`，与真实 IDE/生产网关行为一致 |
| CLI 框架 | `clap` (derive) | 标准 Rust CLI 体验 |
| 延迟统计 | `hdrhistogram` | 纳秒级精确百分位，比 litellm 的 std_dev 更强 |
| Mock Server | `axum` | 轻量，支持 HTTP/2，可模拟可控延迟/429/超时 |
| 并发控制 | `tokio::sync::Semaphore` + `governor` (Token Bucket) | 精确控制并发数与平滑 RPS 限流，基于成熟的令牌桶库以保证高并发下时序精度 |
| 序列化 | `serde` + `serde_json` | 报告输出 |
| 自身基准 | `criterion` (dev-dep) | 确保 benchmark 工具自身开销可忽略 |

---

## 4 CLI 设计

### 4.1 基本用法

```bash
# 压测任意 OpenAI 兼容网关
gateway-bench \
  --target http://127.0.0.1:8080 \
  --scenario streaming \
  --concurrency 50 \
  --duration 60s \
  --warmup 10s

# 使用内置 Mock LLM（无需真实 API key）
gateway-bench \
  --target http://127.0.0.1:8080 \
  --mock-upstream \
  --scenario burst \
  --burst-size 200
```

### 4.2 对比模式

```bash
# 直连 vs 经代理
gateway-bench compare \
  --direct https://api.openai.com \
  --proxy http://127.0.0.1:8080 \
  --scenario mixed \
  --concurrency 100
```

### 4.3 输出报告

```bash
# JSON + Markdown 双格式
gateway-bench ... --report json:results.json --report markdown:results.md
```

### 4.4 全部 CLI 参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--target <URL>` | 必填 | 目标网关地址 |
| `--scenario <NAME>` | `chat` | 测试场景：chat / streaming / mixed / burst / sustained |
| `--concurrency <N>` | `10` | 并发连接数 |
| `--duration <TIME>` | `30s` | 测试持续时间 |
| `--warmup <TIME>` | `5s` | 预热时间（不计入统计） |
| `--rps <N>` | 无限制 | 目标 RPS（限速模式，使用令牌桶平滑限流） |
| `--api-key <KEY>` | `dummy` | API Key（填任意字符测试代理） |
| `--model <NAME>` | `gpt-4` | 请求的模型名 |
| `--mock-upstream` | false | 启动内置 Mock LLM 作为上游 |
| `--mock-port <PORT>` | `0` | Mock Server 端口（`0` 表示动态分配随机空闲端口） |
| `--tls-skip-verify` | false | 忽略不安全的 TLS 证书验证（测试本地/自签名 HTTPS 网关） |
| `--http2-only` | false | 强制使用 HTTP/2 协议 |
| `--pool-max-idle <N>`| `100` | 连接池最大空闲连接数（避免高并发时频繁关闭/重建连接） |
| `--report <FMT:PATH>` | 无 | 报告输出：`json:path` 或 `markdown:path` |
| `--protocol <NAME>` | `openai` | 协议格式：openai / anthropic / gemini |
| `--stream` | false | 是否流式（覆盖参数，如果不指定，默认根据场景自动推断） |
| `--timeout <SECS>` | `120` | 超时时间（秒），对齐 ModelSwitch 默认的 120 秒 |
| `--message-tokens <N>` | `10` | 默认生成的消息长度（Token/词数），用于控制请求大小 |
| `--mix-ratio <RATIO>` | `70:30` | mixed 场景下的流式与非流式比例 |
| `--runs <N>` | `1` | 多次运行进行方差与稳定性分析 |

> **注**：`--stream` 为可选覆盖标志。不指定时，流式场景（如 `streaming`、`mixed` 中的流式请求部分）会自动启用流式传输，普通场景（如 `chat`）则默认非流式。

---

## 5 测试场景设计

### 5.1 场景定义

| 场景 | 模拟的负载 | 核心指标 | 适用场景 |
|------|-----------|---------|---------|
| **chat** | 非流式 `/v1/chat/completions`（stream=false） | P50/P95/P99 延迟、RPS、错误率 | 基础代理转发性能 |
| **streaming** | 流式 `/v1/chat/completions`（stream=true） | TTFB（首 chunk 延迟）、chunk 间隔、总完成时间 | SSE 流式透传质量 |
| **mixed** | 混合读写负载（流式 + 非流式，比例通过 `--mix-ratio` 配置，默认 70:30） | 综合延迟分布、流式/非流式分离统计 | 真实混合负载 |
| **burst** | 瞬间 N 个并发请求 | 峰值延迟、排队等待时间 | 突发流量承受力 |
| **sustained** | 恒定 RPS 持续 N 秒 | 稳态延迟、延迟漂移趋势 | 长时间稳定性 |

### 5.2 Mock LLM Server 设计

内置 Mock Server 模拟真实 LLM 行为，无需 API Key，支持动态端口绑定，避免 CI/多人环境端口冲突。
Mock Server 须同时支持明文 HTTP/2 (h2c) 与带 TLS 的 HTTP/2 (h2) 协议形式，支持 OpenAI、Anthropic 以及 Gemini 协议的请求解析和 mock 响应。

| 可控参数 | 说明 | 示例 |
|---------|------|------|
| `--mock-port <PORT>` | 模拟服务器端口，默认 `0`（动态随机端口） | `--mock-port 9999`（固定端口） |
| `--mock-delay <MS>` | 模拟 LLM 处理延迟 | `--mock-delay 200`（200ms 响应） |
| `--mock-stream-chunks <N>` | 模拟流式输出 chunk数 | `--mock-stream-chunks 50` |
| `--mock-fail-rate <PCT>` | 模拟 429 错误率 | `--mock-fail-rate 30`（30% 返回 429） |
| `--mock-tokens <N>` | 模拟返回的 token 数量 | `--mock-tokens 500` |

Mock Server 架构：

```
gateway-bench --mock-upstream --mock-port 0
    │
    ├── 启动 Mock LLM Server (:RANDOM_PORT)
    │   ├── 支持 HTTP/2 协议 (h2c / h2)
    │   ├── 支持多协议 (OpenAI / Anthropic / Gemini)
    │   ├── POST /v1/chat/completions  → 延迟 N ms → 返回模拟响应
    │   ├── POST /v1/chat/completions (stream=true) → SSE 逐 chunk 输出
    │   └── 随机按 fail-rate 返回 429
    │
    └── 自动配置压测 target (将 RANDOM_PORT 传递给被压测网关/直连客户端)
        ModelSwitch → Mock LLM (:RANDOM_PORT)
```

### 5.3 ModelSwitch 专项测试场景

针对 ModelSwitch 的特有机制，需要执行以下测试场景：
1. **并发合并测试 (In-Flight Request Merging)**：
   - 发送两个完全相同的提示词，在 Mock Server 端验证是否只收到了一次真实请求；
   - 监控 P99 是否未翻倍，并且在极短时间差内返回了同一内容。
2. **缓存击穿与命中测试 (Cache Hit vs Bypass)**：
   - 使用固定的提示词，进行多次请求，测试 P99 延迟是否降至 < 2ms（命中 RequestCache 路径）；
   - 传入随机参数或随机消息（通过 `--message-tokens` 及随机文本生成），测试网关在穿透缓存时的路由转发性能。
3. **流式保活拦截测试 (Keepalive Filtering)**：
   - 上游 Mock Server 模拟慢速回复，并在空闲期发送保活 chunk。验证压测客户端是否能正确滤除保活 chunk，不计入 TTFB 且不影响最终 Token Count 统计。

---

## 6 对比模式设计

### 6.1 架构

借鉴 litellm 的 `benchmark_proxy_vs_provider.py`，但做得更强：

```
┌─────────────┐         ┌───────────────┐
│ Direct Test │─────►───│ api.openai.com│  ← 基线延迟
└─────────────┘         └───────────────┘
                                               代理开销 = Proxy - Direct
┌─────────────┐         ┌───────────────┐         ┌───────────────┐
│ Proxy Test  │─────►───│ ModelSwitch   │─────►───│ api.openai.com│
└─────────────┘         │ :8080         │         └───────────────┘
                        └───────────────┘
```

### 6.2 报告输出示例

```
╔══════════════════════════════════════════════════════════╗
║  LLM Gateway Benchmark Report                            ║
║  Target: http://127.0.0.1:8080 | Scenario: streaming     ║
╠══════════════════════════════════════════════════════════╣
║  Metric           │ Direct     │ Via Proxy  │ Overhead   ║
╠═══════════════════╪════════════╪════════════╪═══════════╣
║  P50 Latency      │  142ms     │  146ms     │  +4ms      ║
║  P95 Latency      │  310ms     │  318ms     │  +8ms      ║
║  P99 Latency      │  520ms     │  535ms     │  +15ms     ║
║  TTFB (stream)    │   85ms     │   89ms     │  +4ms      ║
║  Connection Est   │   12ms     │   14ms     │  +2ms      ║
║  Throughput (RPS) │  486       │  472       │  -2.9%     ║
║  Error Rate       │  0.0%      │  0.0%      │  —         ║
╚══════════════════════════════════════════════════════════╝
```

### 6.3 多次运行方差分析

```bash
gateway-bench ... --runs 5 --report compare:results.md
```

输出每次运行的 P95/P99 值及标准差，判断结果是否稳定。

---

## 7 指标体系

### 7.1 核心指标

| 指标 | 说明 | 计算方式 |
|------|------|---------|
| **P50/P95/P99 Latency** | 延迟百分位 | hdrhistogram 记录每请求端到端延迟 |
| **TTFB** (Time To First Byte) | 流式首 chunk 延迟 | 请求发出 → 第一个 SSE chunk 到达 |
| **Connection Est Time** | TCP/TLS 连接建立耗时 | socket 建立 + TLS 握手耗时（评估网络开销与池复用效果） |
| **Stream End Delay** | 流式传输结束延迟 | 最后一个数据 chunk 到达 → 发生 HTTP 链接关闭或 Stream EOF (用于评估网关后台 Telemetry 轮询等任务的延迟开销) |
| **RPS** (Requests Per Second) | 吞吐量 | 成功请求数 / 测试持续时间 |
| **Error Rate** | 错误率 | 非 2xx 响应数 / 总请求数 |
| **Streaming Chunk Interval** | SSE chunk 间隔统计 | 相邻 chunk 到达时间差，输出 P50/P95 分布及方差 |
| **Concurrency Saturation** | 并发饱和点 | RPS 不再随并发增加的拐点 |

### 7.2 执行阶段

```
│ Warmup       │ Ramp-up         │ Steady State      │ Cooldown    │
│ (不计入统计)  │ (线性增加并发)   │ (恒定并发，计数)   │ (等待尾部)   │
└──────────────┴─────────────────┴───────────────────┴─────────────┘
   5s              10s                 60s                 10s
```

---

## 8 Cargo.toml 设计

```toml
[package]
name = "llm-gateway-bench"
version = "0.1.0"
edition = "2021"
description = "A benchmark tool for LLM proxy gateways"
license = "MIT"

[[bin]]
name = "gateway-bench"
path = "src/main.rs"

[lib]
name = "llm_gateway_bench"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# HTTP
reqwest = { version = "0.12", features = ["stream", "json", "rustls-tls", "http2"] }
axum = { version = "0.7", features = ["http2"] }          # for mock server with HTTP/2 support

# CLI
clap = { version = "4", features = ["derive"] }

# Metrics
hdrhistogram = { version = "7", features = ["serialization"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Utilities
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
futures = "0.3"
tokio-stream = "0.1"
bytes = "1"
uuid = { version = "1", features = ["v4"] }
governor = "0.6"  # 高并发高精度令牌桶限流库

[dev-dependencies]
criterion = { version = "0.5", features = ["async_tokio"] }

[[bench]]
name = "self_bench"
harness = false
```

---

## 9 实现阶段

| 阶段 | 内容 | 产出 | 工时 |
|------|------|------|------|
| **Phase 1** | 项目骨架 + Mock Server (动态端口) + 基础 metrics | 能跑通 `cargo run -- --mock --scenario chat` | 1.5 天 |
| **Phase 2** | 全部 5 个场景 + CLI 完整参数 | `gateway-bench --scenario X --concurrency N --duration T` | 1-2 天 |
| **Phase 3** | 对比模式 + Markdown/JSON 报告 | `gateway-bench compare --direct ... --proxy ...` | 1 天 |
| **Phase 4** | Criterion 自身基准 + examples + 内存泄漏与瓶颈检测 | 工具自身开销验证 + 内存监控机制 | 1 天 |

### Phase 1 详细任务

| 任务 | 文件 | 说明 |
|------|------|------|
| P1-01 | `Cargo.toml` | 项目配置，依赖声明 |
| P1-02 | `src/main.rs` + `src/lib.rs` | CLI 入口 + 库导出 |
| P1-03 | `src/mock/server.rs` | Mock LLM Server：可控延迟、429 模拟、**支持动态随机端口与 HTTP/2** |
| P1-04 | `src/metrics/` | hdrhistogram 封装 + 单请求计时 + **连接建立耗时统计** |
| P1-05 | `src/client/openai.rs` | OpenAI 格式请求客户端，配置 `rustls` + 连接池优化 |
| P1-06 | `src/scenarios/chat.rs` | 非流式 chat 场景 |
| P1-07 | `src/runner/` | 并发控制 (令牌桶算法) + warmup/steady 阶段 |
| P1-08 | `src/report/markdown.rs` | 基础 Markdown 报告输出 |

**Phase 1 验收标准**：`cargo run -- --target http://127.0.0.1:8080 --mock-upstream --scenario chat --concurrency 10 --duration 10s` 输出 P50/P95/P99 延迟及连接建立耗时报告。

### Phase 2 详细任务

| 任务 | 文件 | 说明 |
|------|------|------|
| P2-01 | `src/client/stream.rs` | SSE 流式读取 + TTFB 计时 |
| P2-02 | `src/scenarios/streaming.rs` | 流式场景 |
| P2-03 | `src/scenarios/mixed.rs` | 混合负载场景，支持自定义 `--mix-ratio` 比例 |
| P2-04 | `src/scenarios/burst.rs` | 突发并发场景 |
| P2-05 | `src/scenarios/sustained.rs` | 持续 RPS 场景 |
| P2-06 | `src/client/anthropic.rs` + `src/client/gemini.rs` | Anthropic 和 Gemini 格式支持 |
| P2-07 | CLI 参数完善 | 包含 `--timeout`、`--message-tokens`、`--runs` 等所有新参数可用 |

**Phase 2 验收标准**：5 个场景均可运行，流式场景输出 TTFB 与 Chunk 间隔分布指标，并且多协议客户端可以正常转发。

### Phase 3 详细任务

| 任务 | 文件 | 说明 |
|------|------|------|
| P3-01 | `src/report/compare.rs` | 对比报告生成 |
| P3-02 | `src/report/json.rs` | JSON 报告格式 |
| P3-03 | CLI `compare` 子命令 | 直连 vs 代理自动对比 |
| P3-04 | 多次运行方差分析 | `--runs N` 支持 |

**Phase 3 验收标准**：`gateway-bench compare` 输出完整对比表格。

### Phase 4 详细任务

| 任务 | 文件 | 说明 |
|------|------|------|
| P4-01 | `benches/self_bench.rs` | Criterion 自身开销基准，测试客户端序列化与事件流解析瓶颈 |
| P4-02 | `examples/bench_modelswitch.rs` | ModelSwitch 压测与专项用例（Cache, In-Flight Merging, Keepalive）验证示例 |
| P4-03 | `examples/bench_direct.rs` | 直连基线示例 |
| P4-04 | `examples/compare_overhead.rs` | 自动对比示例 |
| P4-05 | `README.md` | 完整文档 |
| P4-06 | 内存与瓶颈检测评估 | 集成 `jemalloc-ctl` 监控 RSS，或使用 `heaptrack` 进行 2 小时长稳压测，验证无内存泄露 |

**Phase 4 验收标准**：`cargo bench` 确认工具自身开销 < 1ms；监控并记录 2 小时持续高并发压测下的 RSS 内存变化，无泄露。

---

## 10 独立化迁移路径

后续提取为独立项目时只需：

```bash
# 方法 1：直接复制
cp -r benchmark/ ../llm-gateway-bench/

# 方法 2：git subtree
git subtree split --prefix=benchmark -b benchmark-standalone
git push <new-remote> benchmark-standalone:main
```

迁移检查清单：
- [ ] Cargo.toml 不依赖 ModelSwitch 任何路径
- [ ] 无 `path = "../src-tauri"` 等相对路径引用
- [ ] README.md 自包含完整使用说明
- [ ] examples 可独立运行
- [ ] CI 配置（必要）

---

## 11 与 ModelSwitch 的协作关系

### 11.1 当前阶段

- benchmark 作为 ModelSwitch 项目目录下的独立模块
- 不共享 Cargo workspace，不共享代码
- 仅通过 HTTP 协议交互

### 11.2 未来独立后

- benchmark 独立仓库，独立版本管理
- 可集成到 ModelSwitch CI 中作为回归测试
- 可发布到 crates.io 供社区使用

### 11.3 ModelSwitch NFR 验证映射

| NFR | 验证方式 | Benchmark 场景 |
|-----|---------|---------------|
| NFR-01: P99 延迟 < 5ms | `compare` 模式测量代理开销 | streaming + chat |
| NFR-03: 滑动窗口限速精度符合预期 | 边界压测与超出限速的 429 拦截率统计 | sustained（加限速配置） |
| NFR-05: 内存 < 50MB | 运行时 RSS 监控 | sustained（长时间压测） |
| NFR-07: 启动 < 2s | 时间戳记录 | burst（冷启动后立即压测） |
| NFR-09: 流式保活与 Telemetry 开销极低 | 监控 Stream End Delay 与保活过滤准确性 | streaming / mixed |

---

## 12 CI 集成与回归测试规范

为了防止 ModelSwitch 在后续重构与功能迭代中产生性能退化（Performance Regression），可将 `llm-gateway-bench` 部署在 CI 流程中，并执行自动化阈值断言。

### 12.1 运行工作流

每次提 PR 时，CI 运行以下步骤：
1. **构建与初始化**：编译 ModelSwitch 后端及 `gateway-bench` 可执行文件。
2. **拉起测试环境**：
   - 启动 `gateway-bench --mock-upstream --mock-port 0` 得到随机端口。
   - 配置并拉起 ModelSwitch 后端（将上游代理地址指向 Mock Port）。
3. **性能执行**：
   - 运行对比模式测试代理开销：`gateway-bench compare --direct http://127.0.0.1:<MOCK_PORT> --proxy http://127.0.0.1:<PROXY_PORT> --scenario mixed --concurrency 50 --duration 20s --report json:regression.json`。
4. **指标断言（Threshold Verification）**：
   - 使用辅助脚本解析 `regression.json` 并应用断言规则。

### 12.2 退化判定断言规则（示例）

| 判定指标 | 失败判定阈值（CI 红线） | 理由 |
|---------|-----------------------|-----|
| **代理开销 P99 (Overhead P99)** | `> 5ms` | 网关核心路由与限流应保持极低延迟阻碍 |
| **首字节延迟差值 (TTFB Overhead)** | `> 3ms` | 评估流式首 chunk 转发开销 |
| **吞吐量降低百分比 (RPS Loss)** | `> 5%` | 代理吞吐能力折损不应过大 |
| **错误率增幅 (Error Rate Increase)** | `> 0%` | 网关在 50 并发下不应主动引入任何转发错误 |

```json
// 判定规则配置示例 (regression-rules.json)
{
  "max_p99_overhead_ms": 5.0,
  "max_ttfb_overhead_ms": 3.0,
  "max_rps_loss_pct": 5.0,
  "allowed_error_rate": 0.0
}
```

---

*文档结束。*
