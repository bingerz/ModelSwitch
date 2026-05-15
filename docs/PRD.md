# ModelSwitch — 产品需求文档 (PRD)

> **版本**: v1.1 · **日期**: 2026-05-11 · **状态**: Draft

---

## 1 项目概述

### 1.1 背景与痛点

开发者重度使用 Claude Code / Codex CLI / Gemini CLI / Cursor 等 AI 编程工具时，通常同时开通多家 LLM 厂商的高级套餐（Claude Pro、ChatGPT Plus、智谱 Coding Plan、百炼 Token Plan、MiniMax Token Plan、Kimi、DeepSeek 等）或配置多路按量付费 API。

**核心痛点**：在高并发编码、全库索引、长上下文对话场景下，频繁触发 **Rate Limit / 429** 报错，迫使用户手动切换模型与 Key，严重破坏**开发心流（Flow）**。

### 1.2 产品定位

ModelSwitch 是一款 **Rust 驱动的轻量级本地 LLM 智能网关与额度/成本调度桌面软件**，定位为 **"开发者的 AI 额度管家"**——充当 IDE 与远端 LLM 厂商间的智能负载均衡器：

- **智能账本与阶梯降级策略**：按成本阶梯自动调度——优先消耗免费/包月额度，额度耗尽后自动降级到廉价 API，最后才动用高价官方 Key
- **主动配额轮询** + **被动异常熔断** 双轨机制
- 当前模型限流时，按用户自定义优先级 **无感热切换** 至备用模型
- **开箱即用**：优雅的图形化 UI，拖拽式优先级卡片配置，系统托盘常驻——面向全量程序员，零命令行门槛
- **Code Non-Stop** — 保障程序员无间断编码

### 1.3 竞品分析与差异化定位

#### 1.3.1 竞品概览

| 项目 | 形态 | 核心能力 | 局限性 |
|------|------|----------|--------|
| **CLIProxyAPI** | CLI + YAML 配置 | 多协议桥接网关，逆向 OAuth 提取，多账号轮询 | 偏极客向，需命令行 + Redis/PG，无图形界面 |
| **ZeroLimit** | Tauri + React 桌面端 | 配额监控，可视化面板 | 偏监控展示，调度能力有限 |
| **Quotio** | macOS 菜单栏应用 | 配额追踪 + 自动 failover | 仅 macOS，功能较单一 |
| **vibeproxy** | 代理网关 | 请求转发 + 简单轮询 | 无成本感知调度 |

#### 1.3.2 ModelSwitch 的差异化定位

| 维度 | 同类网关产品（如 CLIProxyAPI） | **ModelSwitch** |
|------|-------------------------------|------------------|
| **产品定位** | 多协议桥接网关（偏底层协议转换） | **额度/成本管理器**（偏策略控制与成本平衡） |
| **核心卖点** | 逆向 OAuth、协议适配 | **智能账本 + 阶梯降级 + 成本可视化** |
| **使用门槛** | 命令行 + YAML + 可能需 Redis/PG | **一键安装桌面软件**，图形化拖拽配置 |
| **目标用户** | 重度极客 / DevOps | **全量程序员**（包括不想折腾 CLI 的开发者） |
| **调度策略** | 轮询 + Fallback | **成本感知阶梯降级** + 轮询 + 熔断 |

> 💡 **生态策略**：自主可控，核心能力自研。通过分析 CLIProxyAPI 等优秀项目的核心实现，提取多协议桥接、OAuth 逆向提取、多账号轮询等关键技术要点，在 ModelSwitch 中用 Rust 实现完整的协议转换层。ModelSwitch 同时聚焦**策略调度、成本控制和极致 DX**三大核心价值，构建完全自主可控的技术栈。

### 1.4 核心参考项目

| 项目 | 借鉴要点 |
|------|----------|
| **CLIProxyAPI** | 参考学习：多协议桥接实现、OAuth 逆向提取方案、多账号轮询机制 |
| **php-lsys/token-monitor** | Tauri WebView 注入 JS、Provider 模式、Cookie 拦截、网页套餐用量抓取 |
| **tensorzero/tensorzero** | 纯 Rust 异步网关、P99 < 1ms、高性能流式转发 |
| **QuantumNous/new-api** | Channel 抽象、加权轮询、优先级降级、自动轮换路由算法 |
| **MrFadiAi/free-llm-gateway** | 多渠道故障自愈、429 状态机、熔断降级策略 |
| **ZeroLimit / Quotio** | Tauri 桌面端形态、配额可视化、菜单栏常驻交互 |

