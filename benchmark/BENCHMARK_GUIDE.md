# ModelSwitch 中转性能测试指南

本指南介绍如何使用 `gateway-bench` 工具测试 ModelSwitch 网关的中转性能。

---

## 目录

1. [准备工作](#1-准备工作)
2. [快速开始：使用内置 Mock 测试](#2-快速开始使用内置-mock-测试)
3. [测试真实 ModelSwitch 网关](#3-测试真实-modelswitch-网关)
4. [对比模式：测量代理开销](#4-对比模式测量代理开销)
5. [测试场景说明](#5-测试场景说明)
6. [输出指标解读](#6-输出指标解读)
7. [高级用法](#7-高级用法)
8. [实战示例合集](#8-实战示例合集)

---

## 1. 准备工作

### 1.1 编译 Benchmark 工具

```bash
cd ModelSwitch/benchmark
cargo build --release
```

编译后的二进制文件在 `target/release/gateway-bench`，后续可直接使用：
```bash
alias bench="./target/release/gateway-bench"
```

也可以通过 `cargo run --` 直接运行（开发模式）。

### 1.2 编译 ModelSwitch CLI（可选，用于无 UI 模式）

```bash
cd ModelSwitch/src-tauri
cargo build --bin modelswitch-cli --release
```

编译后的 CLI 在 `target/release/modelswitch-cli`。

### 1.3 验证安装

```bash
# Benchmark 工具帮助
gateway-bench --help

# ModelSwitch CLI 帮助
modelswitch-cli --help
```

---

## 2. 快速开始：使用内置 Mock 测试

**无需任何 API Key**，benchmark 工具内置了 Mock LLM 服务器，适合快速验证工具功能和测试网关中转性能。

### 2.1 基础非流式测试

```bash
# 启动 Mock 服务器 (延迟 50ms) 并测试 chat 场景
gateway-bench \
  --mock-upstream \
  --mock-delay 50 \
  --scenario chat \
  --concurrency 10 \
  --duration 30 \
  --warmup 5
```

### 2.2 流式测试（TTFB + Chunk 统计）

```bash
gateway-bench \
  --mock-upstream \
  --mock-delay 30 \
  --scenario streaming \
  --concurrency 20 \
  --duration 30 \
  --warmup 5 \
  --mock-stream-chunks 20
```

### 2.3 突发并发测试

```bash
gateway-bench \
  --mock-upstream \
  --mock-delay 50 \
  --scenario burst \
  --burst-size 500
```

---

## 3. 测试真实 ModelSwitch 网关

### 3.1 启动 ModelSwitch 网关

**方式一：Tauri 桌面应用**
```bash
# 正常启动桌面应用，网关会自动在 127.0.0.1:8080 启动
cd ModelSwitch/src-tauri
cargo tauri dev
```

**方式二：CLI 无头模式**
```bash
# 初始化配置（首次使用）
modelswitch-cli init

# 验证配置
modelswitch-cli validate

# 查看已配置的渠道
modelswitch-cli channels

# 启动网关
modelswitch-cli serve --port 8080
```

### 3.2 配置文件位置

配置文件路径：`~/.config/modelswitch/config.json`（macOS 下为 `~/Library/Application Support/modelswitch/config.json`）

一个最小配置示例：
```json
{
  "gateway": {
    "port": 8080,
    "host": "127.0.0.1",
    "routing_strategy": "weighted_random"
  },
  "channels": [
    {
      "id": "ch1",
      "name": "OpenAI直连",
      "provider": "openai",
      "base_url": "https://api.openai.com",
      "api_key": "sk-your-key-here",
      "credential_type": "api_key",
      "credential_ref": "openai-key",
      "enabled": true,
      "priority": 1,
      "weight": 100
    }
  ]
}
```

### 3.3 对网关发起压测

```bash
# 基础测试
gateway-bench \
  --target http://127.0.0.1:8080 \
  --api-key your-gateway-key \
  --model gpt-4 \
  --scenario chat \
  --concurrency 50 \
  --duration 60 \
  --warmup 10

# 流式测试（测试 SSE 中转质量）
gateway-bench \
  --target http://127.0.0.1:8080 \
  --api-key your-gateway-key \
  --model gpt-4 \
  --scenario streaming \
  --concurrency 30 \
  --duration 60 \
  --warmup 10

# 混合负载（70% 流式 + 30% 非流式）
gateway-bench \
  --target http://127.0.0.1:8080 \
  --api-key your-gateway-key \
  --model gpt-4 \
  --scenario mixed \
  --mix-ratio 70 \
  --concurrency 50 \
  --duration 60 \
  --warmup 10
```

---

## 4. 对比模式：测量代理开销

这是最重要的测试——**测量 ModelSwitch 网关本身引入了多少额外延迟**。

### 4.1 原理

```
直连测试:    Client ──► LLM API          测量基线延迟
代理测试:    Client ──► ModelSwitch ──► LLM API    测量代理后延迟
代理开销 = 代理延迟 − 直连延迟
```

### 4.2 使用 Mock 上游进行对比（无需 API Key）

```bash
# 第一步：单独启动一个 Mock LLM 服务器
gateway-bench \
  --mock-upstream \
  --mock-port 19876 \
  --mock-delay 100 \
  --scenario chat \
  --duration 1 &

# 第二步：将 ModelSwitch 的上游指向 Mock 服务器
# 在 ModelSwitch 配置中设置 channel.base_url = "http://127.0.0.1:19876"

# 第三步：执行对比
gateway-bench compare \
  --direct http://127.0.0.1:19876 \
  --proxy http://127.0.0.1:8080 \
  --scenario streaming \
  --concurrency 50 \
  --duration 30
```

### 4.3 使用真实 LLM API 进行对比

```bash
gateway-bench compare \
  --direct https://api.openai.com \
  --proxy http://127.0.0.1:8080 \
  --api-key sk-your-openai-key \
  --model gpt-4 \
  --scenario streaming \
  --concurrency 30 \
  --duration 60 \
  --warmup 10 \
  --report overhead_report.md
```

### 4.4 对比报告解读

```
| Metric          | Direct     | Via Proxy  | Overhead   |
|-----------------|------------|------------|-----------|
| P50 Latency     | 142ms      | 146ms      | +4ms      |  ← 网关给 50% 的请求增加了 4ms
| P95 Latency     | 310ms      | 318ms      | +8ms      |  ← 95% 的请求增加了 8ms
| P99 Latency     | 520ms      | 535ms      | +15ms     |  ← 尾部延迟增加 15ms
| TTFB P50        | 85ms       | 89ms       | +4ms      |  ← 流式首字节延迟增加 4ms
| RPS             | 486        | 472        | -2.9%     |  ← 吞吐量损失 2.9%
| Error Rate      | 0.0%       | 0.0%       | —         |  ← 网关未引入额外错误
```

**理想目标**：
- P99 开销 < 5ms
- TTFB 开销 < 3ms
- RPS 损失 < 5%
- 错误率增量 = 0%

---

## 5. 测试场景说明

| 场景 | 模拟负载 | 核心指标 | 适用场景 |
|------|---------|---------|---------|
| **chat** | 非流式请求 | P50/P95/P99、RPS、错误率 | 基础代理转发性能 |
| **streaming** | SSE 流式请求 | TTFB、Chunk 间隔、总完成时间 | 流式透传质量 |
| **mixed** | 流式 + 非流式混合 | 综合延迟分布 + 流式指标 | 真实混合负载 |
| **burst** | 瞬间 N 个并发 | 峰值延迟、突发 RPS | 突发流量承受力 |
| **sustained** | 恒定 RPS 持续压测 | 稳态延迟、丢弃请求数 | 长时间稳定性 |

### 场景选择建议

- **日常验证** → `chat` 或 `streaming`
- **发布前回归** → `mixed` + `compare` 模式
- **压力测试** → `burst`（瞬时高并发）
- **稳定性测试** → `sustained`（恒定 RPS 长时间）
- **性能调优** → `compare` 模式 + `--runs 5`（多次运行取平均）

---

## 6. 输出指标解读

### 6.1 延迟指标

| 指标 | 含义 |
|------|------|
| **P50** | 中位数延迟，50% 的请求快于此值 |
| **P95** | 95% 的请求快于此值（关注尾部延迟） |
| **P99** | 99% 的请求快于此值（最差体验） |
| **Mean** | 平均延迟 |
| **Min / Max** | 最快 / 最慢单次请求 |

### 6.2 吞吐量指标

| 指标 | 含义 |
|------|------|
| **RPS** | 每秒成功请求数（= 成功请求数 / 有效测量时间） |
| **Total Requests** | 总请求数（含预热期） |
| **Error Rate** | 失败率（非 2xx 响应 + 网络错误） |

### 6.3 流式指标

| 指标 | 含义 |
|------|------|
| **TTFB** | Time To First Byte — 从请求发出到第一个 SSE 数据块的时间 |
| **Chunk Interval** | 相邻 SSE 数据块之间的时间间隔 |
| **Avg Chunks/Request** | 每个请求平均接收到的数据块数 |

### 6.4 错误分类

| 类别 | 说明 |
|------|------|
| **4xx Errors** | 客户端错误（含 429 限流） |
| **5xx Errors** | 服务器错误 |
| **Timeouts** | 请求超时 |
| **Network Errors** | 连接失败、TCP 重置、TLS 错误 |

---

## 7. 高级用法

### 7.1 多次运行 + 方差分析

```bash
gateway-bench \
  --target http://127.0.0.1:8080 \
  --scenario chat \
  --duration 30 \
  --warmup 5 \
  --concurrency 50 \
  --runs 5
```

输出 P95 的 mean ± stddev，用于判断结果稳定性。

### 7.2 限速测试（恒定 RPS）

```bash
gateway-bench \
  --target http://127.0.0.1:8080 \
  --scenario sustained \
  --rps 100 \
  --duration 120 \
  --warmup 10
```

### 7.3 Anthropic 协议测试

```bash
gateway-bench \
  --target http://127.0.0.1:8080 \
  --protocol anthropic \
  --scenario chat \
  --duration 30
```

### 7.4 自定义消息大小

```bash
# 大消息（100 词的请求体）
gateway-bench \
  --target http://127.0.0.1:8080 \
  --message-tokens 100 \
  --mock-tokens 500 \
  --scenario chat \
  --duration 30
```

### 7.5 模拟错误场景

```bash
# 模拟上游 30% 返回 429 限流
gateway-bench \
  --mock-upstream \
  --mock-fail-rate 30 \
  --scenario chat \
  --duration 30

# 测试网关在 429 压力下的熔断/重试行为
gateway-bench \
  --target http://127.0.0.1:8080 \
  --scenario sustained \
  --rps 50 \
  --duration 60
```

### 7.6 生成报告文件

```bash
# 同时输出 JSON 和 Markdown
gateway-bench \
  --target http://127.0.0.1:8080 \
  --scenario mixed \
  --duration 60 \
  --report json:results.json,markdown:results.md
```

### 7.7 TLS / 自签名证书

```bash
# 测试 HTTPS 网关（跳过证书验证）
gateway-bench \
  --target https://gateway.internal:8443 \
  --tls-skip-verify \
  --scenario chat \
  --duration 30
```

---

## 8. 实战示例合集

### 8.1 完整的发布前回归测试

```bash
# 1. 启动 ModelSwitch（指向 Mock 上游）
modelswitch-cli serve --port 8080 &
sleep 3

# 2. 启动 Mock LLM 上游
gateway-bench --mock-upstream --mock-port 19876 --mock-delay 100 --scenario chat --duration 1 &
sleep 1

# 3. 多场景回归
echo "=== Chat 场景 ==="
gateway-bench --target http://127.0.0.1:8080 --scenario chat \
  --concurrency 50 --duration 30 --warmup 5

echo "=== Streaming 场景 ==="
gateway-bench --target http://127.0.0.1:8080 --scenario streaming \
  --concurrency 30 --duration 30 --warmup 5

echo "=== Mixed 场景 ==="
gateway-bench --target http://127.0.0.1:8080 --scenario mixed \
  --concurrency 50 --duration 30 --warmup 5

echo "=== Burst 场景 ==="
gateway-bench --target http://127.0.0.1:8080 --scenario burst \
  --burst-size 300

# 4. 对比测试（5 次运行取平均）
echo "=== Overhead 对比 ==="
gateway-bench compare \
  --direct http://127.0.0.1:19876 \
  --proxy http://127.0.0.1:8080 \
  --scenario streaming \
  --concurrency 50 --duration 30 --warmup 5 \
  --report regression.md
```

### 8.2 CI 自动化退化检测

```bash
#!/bin/bash
# ci-benchmark.sh — CI 中的性能退化检测

THRESHOLD_P99_OVERHEAD_MS=5
THRESHOLD_TTFB_OVERHEAD_MS=3
THRESHOLD_RPS_LOSS_PCT=5

# 运行对比测试
gateway-bench compare \
  --direct http://127.0.0.1:19876 \
  --proxy http://127.0.0.1:8080 \
  --scenario mixed \
  --concurrency 50 \
  --duration 20 \
  --warmup 5 \
  --report ci-result.json

# 解析 JSON 并检查阈值（使用 jq）
P99_OVERHEAD=$(jq '.latency_p99_overhead_ms' ci-result.json)
TTFB_OVERHEAD=$(jq '.ttfb_p50_overhead_ms' ci-result.json)
RPS_LOSS=$(jq '.rps_loss_pct' ci-result.json)

echo "P99 overhead: ${P99_OVERHEAD}ms (threshold: ${THRESHOLD_P99_OVERHEAD_MS}ms)"
echo "TTFB overhead: ${TTFB_OVERHEAD}ms (threshold: ${THRESHOLD_TTFB_OVERHEAD_MS}ms)"
echo "RPS loss: ${RPS_LOSS}% (threshold: ${THRESHOLD_RPS_LOSS_PCT}%)"

PASS=true
[ "$P99_OVERHEAD" -gt "$THRESHOLD_P99_OVERHEAD_MS" ] && PASS=false && echo "❌ P99 overhead exceeds threshold"
[ "$TTFB_OVERHEAD" -gt "$THRESHOLD_TTFB_OVERHEAD_MS" ] && PASS=false && echo "❌ TTFB overhead exceeds threshold"
[ "$(echo "$RPS_LOSS > $THRESHOLD_RPS_LOSS_PCT" | bc)" -eq 1 ] && PASS=false && echo "❌ RPS loss exceeds threshold"

if [ "$PASS" = true ]; then
  echo "✅ All performance checks passed"
else
  echo "⛔ Performance regression detected"
  exit 1
fi
```

### 8.3 快速冒烟测试（10 秒）

```bash
# 最快的验证：3 秒 chat + 3 秒 streaming + 3 秒 burst
gateway-bench --mock-upstream --mock-delay 30 \
  --scenario chat --duration 3 --warmup 1 --concurrency 5

gateway-bench --mock-upstream --mock-delay 30 \
  --scenario streaming --duration 3 --warmup 1 --concurrency 5

gateway-bench --mock-upstream --mock-delay 30 \
  --scenario burst --burst-size 50
```

---

## 附录：全部 CLI 参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--target <URL>` | `""` | 目标网关地址（不使用 `--mock-upstream` 时必填） |
| `--scenario <NAME>` | `chat` | 测试场景：`chat`/`streaming`/`mixed`/`burst`/`sustained` |
| `--concurrency <N>` | `10` | 并发连接数（最小 1） |
| `--duration <SECS>` | `30` | 测试持续时间（秒） |
| `--warmup <SECS>` | `5` | 预热时间（不计入统计） |
| `--rps <N>` | 无限制 | 目标 RPS（限速模式，最小 1） |
| `--api-key <KEY>` | `dummy` | API Key |
| `--model <NAME>` | `gpt-4` | 模型名称 |
| `--mock-upstream` | `false` | 启动内置 Mock LLM 服务器 |
| `--mock-port <PORT>` | `0` | Mock 端口（0=随机） |
| `--mock-delay <MS>` | `200` | Mock 延迟（毫秒） |
| `--mock-fail-rate <PCT>` | `0` | Mock 429 失败率（%） |
| `--mock-tokens <N>` | `100` | Mock 返回 Token 数 |
| `--mock-stream-chunks <N>` | `0` | Mock 流式分块数（0=自动） |
| `--timeout <SECS>` | `120` | 请求超时（秒） |
| `--message-tokens <N>` | `10` | 请求消息长度 |
| `--burst-size <N>` | `200` | 突发并发数 |
| `--runs <N>` | `1` | 重复运行次数（方差分析） |
| `--mix-ratio <PCT>` | `70` | 混合场景流式比例（0-100） |
| `--stream` | `false` | 强制流式 |
| `--protocol <NAME>` | `openai` | 协议：`openai`/`anthropic` |
| `--pool-max-idle <N>` | `100` | 连接池最大空闲连接 |
| `--tls-skip-verify` | `false` | 跳过 TLS 验证 |
| `--report <FMT:PATH>` | 无 | 报告输出：`json:path` 或 `markdown:path` |
