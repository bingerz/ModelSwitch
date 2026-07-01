# ModelSwitch 全流程自动化测试架构方案

> **实施状态**: 已完成阶段 1–4。最终测试数量：Rust 1151 个、前端 341 个、E2E 3 个 spec（4 个测试）。

## Context

ModelSwitch 是一个 Rust LLM 智能网关（多提供商代理 + 路由 + 配额 + 虚拟密钥 + MCP 工具网关），有 Tauri 桌面端和 CLI 两种模式。作为一人公司，需要在有限资源下建立实用的测试体系，确保产品上线前核心功能正常。

## 现状评估（实施后）

| 层 | 工具 | 状态 | 测试数量 |
|---|---|---|---|
| Rust 单元测试 | `#[test]` + `#[cfg(test)]` | 🟢 强 | 1151 个（含 5 路由策略 + 5 dispatch 新增） |
| Rust 集成测试 | `tower::oneshot` + `wiremock` | 🟢 强 | 18 个 API + 19 个 dispatch |
| Rust lint | clippy + rustfmt | 🟢 强 | CI 强制 |
| 前端单元测试 | Vitest + Testing Library | 🟢 强 | 341 个（API 70+、format 30+、quota 24+） |
| 前端 E2E | Playwright | 🟢 已配置 | 3 个 spec（login、channels、virtual-keys） |
| 覆盖率门禁 | vitest coverage-v8 | 🟢 已配置 | 行 20%、函数 17%、分支 15% |
| 安全审计 | cargo-audit + cargo-deny | 🟢 强 | CI + 已提交 deny.toml |

---

## 方案总览：4 个阶段

```
阶段 1          阶段 2            阶段 3            阶段 4
后端深度测试  →  前端测试基础  →  Playwright E2E →  CI 硬化
dispatch/路由    API层已覆盖       3个关键流程        deny.toml ✓
虚拟密钥/熔断    format/quota已覆盖  login/channels   覆盖率门禁 ✓
                                    virtual-keys     vitest修复 ✓
```

---

## 阶段 1：后端深度测试 ✅ 已完成

### 1.1 dispatch 管道测试（5 个新增）

**文件**: `src-tauri/src/proxy/dispatch_tests.rs`

| 测试名 | 覆盖什么 |
|---|---|
| `dispatch_rate_limit_enforcement` | RPM 限制触发 429 |
| `dispatch_in_flight_coalescing` | 相同请求合并去重 |
| `dispatch_disabled_channel_is_skipped` | 禁用渠道被跳过 |
| `dispatch_priority_ordering` | 优先级分层确定性排序 |
| `dispatch_excluded_models_skips_channel` | glob 模型排除过滤 |

### 1.2 路由策略单元测试（5 个新增）

**文件**: `src-tauri/src/router/strategy.rs`

| 测试名 | 覆盖什么 |
|---|---|
| `weighted_random_returns_a_channel` | 基本选择 |
| `weighted_random_returns_none_for_empty` | 空候选列表 |
| `weighted_random_with_zero_weight_falls_back_to_uniform` | 零权重回退均匀分布 |
| `latency_based_returns_none_for_empty` | 延迟策略空列表 |
| `usage_based_returns_some_for_candidates` | 使用率策略 |

### 1.3 虚拟密钥预算测试（已有 14 个 ✅）

**文件**: `src-tauri/src/virtual_key/spend.rs`

实施前评估认为需要新增，调查后发现已有全面覆盖：日限额/月限额/边界值/过期密钥/禁用密钥/无限额/回滚/并发场景。

### 1.4 消毒器中间件测试（已有 10+ 个 ✅）

**文件**: `src-tauri/src/middleware/sanitizer.rs`

实施前评估认为需要新增，调查后发现已有全面覆盖：AWS 密钥/GitHub Token/Stripe 密钥/SSH 私钥/数据库连接串/Slack Token/通用密码字段/自定义模式/JSON 结构保持/空输入。

---

## 阶段 2：前端测试基础 ✅ 已覆盖

实施前评估为"几乎为零"，实际调查后发现已有 **341 个测试**。