---

## 2 系统架构

### 2.1 技术栈

| 层级 | 技术选型 | 理由 |
|------|----------|------|
| 应用外壳 | **Tauri 2.0** (Rust + TS + React) | 轻量跨平台、内存低、支持 MenuBar/Tray 常驻 |
| 本地代理 | **Axum + Tokio** | 零 GIL、高性能异步反向代理 |
| HTTP 客户端 | **reqwest** | 支持 Cookie、流式、连接池复用 |
| 安全存储 | **keyring-rs** | 系统级硬件钱包 (macOS Keychain / Win Credential Manager) |
| 前端 UI | **React + TypeScript** | 生态丰富、Tauri 官方推荐 |

### 2.2 运行拓扑

```
IDE (Cursor/Cline/Claude Code)
    │  HTTP Request (OpenAI/Anthropic 格式)
    ▼
┌──────────────────────────────────┐
│  ModelSwitch 本地网关             │
│  http://127.0.0.1:8080           │
│                                  │
│  ┌────────────┐  ┌────────────┐  │
│  │ 路由引擎    │  │ 熔断状态机  │  │
│  │ (Tier路由)  │  │ (429检测)  │  │
│  └──────┬─────┘  └──────┬─────┘  │
│         └───────┬───────┘        │
│                 ▼                │
│  ┌──────────────────────────┐    │
│  │  渠道管理器               │    │
│  │  Tier1: 免费/包月套餐     │    │
│  │  Tier2: 廉价 API (如DS)   │    │
│  │  Tier3: 官方高价 Key 兜底  │    │
│  └──────────────────────────┘    │
└──────────────┬───────────────────┘
               │  流式转发 (SSE)
               ▼
     LLM Provider (Claude/GPT/DeepSeek/...)
```

### 2.3 核心流程（请求生命周期）

```
1. IDE → POST /v1/chat/completions → ModelSwitch (127.0.0.1:8080)
2. 路由引擎选择 Tier 最高 + 权重最大的健康渠道
3. reqwest 携带凭证转发至上游 LLM
4. 流式 SSE 逐 chunk 透传回 IDE
5. 若上游返回 429 / 超时 / 5xx:
   a. 标记当前渠道为"熔断"（锁定 N 分钟）
   b. 不断开 IDE 连接
   c. 立即选择下一健康渠道重发
   d. IDE 端完全无感知
6. 若所有渠道均熔断 → 返回自定义 429 友好提示
```

---

## 3 功能需求 (FR)

### FR-01 本地反向代理网关

| ID | 需求 | 优先级 | 验收标准 |
|----|------|--------|----------|
| FR-01-01 | 兼容 OpenAI `/v1/chat/completions` 格式 | P0 | 接收标准 OpenAI 请求并正确转发 |
| FR-01-02 | 兼容 Anthropic `/v1/messages` 格式 | P0 | 接收标准 Anthropic 请求并正确转发 |
| FR-01-03 | 零改动接入：Base URL 改为本地地址，API Key 填任意字符 | P0 | Cursor/Cline 无需额外配置即可使用 |
| FR-01-04 | 完美支持 SSE 流式传输 | P0 | 打字机效果零延迟，不留存缓存 |
| FR-01-05 | 请求/响应 Header 透传 | P1 | 保留必要 Header (Content-Type, Authorization 等) |
| FR-01-06 | 可配置监听端口 | P1 | 默认 8080，支持用户自定义 |

### FR-02 渠道与梯度优先级管理

| ID | 需求 | 优先级 | 验收标准 |
|----|------|--------|----------|
| FR-02-01 | 混合凭证托管：API Key + Web Session Cookie | P0 | 统一数据结构管理两种凭证类型 |
| FR-02-02 | 三级成本阶梯路由 (Tier 1/2/3) | P0 | Tier1(免费/包月)限流后自动切 Tier2(廉价 API)，Tier2 限流切 Tier3(官方高价 Key) |
| FR-02-03 | 同级加权轮询 | P0 | 同一 Tier 内多渠道按权重分配请求 |
| FR-02-04 | 渠道 CRUD（增删改查） | P0 | UI 支持添加、编辑、删除、排序渠道 |
| FR-02-05 | 一键连接测试 (Ping) | P1 | 点击后发送测试请求验证渠道可用性 |
| FR-02-06 | 渠道启用/禁用开关 | P1 | 手动暂停某渠道而不删除配置 |

### FR-03 额度与限流双轨感知

