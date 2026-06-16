# ModelSwitch 参考项目评估 — 实施进度

> 基于对 LiteLLM、New-API、TensorZero、Higress 的对比分析
> 最后更新: 2026-06-16

## 总览

| 指标 | 初始 | 最新 | 变化 |
|------|------|------|------|
| 测试数量 | 157 | 244 | +87 (+55%) |
| Clippy 警告 | 0 (lib) | 0 (all) | 零警告全覆盖 |
| 缓存哈希 | u64 DefaultHasher | u128 BLAKE3 | 128-bit 抗碰撞 |
| 路由策略 | 3 种 | 5 种 | +用量优先, +最低成本 |
| HTTP 连接池 | 仅 timeout | 6 项调优 + 多实例池 | 连接复用提升 |
| 计费模式 | 事后统计 | 预扣+对账 | 防止超额 |
| 缓存模式 | 单一 On | 4 种 (On/Off/RO/WO) | 灵活控制 |
| Provider 适配 | AuthStyle enum | ProviderAdaptor trait | 可扩展 |
| 响应路径 | 同步阻塞 | 全异步非阻塞 | 低延迟 |
| 流式保护 | 无断连检测 | mpsc 消息通道 | 遥测完整 |
| Provider 预算 | 无 | 每日/每月限额 | 成本控制 |

---

## Phase A: 性能快赢 ✅ 已完成

| 项目 | 状态 | Commit |
|------|------|--------|
| HTTP 连接池调优 (connect_timeout/pool_idle/keepalive/nodelay) | ✅ | f2f3ac2 |
| BLAKE3 缓存哈希 (u128, 抗碰撞) | ✅ | f2f3ac2 |
| stream_options.include_usage 注入 | ✅ | f2cd352 |
| 非阻塞响应写入 (spawn_bg for quota/billing/logging) | ✅ | 4ff3182 |

## Phase B: 路由增强 ✅ 已完成

| 项目 | 状态 | Commit |
|------|------|--------|
| 用量优先路由 (UsageBasedStrategy) | ✅ | 4685323 |
| 延迟滑动窗口 (LatencyTracker, 10样本+p95) | ✅ | 4685323 |
| 并发请求限制 (max_concurrent) | ✅ | 4685323 |
| 优先级分层 (已有，确认无需修改) | ✅ | — |
| 最低成本路由 (LowestCostStrategy) | ✅ | a2ade3e |

## Phase C: 计费增强 ✅ 已完成

| 项目 | 状态 | Commit |
|------|------|--------|
| 虚拟密钥预扣费 + 对账 (reserve_spend/reconcile_spend) | ✅ | 19c1b80 |
| 失败自动退款 | ✅ | 19c1b80 |
| 提供商预算限制 (ProviderBudgetStore, daily/monthly caps) | ✅ | 4ff3182 |
| 批量配额写入 (PersistedStore 周期性持久化) | ✅ | 已有 |

## Phase D: 流式健壮性 ✅ 已完成

| 项目 | 状态 | Commit |
|------|------|--------|
| TTFT (首 token) 超时 — 30s 默认，触发通道重试 | ✅ | 19c1b80 |
| FailureReason::Timeout 变体 | ✅ | 19c1b80 |
| 客户端断连检测 (mpsc channel drain pattern) | ✅ | 4ff3182 |
| 副作用保证 (断连后继续抽取遥测数据) | ✅ | 4ff3182 |
| 流式响应缓存 (SSE text accumulated + replay) | ✅ | 6d469a2 |

## Phase E: 高级特性 ✅ 大部分完成

| 项目 | 状态 | Commit |
|------|------|--------|
| 缓存模式 (On/Off/ReadOnly/WriteOnly) | ✅ | 2cdaea6 |
| 多密钥通道 (key 轮换) | ✅ | 95ae701 |
| 语义缓存 (需 embedding 模型) | ⬜ 长期 |
| 分层定价 (表达式引擎) | ⬜ 长期 |
| 自适应路由 (Thompson Sampling) | ⬜ 长期 |

## 架构改进 ✅ 已完成

| 项目 | 状态 | Commit |
|------|------|--------|
| ProviderAdaptor trait (替代 AuthStyle enum) | ✅ | 2841cb8 |
| ChannelManager 单元测试 (CRUD/熔断/密钥轮换) | ✅ | cc0f2a8 |
| Prometheus /metrics 端点 | ✅ | 已有 |

## 质量修复 ✅ 已完成

| 项目 | 状态 | Commit |
|------|------|--------|
| log 测试不稳定修复 (minute truncation) | ✅ | 8fe9197 |
| 测试代码 clippy 警告清理 (sanitizer/cache/cli) | ✅ | 8a0f5b5 |

---

## 实施总结

### 已实现的改进 (18 项)

1. HTTP 连接池调优 — connect_timeout/pool_idle/keepalive/nodelay
2. BLAKE3 缓存哈希 — 128-bit 抗碰撞
3. stream_options 注入 — 流式请求自动注入 include_usage
4. 用量优先路由 — 基于 TPM 利用率选择最空闲通道
5. 延迟滑动窗口 — 最近 10 次请求的加权平均 + p95
6. 并发请求限制 — per-channel max_concurrent
7. 虚拟密钥预扣费 — 请求前预扣估算成本，失败全额退款
8. TTFT 流式超时 — 30s 首 token 超时，触发通道重试
9. 缓存模式 — On/Off/ReadOnly/WriteOnly 四种
10. 多密钥通道 — api_keys: Vec<String> + round-robin rotation
11. 最低成本路由 — LowestCostStrategy 基于 blended cost
12. ProviderAdaptor trait — 多态化提供商逻辑
13. ChannelManager 测试 — 9 tests (CRUD + circuit breaker + key rotation)
14. 非阻塞响应写入 — spawn_bg 后台执行 quota/billing/logging/metrics
15. 客户端断连保护 — mpsc channel drain 保证遥测数据完整
16. 提供商预算限制 — per-provider daily/monthly caps + admin API
17. 流式响应缓存 — SSE text 累积 + cache hit replay
18. Channel 构建去重 — From<ChannelConfig> for Channel

### 未实现 (长期方向)

- 语义缓存 — 需引入 embedding 模型依赖
- 分层定价表达式引擎 — 复杂度高
- 自适应 Bandit 路由 — 需 Thompson Sampling
