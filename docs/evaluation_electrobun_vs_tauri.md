# Electrobun vs Tauri 2.0 — ModelSwitch 技术选型评估

> **日期**: 2026-05-10 · **评估目的**: 判断 Electrobun 是否可替代 Tauri 2.0 作为 ModelSwitch 的桌面框架

---

## 1 Electrobun 概况

| 维度 | 现状 |
|------|------|
| **版本** | v1.0 稳定版 (2026-02 发布) |
| **运行时** | Bun (主进程) + 系统 WebView (渲染) |
| **语言** | 全栈 TypeScript，底层 Zig 绑定 |
| **包体积** | 12–64 MB (vs Electron 100MB+) |
| **启动速度** | < 50ms |
| **平台** | macOS / Windows / Linux |
| **移动端** | ❌ 不支持 |
| **生态成熟度** | Early Stable — 核心稳定，生态仍在成长 |

---

## 2 与 Tauri 2.0 正面对比

| 维度 | Tauri 2.0 | Electrobun | ModelSwitch 权重 |
|------|-----------|------------|-----------------|
| **后端语言** | Rust | TypeScript (Bun) | 🔴 关键 |
| **包体积** | 5–15 MB | 12–64 MB | 🟡 中等 |
| **启动速度** | < 500ms | < 50ms | 🟢 低 |
| **CPU 密集型性能** | ⭐⭐⭐⭐⭐ (原生编译) | ⭐⭐⭐ (JIT) | 🔴 关键 |
| **I/O 吞吐** | ⭐⭐⭐⭐⭐ (Tokio) | ⭐⭐⭐⭐ (Bun) | 🔴 关键 |
| **SSE 流式转发** | 原生 Axum SSE | Bun fetch stream | 🔴 关键 |
| **安全存储** | keyring-rs (成熟) | Bun.secrets / FFI | 🔴 关键 |
| **WebView Cookie** | Rust 原生钩子 | JS 注入 | 🟡 中等 |
| **Tray 托盘** | 原生支持 | 支持 | 🟢 低 |
| **社区生态** | ⭐⭐⭐⭐⭐ (70k+ stars) | ⭐⭐ (新兴) | 🟡 中等 |
| **学习曲线** | 需要 Rust | 纯 TypeScript | 🟡 中等 |

---

## 3 针对 ModelSwitch 核心需求逐项评估

### 3.1 本地反向代理网关 (P99 < 5ms) ⚠️ 决定性因素

**Tauri (Axum + Tokio)**：
- Rust 编译为原生机器码，零 GC 停顿
- Axum 框架 P99 延迟稳定 < 1ms
- Tokio 异步运行时经数年生产验证
- **完美匹配** PRD "参考 TensorZero 纯 Rust 异步网关" 的设计目标

**Electrobun (Bun HTTP Server)**：
- Bun HTTP 性能在 JS 运行时中属顶级，但仍是 JIT 解释执行
- 存在 GC 停顿风险，P99 尾延迟不可控 (偶发 10-50ms 抖动)
- 高并发下单线程事件循环可能成为瓶颈
- 可能达到 P99 < 5ms 但**没有安全余量**

```
性能对比 (本地代理转发延迟估算):
┌──────────────┬────────────┬──────────────┐
│ 场景         │ Tauri/Axum │ Electrobun   │
├──────────────┼────────────┼──────────────┤
│ P50 延迟     │ < 0.3ms    │ < 1ms        │
│ P99 延迟     │ < 1ms      │ 3-15ms (GC)  │
│ 并发 100 req │ 稳定       │ 可能排队     │
│ SSE 长连接   │ 零拷贝转发 │ Buffer 中转  │
└──────────────┴────────────┴──────────────┘
```

### 3.2 SSE 流式透传

- **Tauri**：Axum 原生 `Sse<impl Stream>` + reqwest stream，字节级零拷贝透传
- **Electrobun**：Bun `fetch` + `ReadableStream`，经 JS 层序列化/反序列化，有额外开销

### 3.3 热重试 (同请求内无感切换)

- **Tauri**：Rust 所有权模型保证请求体在重试时安全复用，无拷贝开销
- **Electrobun**：需手动管理请求体 buffer，技术可行但更复杂