| ID | 需求 | 优先级 | 验收标准 |
|----|------|--------|----------|
| FR-03-01 | **被动熔断**：捕获 429 / 超时 / 5xx 自动熔断 | P0 | 渠道被标记锁定，锁定期内不再分配请求 |
| FR-03-02 | **热重试**：同一请求内无感切换备用渠道 | P0 | IDE 连接不断开，用户无察觉 |
| FR-03-03 | 熔断锁定时长可配置 | P0 | 默认 30 分钟，支持读取 `retry-after` Header |
| FR-03-04 | 熔断自动恢复 | P0 | 锁定期满后渠道自动恢复为"可用" |
| FR-03-05 | 主动轮询 API 余额 | P1 | 定时请求厂商 billing 接口刷新余额 |
| FR-03-06 | 主动轮询 Web 配额 | P2 | 利用 Cookie 请求官网内网接口抓取配额 |

### FR-04 桌面端管理 UI

| ID | 需求 | 优先级 | 验收标准 |
|----|------|--------|----------|
| FR-04-01 | 系统托盘常驻 | P0 | 显示当前工作模型 + 健康通道数 |
| FR-04-02 | 渠道配置面板 | P0 | 可视化管理所有渠道及其优先级 |
| FR-04-03 | **拖拽式优先级卡片** | P0 | 用户通过拖拽卡片调整 Tier 分组与渠道顺序，无需编辑 YAML |
| FR-04-04 | 实时调度日志 | P0 | 展示切换事件（时间、原因、目标渠道、延迟） |
| FR-04-05 | 渠道健康状态看板 | P1 | 每个渠道的当前状态（健康/熔断/禁用） |
| FR-04-06 | **成本统计仪表盘** | P1 | 展示各 Tier 用量分布、估算节省金额、额度消耗趋势 |
| FR-04-07 | WebView 登录引导 | P2 | 内嵌 WebView 引导用户登录 Claude.ai 等获取 Cookie |

---

## 4 非功能需求 (NFR)

| ID | 需求 | 指标 | 验收标准 |
|----|------|------|----------|
| NFR-01 | 极致性能 | P99 延迟 < 5ms | 本地代理层不拖慢 AI 插件响应 |
| NFR-02 | 隐私安全 | 零云端依赖 | 所有凭证本地加密存储，无外部数据上报 |
| NFR-03 | 凭证安全 | 系统级存储 | API Key / Cookie 存入 Keychain / Credential Manager |
| NFR-04 | 优雅降级 | 全渠道熔断提示 | 返回中文友好 429："ModelSwitch: 所有渠道均已限流，请稍作休息。" |
| NFR-05 | 资源占用 | 内存 < 50MB | Tauri 轻量运行时 + Rust 低开销 |
| NFR-06 | 跨平台 | macOS / Windows / Linux | Tauri 2.0 原生支持三平台 |
| NFR-07 | 启动速度 | < 2 秒 | 应用启动到网关可用 < 2 秒 |

---

## 5 数据模型设计

### 5.1 渠道 (Channel)

```rust
struct Channel {
    id: Uuid,
    name: String,              // 用户自定义名称，如 "Claude_Pro_1"
    provider: Provider,        // 枚举: OpenAI, Anthropic, DeepSeek, ...
    tier: u8,                  // 1=免费/包月优先, 2=廉价平替, 3=官方高价兜底
    weight: u32,               // 同级权重 (默认 100)
    cost_per_token: Option<f64>, // 每 token 成本 (用于成本统计与可视化)
    credential: Credential,    // API Key 或 Web Session
    enabled: bool,             // 启用/禁用
    status: ChannelStatus,     // Healthy / CircuitOpen / Disabled
    circuit_open_until: Option<DateTime<Utc>>,  // 熔断恢复时间
    base_url: String,          // 上游 API 地址
    model_mapping: HashMap<String, String>,     // 模型名映射
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

### 5.2 凭证 (Credential)

```rust
enum Credential {
    ApiKey {
        key: String,            // 存储引用，实际值在 keyring
    },
    WebSession {
        cookie: String,         // 存储引用，实际值在 keyring
        expires_at: Option<DateTime<Utc>>,
    },
}
```

### 5.3 调度日志 (DispatchLog)

```rust
struct DispatchLog {
    id: Uuid,
    timestamp: DateTime<Utc>,
    request_model: String,      // 请求的模型名
    channel_id: Uuid,           // 最终使用的渠道
    channel_name: String,
    retry_count: u8,            // 重试次数
    trigger_reason: Option<String>,  // "429", "timeout", "5xx"
    latency_ms: u64,            // 额外延迟
    success: bool,
    estimated_cost: Option<f64>,    // 估算成本 (基于 cost_per_token)
}
```

---

## 6 API 设计

### 6.1 代理端点（面向 IDE）

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/v1/chat/completions` | OpenAI 兼容代理 |
| POST | `/v1/messages` | Anthropic 兼容代理 |
| GET  | `/health` | 网关健康检查 |

