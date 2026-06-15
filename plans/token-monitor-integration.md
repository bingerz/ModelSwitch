# Token Monitor 集成评估与实施计划

> **版本**: v2.1 · **日期**: 2026-05-13 · **范围**: 通用 Provider 配额监控架构（基于实际 API 调研）

## Context

ModelSwitch 作为本地 LLM 智能网关，当前 `quota/` 模块仅硬编码支持 OpenRouter 和 DeepSeek 两家 HTTP API 余额查询。项目 `presets.ts` 已包含 30+ 服务提供商（官方、国内、聚合商、云厂商、第三方中转），覆盖了市场上绝大多数 LLM 接入渠道。

**目标**：设计一套通用的配额监控架构，兼容所有 30+ Provider 的配额/余额/用量查询。

---

## 一、Provider 配额能力全景（基于网络调研）

### 1.1 有主动 Billing API 的 Provider

#### A 类：标准 Bearer Token API（直接可用）

| Provider | 端点 | 认证 | 响应关键字段 |
|----------|------|------|-------------|
| **OpenRouter** | `GET /api/v1/key` | `Bearer {KEY}` | `limit_remaining`, `limit`, `usage`, `usage_daily/weekly/monthly`, `is_free_tier` |
| **DeepSeek** | `GET /user/balance` | `Bearer {KEY}` | `balance_infos[].total_balance` (CNY) |
| **SiliconFlow** | `GET /v1/user/info` | `Bearer {KEY}` | `balance`, `chargeBalance`, `totalBalance` |
| **Moonshot/Kimi** | `GET /v1/users/me/balance` | `Bearer {KEY}` | `data.available_balance`, `voucher_balance`, `cash_balance` |
| **StepFun** | `GET /v1/accounts` | `Bearer {KEY}` | `balance`, `total_cash_balance`, `total_voucher_balance` |
| **Novita AI** | `GET /openapi/v1/user/balance` | `Bearer {KEY}` | 余额（单位 0.0001 USD） |
| **胜算云** | `GET /api/v1/key` | `Bearer {KEY}` | `data.max_quota`, `data.consumed_amount` |

#### D 类：OpenAI 兼容仪表盘（NewAPI/OneAPI 系）