### 3.4 凭证安全存储 — ✅ 两者持平

- **Tauri**：`keyring-rs` 成熟可靠
- **Electrobun**：`Bun.secrets` API 功能等价

### 3.5 WebView Cookie 拦截 — Tauri 略优

- **Tauri 2.0**：`Webview::cookies()` Rust API 可直接获取含 HttpOnly 的 Cookie
- **Electrobun**：JS 注入 `document.cookie` 受 HttpOnly 限制

---

## 4 架构适配性

### 4.1 PRD 参考项目匹配度

| 参考项目 | 核心借鉴 | Tauri | Electrobun |
|----------|----------|-------|------------|
| **TensorZero** | Rust 异步高性能网关 | ✅ 原生复用 | ❌ 需 Bun 重写 |
| **token-monitor** | WebView Cookie 注入 | ✅ Rust 钩子 | ⚠️ JS 注入 |
| **new-api** | Channel 路由算法 | ✅ | ✅ |
| **free-llm-gateway** | 熔断状态机 | ✅ | ✅ |

> PRD 明确要求"参考 TensorZero 纯 Rust 异步网关设计"。选用 Electrobun 意味着**放弃这条核心技术路线**。

### 4.2 架构对比

```
Tauri 方案:                      Electrobun 方案:
┌─────────────────┐              ┌─────────────────┐
│ Tauri App       │              │ Electrobun App  │
│ ├─ Axum Server  │ ← P99<1ms   │ ├─ Bun Server   │ ← P99 不确定
│ ├─ WebView UI   │              │ ├─ WebView UI   │
│ └─ Tauri IPC    │              │ └─ Bun IPC      │
└─────────────────┘              └─────────────────┘
```

---

## 5 风险评估

### Electrobun 特有风险

| 风险 | 严重度 | 说明 |
|------|--------|------|
| GC 尾延迟 | 🔴 高 | JS 运行时 GC 在高负载下可能突破 P99 < 5ms |
| 生态不成熟 | 🔴 高 | v1.0 仅发布 4 个月，排查资料有限 |
| 单线程瓶颈 | 🟡 中 | Bun 主进程单线程，CPU 操作会阻塞 |
| HttpOnly Cookie | 🟡 中 | JS 无法读取 HttpOnly Cookie |
| Breaking Changes | 🟡 中 | 新框架 API 可能不稳定 |

---

## 6 评分矩阵

| 评估维度 | 权重 | Tauri 2.0 | Electrobun |
|----------|------|-----------|------------|
| 网关性能 (P99) | 30% | 10 | 6 |
| SSE 流式转发 | 15% | 10 | 7 |
| 热重试可靠性 | 15% | 9 | 7 |
| 凭证安全 | 10% | 9 | 8 |
| WebView Cookie | 10% | 9 | 6 |
| 开发效率 | 10% | 6 | 9 |
| 生态成熟度 | 5% | 10 | 4 |
| 包体积 | 5% | 9 | 8 |
| **加权总分** | **100%** | **9.15** | **6.80** |

---

## 7 结论

### ❌ Electrobun 不推荐用于 ModelSwitch

1. **性能风险**：核心价值是"无感零延迟代理"，JS GC 停顿在高并发下不可控
2. **架构脱节**：PRD 以 TensorZero (Rust) 为标杆，改用 Bun 等于放弃核心技术路线
3. **生态过新**：文档/社区/插件不足以支撑生产项目
4. **功能受限**：HttpOnly Cookie 限制影响 Web Session 功能

### ✅ 建议维持 Tauri 2.0

| 如果... | 建议... |
|---------|---------|
| 团队不熟悉 Rust | 先用 2 周学 Rust 基础 + Axum，长期回报大 |
| 想快速出 MVP | 里程碑 1 (单渠道代理) Axum 样板代码量很少 |
| 确实想用 Electrobun | 仅用于 UI 壳，代理网关仍用 Rust 独立进程 (增加复杂度) |

---

*评估结束。建议维持 Tauri 2.0 方案，聚焦里程碑 1 开发。*