### 6.2 管理端点（面向前端 UI）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/channels` | 获取所有渠道 |
| POST | `/api/channels` | 创建渠道 |
| PUT | `/api/channels/:id` | 更新渠道 |
| DELETE | `/api/channels/:id` | 删除渠道 |
| POST | `/api/channels/:id/ping` | 测试渠道连通性 |
| GET | `/api/channels/:id/status` | 获取渠道状态 |
| GET | `/api/logs` | 获取调度日志（分页） |
| GET | `/api/stats` | 获取统计摘要 |
| GET | `/api/stats/cost` | 获取成本统计（各 Tier 用量、估算节省、趋势） |

---

## 7 路由算法详细设计

### 7.1 渠道选择算法

```
function selectChannel(request):
    // 1. 按 Tier 分组，从低到高 (Tier1=免费/包月 → Tier2=廉价 → Tier3=高价)
    //    优先消耗低成本渠道，自动阶梯降级
    for tier in [1, 2, 3]:
        candidates = channels.filter(
            tier == tier AND enabled AND status == Healthy
        )
        if candidates.isEmpty: continue

        // 2. 同级加权随机选择
        selected = weightedRandom(candidates)
        return selected

    // 3. 所有渠道均不可用
    return Error("所有渠道均已限流")
```

### 7.2 熔断状态机

```
         ┌──────────┐
         │  Healthy  │ ◄─── 锁定期满自动恢复
         └────┬─────┘
              │ 收到 429 / timeout / 5xx
              ▼
         ┌──────────────┐
         │ CircuitOpen  │ ─── 锁定 N 分钟 (默认30)
         │ (熔断锁定)    │     或遵循 retry-after
         └──────────────┘
```

### 7.3 热重试流程

```
function handleRequest(request):
    maxRetries = 3
    for attempt in 0..maxRetries:
        channel = selectChannel(request)
        if channel is Error: return 429 + 友好提示

        response = forwardToUpstream(channel, request)

        if response.status == 429 or timeout:
            markCircuitOpen(channel)
            log("渠道 {channel.name} 熔断，切换中...")
            continue  // 不断开 IDE 连接，立即重试

        return response  // 成功，流式透传

    return 429 + "所有备用渠道均已耗尽"
```

---

## 8 MVP 里程碑路线图

### 里程碑 1：网关层通畅 (预计 1-2 周)

**目标**：搭建可运行的本地代理，单渠道流式转发成功。

| 任务 | 说明 | 产出物 |
|------|------|--------|
| T1-01 | 初始化 Tauri 2.0 + Axum 项目骨架 | 可编译运行的项目 |
| T1-02 | 实现 `/v1/chat/completions` 代理端点 | OpenAI 格式请求转发 |
| T1-03 | 实现 SSE 流式透传 | 逐 chunk 转发，打字机效果 |
| T1-04 | 实现 `/v1/messages` 代理端点 | Anthropic 格式请求转发 |
| T1-05 | 基础配置文件 (TOML) 加载 | 读取渠道配置 |
| T1-06 | 集成测试：Cursor → ModelSwitch → Claude API | 端到端验证 |

**验收标准**：Cursor 将 Base URL 改为 `http://127.0.0.1:8080`，可正常使用 AI 编码，流式输出无卡顿。

### 里程碑 2：热重试机制 (预计 1-2 周)

**目标**：多渠道配置 + 429 自动熔断与无感切换。

| 任务 | 说明 | 产出物 |
|------|------|--------|
| T2-01 | 实现 Channel 数据结构与管理 | 多渠道 CRUD |
| T2-02 | 实现 Tier 路由 + 加权轮询算法 | 路由引擎 |
| T2-03 | 实现熔断状态机 | 429 检测 + 锁定 + 自动恢复 |
| T2-04 | 实现热重试（同请求内无感切换） | 热重试中间件 |
| T2-05 | 实现凭证安全存储 (keyring-rs) | Key 加密存储 |
| T2-06 | 模拟 429 集成测试 | 验证无感切换 |

**验收标准**：配置两个渠道，模拟第一个返回 429，IDE 连接不断开，自动切换至第二个渠道成功输出。