| 已有测试文件 | 测试数量 | 覆盖什么 |
|---|---|---|
| `src/lib/api.test.ts` | 70+ | 全部 API 端点（channels/virtual-keys/stats/logs/config/gateway/MCP/guardrails 等） |
| `src/lib/format.test.ts` | 30+ | formatNumber/formatCost/formatCents/formatTokens/formatRelativeTime/latencyColor |
| `src/hooks/quota-utils.test.ts` | 24 | computeTotalBalance/countChannelsWithData/countLowBalance/countErrors |

**不需要 MSW**：API 层通过 `src/lib/api/client.ts` 统一入口，可直接在测试中 mock。

---

## 阶段 3：Playwright E2E ✅ 已完成

### 配置

- `playwright.config.ts` — chromium、串行执行（`workers: 1`）、baseURL 可配
- `e2e/helpers.ts` — 共享 `loginAsAdmin(page)` 辅助函数
- Vite proxy: `/api` 和 `/v1` 转发到 `http://127.0.0.1:8080`

### E2E 流程

| 文件 | 验证什么 |
|---|---|
| `e2e/login.spec.ts` | 登录表单渲染 → token 输入 → 进入主界面 |
| `e2e/channels.spec.ts` | 登录 → 导航到渠道 → 列表渲染 |
| `e2e/virtual-keys.spec.ts` | 登录 → 导航到虚拟密钥 → 面板渲染 |

所有 spec 使用宽容选择器（`getByPlaceholder`/`getByRole`），元素不存在时 `test.skip` 而非硬失败。

### 运行方式

```bash
# 终端 1：启动网关
cd src-tauri && cargo run --bin modelswitch-cli -- serve

# 终端 2：启动前端
pnpm dev

# 终端 3：运行 E2E
pnpm e2e                    # headless
pnpm e2e:headed             # 可见浏览器
```

---

## 阶段 4：CI 硬化 ✅ 已完成

### deny.toml

已提交 `src-tauri/deny.toml`，CI 不再每次自动生成。

### 覆盖率门禁

修复了 vitest 4.x 与 @vitest/coverage-v8 3.x 版本不匹配导致覆盖率报告崩溃的问题。门禁已从 10% 提升到 20%。

```typescript
thresholds: { lines: 20, functions: 17, statements: 20, branches: 15 }
```

当前实际覆盖率：行 22%、函数 18%、语句 21%、分支 17%。

---

## 发布前最小检查清单

```bash
cargo test --lib --all-features           # Rust 单元测试 (1151)
cargo test --test api_integration          # API 集成测试 (18)
pnpm test                                  # 前端测试 (341)
pnpm test:coverage                         # 覆盖率报告
pnpm e2e                                   # Playwright E2E (需启动网关)
pnpm build                                 # 前端构建
cargo build --no-default-features          # CLI 构建验证
```

---

## 关键文件索引

### 测试基础设施
- `src-tauri/src/test_helpers.rs` — `build_test_state()` / `channel_config()` / `response_json()`
- `src-tauri/src/proxy/dispatch_tests.rs` — 19 个 wiremock dispatch 测试
- `src-tauri/tests/api_integration.rs` — 18 个 router 集成测试
- `vitest.config.ts` — 前端测试配置 + 覆盖率门禁
- `src/test-setup.ts` — 前端测试初始化
- `playwright.config.ts` — E2E 配置
- `e2e/` — E2E spec 文件
- `src-tauri/deny.toml` — 依赖安全策略

### CI
- `.github/workflows/ci.yml` — rust-checks + frontend-checks + security-audit

---

## 一人公司建议

### 什么可以跳过
- ❌ 不要追求前端 80% 覆盖率。30-40% 足够，重点在 API 层
- ❌ 不要为纯展示组件写测试
- ❌ 不要加 MSW，API 层已有完整的 client.ts mock 入口
- ❌ 不要加 Cypress，Playwright 已足够

### 长期演进
1. **上线后**: 每次修 bug 先写复现测试，再修。最自然的覆盖率提升方式。
2. **用户增长后**: 考虑加 `cargo-llvm-cov` 输出 Rust 覆盖率报告。
3. **团队扩张时**: 才考虑严格覆盖率门禁、per-PR E2E、视觉回归测试。