适用于所有基于 [NewAPI](https://github.com/Calcium-Ion/new-api) 或 [OneAPI](https://github.com/songquanpeng/one-api) 搭建的中转站，**用普通 API Key 即可查询**（无需系统令牌）：

| 端点 | 响应 |
|------|------|
| `GET /dashboard/billing/subscription` | `{ "hard_limit_usd": 100.0, "access_until": 1735689600 }` |
| `GET /dashboard/billing/usage?start_date=YYYY-MM-DD&end_date=YYYY-MM-DD` | `{ "total_usage": 1250 }` (单位：美分) |

**覆盖 Provider**：AiHubMix, DMXAPI, Compshare, ModelScope, NewAPI 自建站，以及所有 NewAPI/OneAPI 系中转站。

#### C 类：云厂商 HMAC 签名 API（复杂，低优先级）

| Provider | 端点 | 认证方式 | 备注 |
|----------|------|---------|------|
| **百度千帆** | `POST billing.baidubce.com/v1/finance/cash/balance` | BCE AK/SK HMAC-SHA256 | 需 RAM 权限 |
| **阿里云百炼** | Aliyun BSS `Action=QueryAccountBalance` | AK/SK HMAC-SHA1 | 需 RAM `bss:DescribeAcccount` |
| **火山引擎/豆包** | `GET billing.volcengineapi.com/?Action=QueryBalanceAcct` | AK/SK HMAC-SHA256 | 频率限制 5 QPS |

> 这三家均需要 AccessKey/SecretKey 签名认证，与 ModelSwitch 当前 API Key 凭证模型不兼容。建议通过 WebView 抓取控制台页面来间接获取余额，或作为 Phase 5 独立实现。

#### Anthropic OAuth 端点（高价值）

| 端点 | 认证 | 响应 |
|------|------|------|
| `GET console.anthropic.com/api/oauth/usage` | Session Cookie / OAuth | `{ "five_hour": { "utilization": 27.0, "resets_at": "..." }, "seven_day": { "utilization": 4.0, "resets_at": "..." } }` |

> 这个端点返回 Claude Pro/Team 套餐的 5 小时和 7 天用量百分比，非常适合 WebView Cookie 抓取。

### 1.2 无主动 Billing API 的 Provider

| Provider | 现状 | 替代方案 |
|----------|------|---------|
| **Zhipu/GLM** | 无任何余额 API（已确认 `docs.bigmodel.cn` 400+ 端点均无） | 仅被动响应头提取 |
| **MiniMax** | 无余额 API（GitHub Issue [#6](https://github.com/MiniMax-AI/MiniMax-M2.5/issues/6) 请求中） | 仅被动响应头提取 |
| **NVIDIA NIM** | 无余额 API（论坛已确认痛点） | 仅被动响应头提取 |
| **PipeLLM** | 无余额端点 | 响应头含 `x-ratelimit-*`（RPM=余额×3） |
| **Google Gemini** | 无 billing API，无速率限制头 | 仅 `usageMetadata` 提取 + 429 状态码 |
| **AWS Bedrock** | 需 CloudWatch SDK | 复杂，低优先级 |
| **Azure OpenAI** | 需 Consumption API | 复杂，低优先级 |

### 1.3 被动速率限制响应头（零额外请求）

从代理转发的实际响应中自动提取：

| Provider | 响应头前缀 | 提供的维度 |
|----------|-----------|-----------|
| **OpenAI** | `x-ratelimit-*` (6 个头) | limit-requests, limit-tokens, remaining-requests, remaining-tokens, reset-requests, reset-tokens |
| **Anthropic** | `anthropic-ratelimit-*` (6 个头) | requests-limit, requests-remaining, tokens-limit, tokens-remaining, requests-reset, tokens-reset |
| **Azure OpenAI** | `x-ratelimit-*` | remaining-requests, remaining-tokens |
| **PipeLLM** | `x-ratelimit-*` | 动态 RPM（= 余额 × 3，范围 30-1800） |
| **OpenRouter** | 不返回速率限制头 | -- |
| **Google Gemini** | 不返回速率限制头 | 仅 429 `RESOURCE_EXHAUSTED` |

> **重要发现**：当前 `proxy/mod.rs` 的 `PASSTHROUGH_RESPONSE_HEADERS` 常量仅捕获 `x-ratelimit-*` 头，**完全遗漏了 `anthropic-ratelimit-*` 头**。需要修复。

---

## 二、配额获取策略分类（5 种）

| 策略 | 适用 Provider | 数量 | 实现 |
|------|-------------|------|------|
| **A: 专用 Billing API** | OpenRouter, DeepSeek, SiliconFlow, Moonshot, StepFun, Novita, 胜算云 | 7 | 专用 `QuotaProvider` 实现 |
| **D: NewAPI/OneAPI 兼容** | AiHubMix, DMXAPI, Compshare, ModelScope, 所有 NewAPI 系中转站 | ~15 | 通用 `openai_compat` poller |
| **E: 被动响应头提取** | OpenAI, Anthropic, Azure, PipeLLM, 全部 Custom | ~30 | `dispatch()` 中自动提取 |
| **B: WebView Cookie 抓取** | 阿里云百炼, 百度千帆(编码套餐), Anthropic(OAuth), ChatGPT | ~5 | feature-gated WebView |
| **C: 云厂商 HMAC 签名** | 百度, 阿里云, 火山引擎 | 3 | 延期（需 AK/SK） |

### 覆盖率估算

- **P0 交付后**（A + D + E）：7 家主动 API + 15 家 NewAPI 兼容 + 30 家被动提取 = **100% 基础覆盖**
- **P2 交付后**（+ B）：+5 家 WebView 深度监控（含套餐用量分时段）
- **P3 交付后**（+ C）：+3 家云厂商精确余额

---

## 三、架构设计

### 3.1 核心抽象 — `QuotaProvider` Trait

```rust
#[async_trait]
pub trait QuotaProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn poll(&self, ctx: &PollContext) -> QuotaResult;
    fn supports(&self, ctx: &PollContext) -> bool;
}

pub struct PollContext {
    pub channel_id: Uuid,
    pub channel_name: String,
    pub provider: String,
    pub base_url: String,
    pub credential: String,
    pub quota_config: Option<QuotaConfig>,  // TOML 中的自定义配置
    pub http_client: reqwest::Client,
}

pub type QuotaResult = Result<QuotaInfo, QuotaError>;

#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("auth failed: {0}")]
    AuthFailed(String),
    #[error("network: {0}")]
    Network(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("session expired")]
    SessionExpired,
}
```

### 3.2 Registry + 自动匹配

```rust
pub struct QuotaProviderRegistry {
    providers: Vec<Box<dyn QuotaProvider>>,
}

impl QuotaProviderRegistry {
    /// 按 provider 名称或 base_url 模式自动选择采集策略
    pub fn resolve(&self, ctx: &PollContext) -> Option<&dyn QuotaProvider> {
        // 1. 用户显式配置 strategy → 精确匹配
        if let Some(ref cfg) = ctx.quota_config {
            if cfg.strategy == "disabled" { return None; }
            return self.find_by_id(&cfg.strategy);
        }
        // 2. 按 provider 名称精确匹配
        if let Some(p) = self.find_by_id(&ctx.provider) { return Some(p); }
        // 3. 按 base_url 模式匹配（siliconflow, moonshot 等）
        if let Some(p) = self.match_by_url(&ctx.base_url) { return Some(p); }
        // 4. 兜底：尝试 NewAPI/OneAPI 兼容端点
        self.find_by_id("openai_compat")
    }
}
```

### 3.3 已知 Provider 内置匹配表

```rust
fn match_by_url(&self, url: &str) -> Option<&dyn QuotaProvider> {
    match () {
        _ if url.contains("siliconflow") => self.find_by_id("siliconflow"),
        _ if url.contains("moonshot")    => self.find_by_id("moonshot"),
        _ if url.contains("stepfun")     => self.find_by_id("stepfun"),
        _ if url.contains("novita")      => self.find_by_id("novita"),
        _ if url.contains("shengsuanyun")=> self.find_by_id("shengsuanyun"),
        // 云厂商 WebView 抓取（Tauri only）
        #[cfg(feature = "tauri")]
        _ if url.contains("dashscope") || url.contains("aliyuncs") => self.find_by_id("aliyun_webview"),
        #[cfg(feature = "tauri")]
        _ if url.contains("baidubce") => self.find_by_id("baidu_webview"),
        _ => None,
    }
}
```

---

## 四、专用 Provider 实现详情

### 4.1 OpenRouter

```rust
// GET /api/v1/key → { "data": { "limit_remaining": 87.35, "limit": 100.0,
//   "usage": 12.65, "usage_daily": 1.23, "usage_weekly": 4.56, "usage_monthly": 12.65 } }
// 映射: balance = limit_remaining, limit, usage
//       items = [{ daily, weekly, monthly }]
```

### 4.2 DeepSeek（已有，迁移）

```rust
// GET /user/balance → { "balance_infos": [{ "currency": "CNY", "total_balance": "110.00" }] }
// 映射: balance = total_balance (CNY)
```

### 4.3 SiliconFlow

```rust
// GET /v1/user/info → { ... "balance": ..., "totalBalance": ... }
// 映射: balance = totalBalance
```

### 4.4 Moonshot/Kimi

```rust
// GET /v1/users/me/balance → { "data": { "available_balance": 49.59, "voucher_balance": 46.59, "cash_balance": 3.00 } }
// 映射: balance = available_balance
//       items = [{ voucher_balance }, { cash_balance }]
```

### 4.5 StepFun

```rust
// GET /v1/accounts → { "object": "account", "type": "prepaid",
//   "balance": 0.0, "total_cash_balance": 0.0, "total_voucher_balance": 26.0 }
// 映射: balance = total_cash_balance + total_voucher_balance
//       items = [{ cash }, { voucher }]
```

### 4.6 Novita AI

```rust
// GET /openapi/v1/user/balance → 余额（单位 0.0001 USD）
// 映射: balance = value / 10000.0
```

### 4.7 胜算云

```rust
// GET /api/v1/key → { "data": { "max_quota": 500000, "consumed_amount": 12345 } }
// 映射: balance = (max_quota - consumed_amount) / 500000.0 (单位转换)
```

### 4.8 NewAPI/OneAPI 兼容（通用兜底）

```rust
// 尝试1: GET {base_url}/dashboard/billing/subscription → hard_limit_usd
// 尝试2: GET {base_url}/dashboard/billing/usage?start_date=...&end_date=... → total_usage
// 尝试3: GET {base_url}/v1/dashboard/billing/subscription (带 /v1/ 前缀变体)
// 映射: balance = hard_limit_usd - total_usage/100, limit = hard_limit_usd, usage = total_usage/100
```

---

## 五、被动响应头提取实现

### 5.1 Header 映射表

```rust
pub struct QuotaHeaders {
    pub remaining_requests: Option<u64>,
    pub limit_requests: Option<u64>,
    pub remaining_tokens: Option<u64>,
    pub limit_tokens: Option<u64>,
    pub reset_requests: Option<String>,  // e.g. "2m0.12s" or ISO 8601
    pub reset_tokens: Option<String>,
}

impl QuotaHeaders {
    pub fn extract(provider: &str, headers: &HeaderMap) -> Option<Self> {
        match provider {
            // OpenAI / Azure / PipeLLM / 多数 Custom
            "openai" | _ => Self::extract_x_ratelimit(headers),
            // Anthropic 使用不同前缀
            "anthropic" => Self::extract_anthropic_ratelimit(headers),
        }
    }

    fn extract_x_ratelimit(headers: &HeaderMap) -> Option<Self> { ... }

    fn extract_anthropic_ratelimit(headers: &HeaderMap) -> Option<Self> {
        // anthropic-ratelimit-requests-remaining
        // anthropic-ratelimit-requests-limit
        // anthropic-ratelimit-tokens-remaining
        // anthropic-ratelimit-tokens-limit
        // anthropic-ratelimit-requests-reset (ISO 8601)
        // anthropic-ratelimit-tokens-reset (ISO 8601)
    }
}
```

### 5.2 集成点

在 `proxy/mod.rs` 的 `dispatch()` 函数中，成功响应后提取：

```rust
// dispatch() 成功路径末尾添加
if let Some(quota_headers) = QuotaHeaders::extract(&provider_str, &resp_headers) {
    let mut info = quota_store.get(channel.id).await
        .unwrap_or_else(|| QuotaInfo::new(channel.id, &channel.name, &provider_str));
    info.update_from_headers(&quota_headers);
    quota_store.update(info).await;
}
```

### 5.3 修复现有 Bug

当前 `proxy/mod.rs:231-236` 的 `PASSTHROUGH_RESPONSE_HEADERS` 缺少 Anthropic 头。需添加：
```rust
"anthropic-ratelimit-requests-limit",
"anthropic-ratelimit-requests-remaining",
"anthropic-ratelimit-requests-reset",
"anthropic-ratelimit-tokens-limit",
"anthropic-ratelimit-tokens-remaining",
"anthropic-ratelimit-tokens-reset",
```

---

## 六、WebView Cookie 抓取（Feature-gated）

### 6.1 可抓取的 Provider

| Provider | 抓取目标 | 方式 | 数据产出 |
|----------|---------|------|---------|
| **Anthropic Console** | `console.anthropic.com/api/oauth/usage` | Cookie + fetch | 5h/7d utilization + resets_at |
| **百度千帆 编码套餐** | `console.bce.baidu.com/api/qianfan/charge/codingPlan/resourceList` | Cookie + fetch | used/limit/remain per 5h/week/month |
| **阿里云百炼 编码套餐** | `bailian.console.aliyun.com` 订阅页面 | Cookie + DOM | 状态/天数/5h/周/月用量 |
| **ChatGPT** | `chatgpt.com` 订阅状态 | Cookie + DOM | Plus/Pro 状态, 用量 |
| **DeepSeek Web** | `chat.deepseek.com` 用量 | Cookie + DOM | 套餐用量 |

### 6.2 Anthropic OAuth Usage（高优先级 WebView 目标）

已确认端点和响应格式：
```
GET https://console.anthropic.com/api/oauth/usage
Cookie: <session cookies>

Response:
{
  "five_hour": {
    "utilization": 27.0,      // 百分比
    "resets_at": "2026-02-26T01:00:01Z"
  },
  "seven_day": {
    "utilization": 4.0,
    "resets_at": "2026-03-04T15:00:00Z"
  },
  "seven_day_sonnet": { ... },
  "seven_day_opus": null,
  "extra_usage": null
}
```

这是一个结构清晰的 API，只需 Cookie 认证，数据归一化简单。

---

## 七、数据模型

### 7.1 通用 `QuotaInfo`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaInfo {
    pub channel_id: Uuid,
    pub channel_name: String,
    pub provider: String,

    // 标准化字段
    pub balance: Option<f64>,          // 可用余额
    pub limit: Option<f64>,            // 总额度
    pub usage: Option<f64>,            // 已用
    pub remaining_tokens: Option<u64>,
    pub remaining_requests: Option<u64>,

    // 套餐/订阅
    pub plan_status: Option<String>,
    pub expires_at: Option<String>,

    // 结构化详情
    pub items: Vec<QuotaItem>,         // 键值对（日用量、周用量等）
    pub groups: Vec<QuotaGroup>,       // 分时段用量组
    pub compact_text: Option<String>,

    // 被动速率限制
    pub rate_limit_remaining_req: Option<u64>,
    pub rate_limit_limit_req: Option<u64>,
    pub rate_limit_remaining_tok: Option<u64>,
    pub rate_limit_limit_tok: Option<u64>,
    pub rate_limit_updated_at: Option<DateTime<Utc>>,

    // 元信息
    pub source: String,                // "http_api" | "webview" | "response_header"
    pub updated_at: DateTime<Utc>,
    pub error: Option<String>,
}
```

### 7.2 归一化映射（实际 Provider 数据）

| Provider | balance | limit | usage | items | groups | source |
|----------|---------|-------|-------|-------|--------|--------|
| OpenRouter | `limit_remaining` | `data.limit` | `data.usage` | daily/weekly/monthly | -- | http_api |
| DeepSeek | `total_balance` | -- | -- | -- | -- | http_api |
| SiliconFlow | `totalBalance` | -- | -- | -- | -- | http_api |
| Moonshot | `available_balance` | -- | -- | voucher, cash | -- | http_api |
| StepFun | `balance` | -- | -- | cash, voucher | -- | http_api |
| 胜算云 | max-consumed | max_quota | consumed | -- | -- | http_api |
| NewAPI系 | limit-usage/100 | hard_limit_usd | total_usage/100 | -- | -- | http_api |
| Anthropic(Web) | -- | -- | -- | 5h/7d utilization | -- | webview |
| 百度千帆(Web) | -- | -- | -- | 状态/到期 | 5h/周/月 | webview |
| 阿里云(Web) | -- | -- | -- | 状态/天数 | 5h/周/月 | webview |
| OpenAI(被动) | -- | -- | -- | -- | -- | response_header |
| Anthropic(被动) | -- | -- | -- | -- | -- | response_header |

---

## 八、配置设计

### 8.1 Channel 级配额配置

```toml
[[channels]]
name = "My SiliconFlow"
provider = "custom"
base_url = "https://api.siliconflow.cn"

# 自动识别，无需手动配置
# 若需覆盖默认行为:
[channels.quota]
strategy = "http_api"              # http_api | openai_compat | webview | response_header | disabled
balance_url = "/v1/user/info"     # 自定义端点
balance_path = "$.totalBalance"    # JSONPath 提取
refresh_secs = 300
```

### 8.2 Preset 扩展

```typescript
export interface ProviderPreset {
  // ...现有字段...
  quotaStrategy?: "http_api" | "openai_compat" | "webview" | "response_header" | "disabled";
  quotaEndpoint?: string;
  quotaBalancePath?: string;
}
```

### 8.3 匹配逻辑优先级

1. 用户显式 `quota.strategy` → 精确匹配
2. Provider 精确匹配（`openrouter`, `deepseek` 等）
3. Base URL 模式匹配（`siliconflow`, `moonshot` 等）
4. 兜底：尝试 `openai_compat`（`/dashboard/billing/*`）
5. 最终兜底：仅被动响应头提取

---

## 九、分阶段实施

### Phase 1: 框架抽象 + 数据模型（2-3h）

| 文件 | 变更 |
|------|------|
| `src-tauri/src/quota/provider.rs` | **新建** `QuotaProvider` trait + `QuotaError` + `PollContext` |
| `src-tauri/src/quota/registry.rs` | **新建** `QuotaProviderRegistry` |
| `src-tauri/src/quota/mod.rs` | 扩展 `QuotaInfo`（+items, groups, rate_limit_*, source） |
| `src-tauri/src/quota/poller.rs` | 重构为调用 Registry |
| `src-tauri/src/config.rs` | 增加 `QuotaConfig` 结构 |
| `src-tauri/src/lib.rs` | 构建 Registry 并注入 AppState |

### Phase 2: 7 个专用 Billing API 采集器（3-4h）

| 文件 | Provider |
|------|----------|
| `collectors/openrouter.rs` | OpenRouter：`/api/v1/key` |
| `collectors/deepseek.rs` | DeepSeek：`/user/balance` |
| `collectors/siliconflow.rs` | SiliconFlow：`/v1/user/info` |
| `collectors/moonshot.rs` | Moonshot：`/v1/users/me/balance` |
| `collectors/stepfun.rs` | StepFun：`/v1/accounts` |
| `collectors/novita.rs` | Novita：`/openapi/v1/user/balance` |
| `collectors/shengsuanyun.rs` | 胜算云：`/api/v1/key` |

### Phase 3: NewAPI 兼容采集器 + URL 匹配（2-3h）

| 文件 | 说明 |
|------|------|
| `collectors/openai_compat.rs` | 通用 `/dashboard/billing/*` 采集器 |
| `registry.rs` (URL 匹配) | 按 base_url 自动匹配已知 provider |

覆盖：AiHubMix, DMXAPI, Compshare, ModelScope, 所有 NewAPI 系中转站。

### Phase 4: 被动响应头提取（1-2h）

| 文件 | 变更 |
|------|------|
| `collectors/response_header.rs` | **新建** `QuotaHeaders::extract()` |
| `src-tauri/src/proxy/mod.rs` | `dispatch()` 成功路径中集成；**修复 `anthropic-ratelimit-*` 头遗漏** |

### Phase 5: 前端通用配额仪表盘（3-4h）

| 文件 | 说明 |
|------|------|
| `src/lib/api.ts` | 扩展 QuotaInfo 接口 |
| `src/components/QuotaCard.tsx` | **新建** 通用配额卡片 |
| `src/components/StatusDashboard.tsx` | 集成 QuotaCard，按 source 渲染 |

### Phase 6: WebView Cookie 抓取（4-6h，Feature-gated）

| 文件 | 说明 |
|------|------|
| `collectors/webview.rs` | WebView 采集器管理器 |
| `collectors/webview_scripts/anthropic.rs` | Anthropic OAuth usage（高优先级） |
| `collectors/webview_scripts/baidu.rs` | 百度千帆编码套餐 |
| `collectors/webview_scripts/aliyun.rs` | 阿里云百炼编码套餐 |
| `src-tauri/capabilities/provider-remote.json` | WebView IPC 权限 |

### Phase 7（延期）：云厂商 HMAC + 配额感知路由

- 百度/阿里/火山引擎 AK/SK 签名认证
- `router/mod.rs` 配额感知 channel 过滤

---

## 十、模块结构

```
quota/
├── mod.rs                        # QuotaInfo, QuotaItem, QuotaGroup, QuotaStore
├── provider.rs                   # QuotaProvider trait, QuotaError, PollContext
├── registry.rs                   # QuotaProviderRegistry + URL 匹配
├── poller.rs                     # 后台轮询调度器
└── collectors/
    ├── mod.rs
    ├── openrouter.rs             # GET /api/v1/key
    ├── deepseek.rs               # GET /user/balance
    ├── siliconflow.rs            # GET /v1/user/info
    ├── moonshot.rs               # GET /v1/users/me/balance
    ├── stepfun.rs                # GET /v1/accounts
    ├── novita.rs                 # GET /openapi/v1/user/balance
    ├── shengsuanyun.rs           # GET /api/v1/key
    ├── openai_compat.rs          # /dashboard/billing/* (NewAPI 系)
    ├── response_header.rs        # 被动 x-ratelimit-* + anthropic-ratelimit-*
    └── #[cfg(feature = "tauri")]
        webview.rs                # WebView Cookie 抓取
        └── webview_scripts/
            ├── anthropic.rs      # OAuth usage 端点
            ├── baidu.rs          # 编码套餐 resourceList
            ├── aliyun.rs         # DOM 抓取
            └── chatgpt.rs        # 订阅状态
```

---

## 十一、风险评估

| 风险 | 严重度 | 缓解措施 |
|------|--------|----------|
| **Provider API 变更** | 高 | 7 个专用 + 1 个通用兼容 = 两层防御；通用层覆盖面最广 |
| **NewAPI 系端点差异** | 中 | 尝试 `/dashboard/billing/` + `/v1/dashboard/billing/` 两种前缀 |
| **Anthropic 响应头遗漏**（已发现 Bug） | 高 | Phase 4 修复：添加 `anthropic-ratelimit-*` 到提取逻辑 |
| **WebView 仅 Tauri 可用** | 高 | Feature gate；无 WebView 时回退到被动响应头 |
| **JS 注入脚本随厂商改版失效** | 中 | 优先抓取 API 端点（百度、Anthropic）；DOM 抓取加 error 监控 |
| **Google Gemini 无任何配额信息** | 中 | 从响应 `usageMetadata` 累计 token 用量；429 时标记限流 |
| **云厂商 AK/SK 认证复杂** | 中 | Phase 7 延期；初期通过 WebView 或手动配置解决 |
| **WebView 内存（100-200MB/窗口）** | 中 | 限制最多 3 个活跃 WebView；采集后销毁重建 |

---

## 十二、实施优先级

| 优先级 | Phase | 覆盖 | 预估 |
|--------|-------|------|------|
| **P0** | Phase 1: 框架抽象 | 基础 | 2-3h |
| **P0** | Phase 2: 7 个专用 API | 7 家 | 3-4h |
| **P0** | Phase 3: NewAPI 兼容 | ~15 家 | 2-3h |
| **P0** | Phase 4: 被动响应头 | ~30 家 | 1-2h |
| **P1** | Phase 5: 前端仪表盘 | 展示 | 3-4h |
| **P2** | Phase 6: WebView 抓取 | ~5 家 | 4-6h |
| **P3** | Phase 7: 云厂商 + 路由 | 扩展 | 4-5h |

**P0 交付后**：所有 30+ Provider 均有配额数据（22 家主动 API + 30 家被动提取）。

---

## 十三、参考来源

- [OpenAI Rate Limits & Admin Billing API](https://platform.openai.com/docs/guides/rate-limits)
- [Anthropic OAuth Usage Endpoint](https://docs.anthropic.com/en/api/rate-limits)
- [OpenRouter Key Info API](https://openrouter.ai/docs/api/reference/limits)
- [StepFun Account API](https://platform.stepfun.com/docs/zh/api-reference/accounts/get)
- [百度智能云余额查询](https://cloud.baidu.com/doc/Finance/s/Skhtyytwu)
- [火山引擎 QueryBalanceAcct](https://www.volcengine.com/docs/6269/1223898)
- [NewAPI Dashboard Router](https://github.com/Calcium-Ion/new-api/blob/main/router/dashboard.go)
- [OneAPI Billing Controller](https://github.com/songquanpeng/one-api/blob/main/controller/billing.go)
- [Novita AI Billing API](https://novita.ai/docs/api-reference/basic-query-monthly-bill)
- [胜算云 API Key](https://docs.router.shengsuanyun.com/v1/key)
- [TheRouter Credits API](https://therouter.ai/docs/api/api-reference/credits/get-credits/)
- [PipeLLM Rate Limits](https://docs.pipellm.ai/guides/rate-limits.md)
- [MiniMax Balance Feature Request](https://github.com/MiniMax-AI/MiniMax-M2.5/issues/6)
- [NVIDIA NIM Credits Forum](https://forums.developer.nvidia.com/t/cannot-find-the-amount-of-credits-left-on-nim-api/337051)
- [阿里云 BSS QueryAccountBalance](https://help.aliyun.com/zh/user-center/developer-reference/api-bssopenapi-2017-12-14-queryaccountbalance)