### 里程碑 3：Web Session + 桌面 UI (预计 2-3 周)

**目标**：混合调度链路 + 可视化管理界面。

| 任务 | 说明 | 产出物 |
|------|------|--------|
| T3-01 | Tauri WebView 登录引导 | 内嵌浏览器登录 Claude.ai |
| T3-02 | Cookie 拦截与持久化 | 提取并安全存储 Session |
| T3-03 | Web Session 渠道适配器 | Cookie → 请求转发 |
| T3-04 | 系统托盘 + 状态显示 | Tray 图标 + 菜单 |
| T3-05 | 渠道管理 UI | React 配置面板 |
| T3-06 | 调度日志看板 | 实时日志展示 |

**验收标准**：通过 WebView 登录 Claude 网页端，Cookie 被提取，Web Session 渠道与 API 渠道混合调度正常工作。

---

## 9 项目目录结构（建议）

```
ModelSwitch/
├── docs/                     # 文档
│   └── PRD.md               # 本文档
├── src-tauri/                # Rust 后端 (Tauri + Axum)
│   ├── src/
│   │   ├── main.rs          # 入口：启动 Tauri + Axum
│   │   ├── proxy/           # 代理层
│   │   │   ├── mod.rs
│   │   │   ├── openai.rs    # OpenAI 格式处理
│   │   │   ├── anthropic.rs # Anthropic 格式处理
│   │   │   └── stream.rs    # SSE 流式转发
│   │   ├── router/          # 路由引擎
│   │   │   ├── mod.rs
│   │   │   ├── tier.rs      # Tier 梯度路由
│   │   │   ├── weighted.rs  # 加权轮询
│   │   │   └── circuit.rs   # 熔断状态机
│   │   ├── channel/         # 渠道管理
│   │   │   ├── mod.rs
│   │   │   ├── manager.rs   # 渠道 CRUD
│   │   │   └── credential.rs # 凭证管理
│   │   ├── quota/           # 额度感知
│   │   │   ├── mod.rs
│   │   │   ├── poller.rs    # 主动轮询
│   │   │   └── web_session.rs # Web Session
│   │   ├── config.rs        # 配置加载
│   │   └── log.rs           # 调度日志
│   └── Cargo.toml
├── src/                      # 前端 (React + TS)
│   ├── App.tsx
│   ├── components/
│   │   ├── ChannelPanel.tsx  # 渠道配置面板
│   │   ├── LogViewer.tsx     # 日志看板
│   │   ├── StatusBar.tsx     # 状态栏
│   │   └── TrayMenu.tsx      # 托盘菜单
│   └── main.tsx
├── package.json
└── tauri.conf.json
```

---

## 10 风险与应对

| 风险 | 影响 | 概率 | 应对策略 |
|------|------|------|----------|
| Web Session Cookie 频繁过期 | Web 渠道不可用 | 高 | 定时检测有效性 + 提醒用户重新登录 |
| SSE 热重试时部分 chunk 已发送 | 响应内容重复 | 中 | 仅在首个 chunk 发送前重试；已开始流式则降级为新请求 |
| 厂商修改 API 格式 | 代理转发失败 | 低 | 抽象 Provider 适配层，快速适配 |
| 所有渠道同时限流 | 编码中断 | 低 | 优雅降级提示 + 建议用户扩充渠道池 |
| Tauri WebView 跨平台兼容性 | 登录流程异常 | 中 | 分平台测试 + fallback 手动粘贴 Cookie |

---

## 11 核心 Rust 依赖清单

```toml
[dependencies]
# 应用框架
tauri = { version = "2", features = ["tray-icon", "devtools"] }

# 异步运行时 & Web 框架
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["ws"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }

# HTTP 客户端
reqwest = { version = "0.12", features = ["stream", "cookies", "json"] }

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# 安全存储
keyring = "3"

# 工具
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
```

---

## 12 术语表

| 术语 | 定义 |
|------|------|
| **Channel (渠道)** | 一个可用于转发请求的 LLM 接入点，包含凭证和配置 |
| **Tier (梯度)** | 渠道优先级层级，数字越小优先级越高 |
| **Circuit Breaker (熔断器)** | 检测故障并临时隔离问题渠道的机制 |
| **Hot Retry (热重试)** | 在同一 HTTP 请求内透明切换渠道重发的机制 |
| **Web Session** | 通过浏览器 Cookie 维持的网页端登录会话 |
| **SSE** | Server-Sent Events，服务端推送流式响应的协议 |

---

*文档结束。如有疑问请联系项目负责人。*
