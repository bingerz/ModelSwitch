# ModelSwitch OPC 场景分阶段优化升级方案

> **版本**: v1.0 · **日期**: 2026-07-03 · **状态**: Approved
> **编制依据**: 需求调研报告（18 条需求）· 差距分析报告（Top 10 差距）· 后端评估（7.8/10）· 前端运维评估（UX 6.4 / Ops 6.8 / 合规 3.0）
> **核心约束**: 一人公司极简运营 · 不引入专职运维依赖 · 渐进式交付 · 合规底线 IEC 62443 SL2 + 等保 2.0 基础

---

## 1 战略定位

### 1.1 产品定位声明

ModelSwitch 在 OPC 场景中**不是 SCADA 替代品**，而是"**AI 运维分析平面**"。现有产品 Ignition / Node-RED / Grafana 已经覆盖了数据采集、流程编排、可视化的能力域。ModelSwitch 不与它们竞争，而是作为"轻量 AI 增值层"接入数据流，提供三个差异化能力：

1. **告警智能分级**：LLM + 历史上下文 + MCP 工具注入，实现"AI 分析告警 → 调用工具查证 → 生成处置建议"闭环
2. **报表自动化**：时序数据 + 模板 + LLM 摘要，将工程师从周期性报告撰写中解放
3. **RAG 知识库**：客户文档 / SOP / 历史故障的可检索知识库，解决工具链碎片化下的知识管理痛点

### 1.2 双平面架构

```
┌──────────────────────────────────────────────────┐
│         AI 分析平面（ModelSwitch 现有核心）        │
│  多 LLM 调度 + MCP 工具注入 + 虚拟密钥 + 审计     │
│  + 告警智能分级 + 报表生成 + RAG 知识库           │
└──────────────────┬───────────────────────────────┘
                   │ Tokio broadcast channel（进程内）
┌──────────────────┴───────────────────────────────┐
│         数据平面（新建 opcua-bridge 模块）         │
│  OPC UA 客户端 + SQLite 时序存储 + 规则引擎       │
│  + WebSocket 实时推送 + 通知分发                  │
└──────────────────────────────────────────────────┘
```

**关键原则**：两平面在同一进程内通过 Tokio broadcast channel 解耦，不引入 Redis / Kafka 等外部消息中间件。数据平面独立演进，AI 分析平面无需感知 OPC UA 协议细节。

### 1.3 目标用户与商业模式

| 维度 | 定位 |
|------|------|
| 目标用户 | 工业自动化一人公司（3-8 家 retainer 客户） |
| 合同区间 | retainer 月 5,000-30,000 元/客户 |
| 年收入预期 | 20-100 万元（3-8 家客户） |
| 差异化 | AI 驱动的告警分析 + 报表自动化 + RAG 知识库 |
| 竞争壁垒 | 轻量化（单二进制 + SQLite）× AI 能力 × 合规留痕 |

### 1.4 不做什么

以下功能**明确排除**，避免功能蔓延：

- **不做 SCADA/HMI 画面组态**——Ignition / WinCC 已充分覆盖
- **不做 PLC 编程接口**——这是 TIA Portal / RSLogix 的领域
- **不做 LLM 直接控制指令下发**——功能安全红线，LLM 仅用于解读和建议
- **不做全量数据上云**——工业数据本地化优先，仅 LLM 调用走外部 API

---

## 2 分阶段路线图

### 总览

| 阶段 | 名称 | 周期 | 核心交付 | 需求覆盖 |
|------|------|------|----------|----------|
| A | 加固现有基座 | 4 周 | 修复 P0/P1 问题，修正 PRD 目标，提升测试覆盖 | 无新需求，地基加固 |
| B | OPC 数据平面 MVP | 6 周 | OPC UA 客户端 + SQLite 时序存储 + 规则引擎 + WebSocket | REQ-01~07 基础版 |
| C | 智能运维闭环 | 6 周 | 告警智能分级 + 报表自动化 + RAG 知识库 + 多渠道通知 | REQ-04, 09, 12, 13, 17 |
| D | 合规硬化与规模化 | 4 周 | IEC 62443 SL2 + 等保 2.0 + 控制指令安全管道 | REQ-08, 11, 15 + 合规 |

**总周期**：20 周（约 5 个月），含调试 buffer。

---

## 3 阶段 A：加固现有基座（4 周）

### 3.1 阶段目标

修复系统评估发现的 P0/P1 问题，使 ModelSwitch 的 LLM 网关基座达到"加固可生产"状态。本阶段**不引入任何新功能**，只加固。

### 3.2 验收标准

| 指标 | 当前值 | 目标值 |
|------|--------|--------|
| 后端 P1 问题 | 6 个未修 | 0 个未修 |
| 前端 P0 问题 | 4 个（OPC 合规相关） | 审计日志持久化 + PRD 修正完成 |
| 审计日志上限 | 1000 条环形缓冲 | 持久化 NDJSON 文件分页查询 |
| 安全头 | 缺 CSP/HSTS/Permissions-Policy | 补全 3 个头 |
| KDF 强度 | SHA-256 单次 | PBKDF2 ≥600k 轮 或 Argon2id |
| 前端测试覆盖 | ~22% lines | ≥40% lines |
| Prometheus 内存指标 | 缺失 | `process_resident_memory_bytes` 暴露 |
| PRD NFR-05 | < 50MB（不现实） | < 200MB（生产负载） |

### 3.3 任务清单

| # | 任务 | 工时 | 依赖 | 优先级 |
|---|------|------|------|--------|
| A-01 | 审计日志改为 NDJSON 文件持久化分页查询，移除 1000 条环形缓冲硬上限 | 2d | 无 | P0 |
| A-02 | `derive_key` 改用 PBKDF2（≥600k 轮）或支持 `MODELSWITCH_MASTER_KEY` env var 独立主密钥 | 1d | 无 | P0 |
| A-03 | 补全 CSP / HSTS / Permissions-Policy 安全头 | 0.5d | 无 | P0 |
| A-04 | Guardrails 增加 `check_response` 钩子，流式路径注入响应扫描 | 1.5d | 无 | P0 |
| A-05 | 预扣估值改为按 model_pricing 表查表估算 | 1d | 无 | P0 |
| A-06 | 审计哈希链增加周期性外部锚点（写入文件尾部签名，可后续升级为 S3 Object Lock） | 1d | A-01 | P0 |
| A-07 | Prometheus 暴露 `process_resident_memory_bytes` + `build_info{version, commit}` | 0.5d | 无 | P0 |
| A-08 | 补充告警规则：磁盘空间 > 80%、内存 > 150MB、上游不可达、预算 80% 预警 | 0.5d | A-07 | P0 |
| A-09 | PRD NFR-05 修正为 < 200MB（生产负载），同步更新 deployment-guide.md 的 Docker 内存限制 | 0.5d | 无 | P0 |
| A-10 | 前端测试覆盖率提升到 40%（核心组件：Playground / VirtualKeys / AuditLog / Channels） | 4d | 无 | P0 |
| A-11 | `translate.rs`（61KB）拆分为 `translate/openai.rs` / `anthropic.rs` / `gemini.rs` 子模块 | 2d | 无 | P1 |
| A-12 | i18n 中文缺失 69 keys 补全 + CI 加 i18n-key-consistency 检查 | 0.5d | 无 | P1 |
| A-13 | `cargo audit` + `cargo deny` 接入 CI | 0.5d | 无 | P1 |
| A-14 | MCP 子进程加 per-server 失败率熔断（复用 CooldownTracker） | 1d | 无 | P1 |
| A-15 | CostDashboard 轮询间隔从 5s 改为 30s（成本数据无需秒级刷新） | 0.5d | 无 | P2 |

**总工时估算**：约 17d（含 20% 调试 buffer = 约 20d = 4 周）。

### 3.4 技术方案

**审计日志持久化改造**：现有 `admin/audit.rs` 的环形缓冲改为"NDJSON 追加写入 + 按需读取"。查询接口改为 `offset + limit` 分页，直接读文件尾部 N 行。哈希链不变，只是不再截断。WAL 模式保证崩溃不丢。

**KDF 强化**：优先支持 `MODELSWITCH_MASTER_KEY` 环境变量，提供独立于 admin_token 的主密钥。env var 不存在时 fallback 到 PBKDF2(admin_token, salt, 600_000)。Argon2id 作为可选 feature（增加编译依赖，默认关闭）。

**translate.rs 拆分**：按目标协议分文件。`translate/mod.rs` 做 trait 定义与路由，子模块各自处理一方协议的转换逻辑。纯重构，零行为变化，靠现有 74KB 测试文件保障回归。

### 3.5 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| 审计日志文件持久化的分页查询性能在大文件下退化 | 中 | 中 | 用文件 seek + 反向读取尾部 N 行，避免全文件扫描 |
| translate.rs 拆分引入回归 | 低 | 高 | 拆分前先确保现有测试覆盖核心翻译路径，拆分后跑全量测试 |
| KDF 改动导致已加密 credentials 不可读 | 中 | 高 | 做迁移逻辑：旧格式检测后用旧 KDF 解密 → 新 KDF 重新加密 → 原子替换 |

### 3.6 资源需求

- 无外部采购
- 无硬件需求
- 工时：4 人周

---

## 4 阶段 B：OPC 数据平面 MVP（6 周）

### 4.1 阶段目标

从零建立 OPC UA 数据采集能力，实现"从一台 OPC UA Server 采集数据 → 存入 SQLite → 规则触发告警 → WebSocket 实时推送"的端到端闭环。

### 4.2 验收标准

| 指标 | 目标值 |
|------|--------|
| OPC UA 连接 | 可连接标准 OPC UA Server（Ignition / KEPServerEX），X.509 证书认证 |
| 订阅容量 | 支持 1,000+ Monitored Item |
| 写入性能 | 1,000 点/秒持续 24 小时不丢数据 |
| 查询性能 | tag_id + 时间范围查询 100 万点 P99 < 200ms |
| 规则引擎 | 支持阈值规则、时间窗口规则、复合规则 + 告警抑制 |
| WebSocket | 前端订阅 Tag 后 1 秒内开始接收数据，≥ 50 并发连接 |
| 重连恢复 | 连接异常自动重连，重连后订阅自动恢复 |
| 配置热重载 | 订阅配置（采样间隔、队列大小、死区）TOML 热重载 |

### 4.3 任务清单

| # | 任务 | 工时 | 依赖 | 需求 |
|---|------|------|------|------|
| B-01 | 引入 `opcua` crate，建立 `opcua-client/` 模块骨架 | 1d | 无 | REQ-01 |
| B-02 | 实现 OPC UA 客户端连接管理：会话建立、X.509 证书认证、自动重连 | 3d | B-01 | REQ-01 |
| B-03 | 实现订阅管理：创建 Subscription、添加 Monitored Item、数据变更回调 | 3d | B-02 | REQ-01 |
| B-04 | 定义 Tokio broadcast channel 数据结构 `TagUpdate { tenant_id, tag_id, timestamp_ns, value, quality }` | 1d | B-01 | REQ-01/07 |
| B-05 | 将 OPC UA 数据变更事件桥接到 broadcast channel | 1d | B-03, B-04 | REQ-01 |
| B-06 | 新增 `storage/` 模块，定义 `TimeSeriesStore` trait | 1d | 无 | REQ-02 |
| B-07 | 实现 SQLite + WAL 时序存储：建表、写入、查询 | 3d | B-06 | REQ-02 |
| B-08 | 实现数据保留策略（按时间或容量滚动删除） | 1d | B-07 | REQ-02 |
| B-09 | 实现写入性能压测（1,000 点/秒 × 24 小时） | 1d | B-07 | REQ-02 |
| B-10 | 新增 `device/` 模块，定义 ISA-95 层次数据模型（Site / Device / Tag） | 2d | 无 | REQ-05 |
| B-11 | 实现 Tag 关联 OPC UA NodeId + CSV 批量导入点位表 | 1.5d | B-10 | REQ-05 |
| B-12 | 新增 `rules/` 模块，定义 `Rule { condition, action, window, cooldown }` 结构 | 1d | 无 | REQ-03 |
| B-13 | 引入 `expr` crate，实现表达式求值引擎 | 1.5d | B-12 | REQ-03 |
| B-14 | 实现规则类型：阈值规则、时间窗口规则、复合规则 | 2d | B-13 | REQ-03 |
| B-15 | 实现告警抑制（同规则 N 分钟内只触发一次）+ 分级（INFO/WARN/CRITICAL） | 1d | B-14 | REQ-03 |
| B-16 | 规则触发后调用现有 `notification/`（Webhook + SMTP） | 0.5d | B-15 | REQ-03 |
| B-17 | 规则配置 TOML 热重载 | 0.5d | B-15 | REQ-03 |
| B-18 | broadcast channel 消费者：规则引擎订阅 TagUpdate 触发求值 | 1d | B-05, B-14 | REQ-03 |
| B-19 | broadcast channel 消费者：SQLite 存储订阅 TagUpdate 异步写入 | 1d | B-05, B-07 | REQ-02 |
| B-20 | Axum 新增 `/ws` WebSocket 端点，前端订阅指定 Tag 实时推送 | 2d | B-05 | REQ-07 |
| B-21 | WebSocket 订阅过滤（按 tag、按变化阈值）+ 断线自动重连 | 1d | B-20 | REQ-07 |
| B-22 | OPC UA 客户端配置集成到 TOML（endpoint、security_policy、cert_path） | 1d | B-02 | REQ-01 |
| B-23 | 端到端集成测试：OPC UA Server（模拟）→ ModelSwitch → SQLite + WebSocket | 2d | B-19, B-20 | ALL |

**总工时估算**：约 33d（含 20% 调试 buffer = 约 40d = 6 周，含 `opcua` crate 学习曲线 3-5 天）。

### 4.4 依赖关系图

```
B-01 ──► B-02 ──► B-03 ──► B-05 ──┬──► B-18 ──► B-23
                                   ├──► B-19 ──► B-23
                                   └──► B-20 ──► B-21 ──► B-23
B-04 ──────────────────────────────┘
B-06 ──► B-07 ──► B-08             │
                   B-09             │
B-10 ──► B-11                      │
B-12 ──► B-13 ──► B-14 ──► B-15 ──► B-16
                            ──► B-17
B-22 (并行)
```

### 4.5 技术方案

**OPC UA 客户端**：`opcua` crate（locka99/opcua）是纯 Rust + Tokio 实现，与 ModelSwitch 现有依赖零冲突。客户端模块独立于 LLM 网关，通过 `spawn_bg()` 启动会话保活任务。重连逻辑用指数退避（1s → 2s → 4s → ... → 60s 封顶）。

**SQLite 时序存储**：
```sql
CREATE TABLE tag_data (
    tenant_id  TEXT    NOT NULL,
    tag_id     TEXT    NOT NULL,
    ts_ns      INTEGER NOT NULL,
    value      REAL,
    quality    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_tag_time ON tag_data(tenant_id, tag_id, ts_ns DESC);
```
WAL 模式 + `PRAGMA synchronous = NORMAL`，平衡持久性与写入吞吐。保留策略用定时任务执行 `DELETE FROM tag_data WHERE ts_ns < ?`。

**规则引擎**：`expr` crate 编译表达式为 AST，求值时绑定变量（`value`, `prev_value`, `duration`, `tag.quality`）。规则求值挂在 TagUpdate 事件上，触发后通过 `notification/` 分发。不引入 BPMN / Drools 等重型框架。

**WebSocket 端点**：Axum 已启用 `ws` feature，直接新增路由。前端连接后发送订阅消息 `{ tags: ["tag1", "tag2"], threshold: 0.1 }`，服务端创建 broadcast channel Receiver 转发。

### 4.6 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| `opcua` crate 学习曲线陡峭，订阅 / Monitored Item 生命周期管理复杂 | 高 | 高 | 预留 3-5 天学习曲线；先跑通官方 example 再集成 |
| SQLite 写入性能在 1,000 点/秒下可能瓶颈 | 中 | 中 | 批量写入（每 100 条或 100ms flush 一次）+ prepared statement |
| broadcast channel 背压：慢消费者拖慢整个管道 | 中 | 中 | channel capacity 设为有限值（如 10,000），满时丢弃最旧数据点 + 计数告警 |
| OPC UA Server 品牌差异（Ignition vs KEP vs 自研）行为不一致 | 中 | 中 | 至少测试 2 种 Server（Ignition + 开源 OPC UA Server demo） |

### 4.7 资源需求

| 资源 | 说明 |
|------|------|
| OPC UA 测试 Server | Ignition 试用版（免费 2 小时重置）或开源 `opcua` sample-server |
| 工控机或 VM | 4 核 8GB，用于跑端到端测试 |
| 工时 | 6 人周 |

---

## 5 阶段 C：智能运维闭环（6 周）

### 5.1 阶段目标

在数据平面 MVP 之上，构建 AI 驱动的智能运维闭环：告警智能分级、报表自动生成、RAG 知识库、多渠道通知。实现"数据 → 告警 → AI 分析 → 处置建议 → 通知推送"的全链路。

### 5.2 验收标准

| 指标 | 目标值 |
|------|--------|
| AI 告警分析延迟 | 告警触发后 30 秒内生成 AI 分析建议 |
| AI 分析内容 | 包含"根因推测 + 处置建议 + 置信度" |
| LLM 熔断降级 | 所有 LLM 渠道熔断时降级为只发原始告警，不阻塞主链路 |
| 报表生成 | 支持 cron 定时生成日报/周报/月报，含 KPI 摘要 + 告警列表 + AI 总结 |
| RAG 检索 | Top-K 检索 P95 < 500ms，新增文档后 5 分钟内可检索 |
| 通知渠道 | Webhook + SMTP + 钉钉机器人 + 企业微信机器人 |
| 前端仪表盘 | 设备拓扑图 + 实时曲线（刷新 ≤ 1 秒）+ 告警列表 |
| 移动端 | 核心面板（Dashboard / 告警列表）≥ 375px 宽度可用 |

### 5.3 任务清单

| # | 任务 | 工时 | 依赖 | 需求 |
|---|------|------|------|------|
| C-01 | 封装 OPC UA 查询能力为 MCP Server（Python / Node.js），暴露 `read_tag`、`browse_nodes`、`history_read` 工具 | 3d | 阶段 B | REQ-04 |
| C-02 | 告警触发后构造 prompt（告警上下文 + 历史趋势 + 相关 SOP 片段），通过 ModelSwitch 多模型调度调用 LLM | 2d | C-01 | REQ-04 |
| C-03 | 实现 MCP 自动注入循环（复用现有 5 轮上限）：LLM 调用 `read_tag` 查证 → 生成根因建议 | 2d | C-02 | REQ-04 |
| C-04 | AI 分析结果写入审计日志 + 降级策略（LLM 全熔断时只发原始告警） | 1d | C-03 | REQ-04 |
| C-05 | 按客户/站点配置是否启用 AI 分析（TOML + 热重载） | 0.5d | C-04 | REQ-04 |
| C-06 | 新增 `report/` 模块，定义报表模板结构（KPI 摘要 / 告警列表 / 趋势图 / AI 总结） | 1d | 无 | REQ-12 |
| C-07 | 实现 cron 定时任务调度器 | 1d | C-06 | REQ-12 |
| C-08 | 从 SQLite 时序数据聚合报表数据（日/周/月维度） | 2d | C-06 | REQ-12 |
| C-09 | LLM 生成报表 AI 总结段落（自然语言摘要） | 1d | C-08 | REQ-12 |
| C-10 | 报表 Markdown 输出 + PDF 导出（用 `printpdf` 或 `wkhtmltopdf` 子进程） | 1.5d | C-09 | REQ-12 |
| C-11 | 报表按租户导出 + AI 总结段落支持人工编辑后定稿 | 1d | C-10 | REQ-12 |
| C-12 | 新增 `rag/` 模块，定义文档管理结构（上传、分块、向量化） | 1d | 无 | REQ-13 |
| C-13 | 复用 ModelSwitch Embeddings 代理，将文档分块向量化 | 1.5d | C-12 | REQ-13 |
| C-14 | SQLite 存储向量 + 余弦相似度检索（先用纯 SQLite 计算，数据量大时迁移 sqlite-vss） | 2d | C-13 | REQ-13 |
| C-15 | RAG 查询接口：自然语言提问 → 向量检索 Top-K → LLM 生成回答 + 引用来源 | 2d | C-14 | REQ-13 |
| C-16 | 文档按租户隔离 + 支持上传 PDF/Markdown/Word | 1d | C-12 | REQ-13 |
| C-17 | 新增钉钉机器人通知渠道（`notification/dingtalk.rs`） | 1d | 无 | REQ-17 |
| C-18 | 新增企业微信机器人通知渠道（`notification/wework.rs`） | 1d | 无 | REQ-17 |
| C-19 | 通知多渠道并发分发 + 级别过滤 + 失败重试 3 次降级 | 1d | C-17, C-18 | REQ-17 |
| C-20 | 前端：新增 `src/components/opc/` 目录骨架 | 0.5d | 无 | REQ-09 |
| C-21 | 前端：设备拓扑图组件（React Flow，ISA-95 层次展示） | 3d | C-20 | REQ-09 |
| C-22 | 前端：实时曲线图组件（ECharts，WebSocket 数据源，刷新 ≤ 1 秒） | 3d | C-20, B-20 | REQ-09 |
| C-23 | 前端：告警列表组件（按时间/级别排序，可确认/关闭） | 2d | C-20 | REQ-09 |
| C-24 | 前端：按租户切换视图 | 1d | C-21 | REQ-09 |
| C-25 | 前端：核心面板移动端适配（≥ 375px：Dashboard / 告警列表 / 状态栏） | 2d | C-23 | REQ-09 |
| C-26 | 多租户数据隔离强化：storage 层引入 `tenant_id`，所有查询带过滤 | 2d | 阶段 B | REQ-06 |
| C-27 | 跨租户访问返回 403 + 审计日志按租户独立导出 | 1d | C-26 | REQ-06 |

**总工时估算**：约 40d（含 20% 调试 buffer = 约 48d = 6 周，含 MCP Server 开发与调试）。

### 5.4 技术方案

**OPC UA MCP Server**：用 Python 实现（`asyncua` 库），封装为独立子进程。工具列表：
- `read_tag(tag_id)` → 返回当前值 + 质量 + 时间戳
- `history_read(tag_id, start, end)` → 返回历史数据
- `browse_nodes(node_id)` → 返回子节点列表

ModelSwitch 通过现有 MCP 管理器拉起此子进程，LLM 在自动注入循环中按需调用。这实现了"AI 分析告警 → 调用工具查证设备状态 → 生成根因建议"的闭环。

**RAG 方案**：文档分块用 sliding window（512 tokens，128 overlap）。向量化复用 ModelSwitch Embeddings 代理（支持 OpenAI / 国产模型）。向量存储用 SQLite 表 `doc_chunks(id, tenant_id, doc_id, chunk_index, embedding_blob, text)`，检索时全表扫描计算余弦相似度（单租户文档量 < 10,000 chunks 时 P95 < 500ms）。文档量增大后可平滑迁移到 sqlite-vss 扩展。

**前端图表选型**：ECharts。理由：
- 实时曲线场景需要高性能增量渲染（ECharts `appendData` API 原生支持）
- 中国工业领域 ECharts 知名度最高，客户二次开发门槛低
- Tree-shaking 后打包体积可控（只引 LineChart + GridComponent 约 200KB）

**报表 PDF 生成**：优先用 `wkhtmltopdf` 子进程（Markdown → HTML → PDF），避免引入重量级 Rust PDF 库。如果部署环境无 `wkhtmltopdf`，fallback 为 Markdown 导出。

### 5.5 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| MCP Server 子进程不稳定影响 AI 分析链路 | 中 | 中 | 复用 ModelSwitch 现有 MCP 健康探测 + 重启；AI 分析有降级策略兜底 |
| RAG 向量检索全表扫描在数据量大时性能退化 | 中 | 中 | 设定阈值（单租户 > 5,000 chunks），超阈值提示迁移 sqlite-vss |
| LLM 幻觉导致 AI 分析建议误导工程师 | 中 | 高 | AI 建议标注"仅供参考"；控制指令禁止 LLM 直接下发（合规红线） |
| 前端组件开发工时超预期 | 中 | 中 | React Flow / ECharts 有成熟示例，先抄后改 |

### 5.6 资源需求

| 资源 | 说明 | 费用 |
|------|------|------|
| LLM API | AI 告警分析 + 报表生成 + RAG 问答 | 约 200-500 元/月/客户（GPT-4o-mini / DeepSeek） |
| 工时 | 6 人周 | — |
| `wkhtmltopdf` | 报表 PDF 导出 | 免费 |

---

## 6 阶段 D：合规硬化与规模化（4 周）

### 6.1 阶段目标

在功能闭环基础上，完成工业合规准入硬化，使 ModelSwitch 满足 IEC 62443 SL2 基础 + 等保 2.0 工控扩展的基本要求。同时建立控制指令安全管道，支撑远程运维场景。

### 6.2 验收标准

| 指标 | 目标值 |
|------|--------|
| OPC UA 安全策略 | 支持 None / Basic128Rsa15 / Basic256 / Basic256Sha256 四种策略 |
| 证书管理 | PEM 文件加载 + 客户端证书自签名 + 过期前 30 天告警 |
| 控制指令 | "申请 → 审批 → 执行 → 审计"四步流程，高危指令强制双人确认 |
| 审计字段 | 含操作人 / 租户 / 设备 / 点位 / 指令类型 / 执行结果 / 来源 IP |
| 审计防篡改 | 哈希链 + 周期性外部锚点签名 |
| 数据分类分级 | 元数据字段标注数据级别（一般 / 重要 / 敏感） |
| 灾备演练 | 备份恢复流程有实际演练记录 + 自动化脚本 |

### 6.3 任务清单

| # | 任务 | 工时 | 依赖 | 需求 |
|---|------|------|------|------|
| D-01 | 实现 OPC UA 安全策略配置（四种策略 + TOML 配置） | 2d | 阶段 B | REQ-11 |
| D-02 | X.509 证书管理：PEM 加载、自签名生成（`rcgen`）、CA 签名支持 | 2d | D-01 | REQ-11 |
| D-03 | 证书过期前 30 天告警 + 安全世界事件全部写入审计日志 | 1d | D-02 | REQ-11 |
| D-04 | 新增 `control/` 模块骨架，定义控制指令数据结构 | 1d | 无 | REQ-08 |
| D-05 | 实现控制指令四步流程：创建申请单（who/what/why）→ 审批（RBAC）→ 执行（OPC UA Method Call / Write）→ 审计 | 3d | D-04 | REQ-08 |
| D-06 | 高危指令强制双人确认（操作人 + 批准人不同账号） | 1d | D-05 | REQ-08 |
| D-07 | 指令执行超时与结果审计写入 | 1d | D-05 | REQ-08 |
| D-08 | 审计日志扩展工业字段：操作人 / 租户 / 设备 / 点位 / 指令类型 / 执行结果 / 来源 IP | 1.5d | 阶段 A | REQ-15 |
| D-09 | 审计日志按租户独立导出（CSV / JSON）+ 按字段检索 | 1d | D-08 | REQ-15 |
| D-10 | 审计哈希链周期性外部锚点签名（写入独立签名文件） | 1d | D-08 | REQ-15 |
| D-11 | 数据分类分级：元数据字段（`data_class: general / important / sensitive`） | 1d | 无 | 合规 |
| D-12 | 敏感数据 LLM 请求出境标记 + 脱敏前置（复用 Sanitizer） | 1.5d | D-11 | 合规 |
| D-13 | 备份恢复演练自动化脚本（备份 → 模拟故障 → 恢复 → 验证） | 1d | 无 | REQ-18 |
| D-14 | 备份内容含配置 + 数据库 + 证书 + 加密（AES-256） | 1d | D-13 | REQ-18 |
| D-15 | 等保 2.0 三权分立角色扩展（系统管理员 / 安全管理员 / 审计管理员） | 1d | 无 | 合规 |
| D-16 | 合规自查清单文档 + IEC 62443 SL2 控制项对照表 | 1d | ALL | 合规 |

**总工时估算**：约 20d（含 20% 调试 buffer = 约 24d = 4 周）。

### 6.4 技术方案

**OPC UA 安全策略**：`opcua` crate 原生支持 Security Policy 配置。证书管理用 `rcgen`（纯 Rust）生成自签名证书，CA 签名走 OpenSSL CLI 子进程（不引入 `openssl` crate 编译依赖）。证书文件路径通过 TOML 配置。

**控制指令安全管道**：
```
创建申请单（operator）
    │
    ▼
审批（approver，RBAC 校验）
    │  高危指令：操作人 ≠ 批准人
    ▼
执行（OPC UA Method Call / Write Service）
    │  超时 30s
    ▼
审计（结果 + 全字段写入审计日志）
```

LLM **不允许**直接进入此流程。AI 分析建议中可包含"建议下发 XX 指令"，但必须由人工创建申请单。

**数据分类分级**：在 `Tag` 和 `Device` 结构体中增加 `data_class` 字段。LLM 请求前检查涉及的数据级别：`sensitive` 数据强制经过 Sanitizer 脱敏后才允许发送到 LLM API。

### 6.5 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| OPC UA 安全策略实现与真实 Server 互通失败 | 中 | 高 | 至少测试 Ignition + KEPServerEX 两种 Server 的证书认证 |
| 控制指令执行涉及功能安全 | 低 | 极高 | 强制四步流程 + 双人确认；初期只支持只读类指令（如参数读取），写指令需客户单独签署授权 |
| 合规标准理解偏差导致实现不达标 | 中 | 中 | 合规自查清单与标准条款逐条对照；必要时请外部合规顾问 review |

### 6.6 资源需求

| 资源 | 说明 | 费用 |
|------|------|------|
| Ignition 授权 | 用于 OPC UA 安全测试 | ~$3K/年（无限点数） |
| 合规顾问（可选） | 等保 / IEC 62443 自查 review | 5,000-15,000 元（一次性） |
| 工时 | 4 人周 | — |

---

## 7 技术路线决策

### 决策 1：OPC UA 实现

| 选项 | 评价 | 决策 |
|------|------|------|
| `opcua` crate（Rust 原生） | 纯 Rust + Tokio，零 C 依赖，与 ModelSwitch 依赖栈零冲突，社区活跃 | **选择** |
| `open62541` FFI | 性能更高，但引入 C 编译依赖，破坏纯 Rust 构建 | 否决 |
| 外部 OPC UA MCP Server | 适合 LLM 查询场景，但不适合订阅模式（长连接推送） | 阶段 C 辅助使用（AI 查询通道） |

**Rationale**：`opcua` crate 是 Rust 生态唯一的成熟 OPC UA 实现，与 ModelSwitch 的 Tokio full feature + rustls 技术栈天然兼容。订阅模式必须走 Rust 原生模块，不能依赖 MCP 子进程的请求-响应模型。

### 决策 2：时序存储

| 选项 | 评价 | 决策 |
|------|------|------|
| SQLite + WAL + 时间索引 | 零运维成本，单文件，崩溃不丢，一人公司最优 | **选择** |
| InfluxDB | 专用 TSDB，性能更强，但引入额外服务依赖 | 否决（违反"不引入专职运维依赖"约束） |
| TimescaleDB | PostgreSQL 扩展，功能强但需 PG 运维 | 否决（同上） |

**Rationale**：一人公司的核心约束是"不引入需要专职运维的外部依赖"。SQLite 在 1,000 点/秒 + 单库 10GB 规模下完全够用。数据量增长后可平滑迁移到 TimescaleDB，因为 `TimeSeriesStore` trait 抽象了存储层。

### 决策 3：规则引擎

| 选项 | 评价 | 决策 |
|------|------|------|
| `expr` crate（表达式引擎） | 轻量，编译表达式为 AST，求值快速，足以覆盖阈值 / 窗口 / 复合规则 | **选择** |
| `cel-interpreter`（Google CEL） | 功能更强，但 Rust 实现成熟度不如 `expr` | 备选 |
| 自研 | 完全可控，但投入产出比差 | 否决 |

**Rationale**：工业告警规则的核心模式是"值比较 + 时间窗口 + 逻辑组合"，`expr` crate 的表达式求值完全覆盖。不引入 BPMN / Drools 等重型规则框架。

### 决策 4：实时数据流

| 选项 | 评价 | 决策 |
|------|------|------|
| WebSocket（Axum 内置） | 双向，Axum 已启用 `ws` feature，零新依赖，前端原生支持 | **选择** |
| MQTT（`rumqttc`） | 工业标准，但需引入 MQTT broker 或自建 | 否决（增加运维负担） |
| SSE 复用 | 单向，不适合双向通信 | 否决（SSE 留给 LLM 流式输出） |

**Rationale**：WebSocket 在 Axum 上零额外依赖。数据平面内部用 Tokio broadcast channel，前端接入用 WebSocket 转发。MQTT 仅在需要与外部 MQTT 设备直连时考虑（目前不存在此需求）。

### 决策 5：前端图表

| 选项 | 评价 | 决策 |
|------|------|------|
| ECharts | 实时增量渲染（`appendData`），中国工业领域知名度最高，tree-shaking 后约 200KB | **选择** |
| Uplot | 极致性能（百万点），但 API 低级、中文文档少 | 备选（超大点数场景） |
| Recharts | React 原生，但性能弱于 ECharts，不适合实时高频更新 | 否决 |

**Rationale**：实时曲线刷新间隔 ≤ 1 秒，ECharts 增量渲染 API 完全满足。工业客户对 ECharts 认知度高，降低客户二次开发门槛。

### 决策 6：RAG 方案

| 选项 | 评价 | 决策 |
|------|------|------|
| SQLite + 向量列（纯计算） | 零额外依赖，单租户 < 10,000 chunks 时 P95 < 500ms | **选择（先期）** |
| SQLite-VSS 扩展 | ANN 索引，性能更好，但需加载扩展（部署多一步） | **选择（超阈值后迁移）** |
| Qdrant 外部服务 | 专业向量库，但引入额外服务依赖 | 否决（违反一人公司约束） |

**Rationale**：先跑通 SQLite 全表扫描，数据量超阈值时迁移到 sqlite-vss。`TimeSeriesStore` 和 RAG 查询接口都做了抽象，迁移成本可控。不引入 Qdrant / Milvus 等外部向量库。

### 决策 7：部署形态

| 选项 | 定位 | 决策 |
|------|------|------|
| CLI + systemd | **生产形态**（工控机 headless Linux） | **主推** |
| Docker Compose | 便捷部署（开发 / 测试 / 小规模生产） | 保留 |
| Tauri 桌面端 | **调试工具**（工程师站可视化调试） | 降级定位 |

**Rationale**：工控机普遍运行 headless Linux，Tauri 桌面端依赖 GUI 无法适应。CLI + systemd 已有 `deploy/install-systemd.sh` 脚本，安全加固到位。Tauri 从"主形态"降级为"调试辅助工具"。

---

## 8 资源投入计划

| 资源 | 阶段 A | 阶段 B | 阶段 C | 阶段 D | 备注 |
|------|--------|--------|--------|--------|------|
| 工时（人周） | 4 | 6 | 6 | 4 | 含开发 + 测试 + 调试，总计 20 人周 |
| 外部授权 | 0 | 0 | 0 | Ignition ~$3K/年 | Ignition 用于 OPC UA 安全测试 |
| 云服务 / API | 0 | 0 | 200-500 元/月/客户 | 0 | LLM API（GPT-4o-mini / DeepSeek），通过虚拟密钥配额管控 |
| 硬件 | 0 | 0 | 0 | 0 | 利用现有开发机 + VM |
| 合规顾问 | 0 | 0 | 0 | 5K-15K 元（可选） | 等保自查 review，一次性 |

**总计年化运营成本**（3 家 retainer 客户）：
- LLM API: 600-1,500 元/月 = 7,200-18,000 元/年
- Ignition 授权: ~21,000 元/年
- 合规顾问: 5,000-15,000 元（一次性）
- **年化总计**: 约 33,000-54,000 元（远低于 5 万元授权预算上限）

---

## 9 优先级矩阵

### 18 条需求 × 4 阶段映射

| 需求编号 | 需求名称 | 优先级 | 落地阶段 | 工时（天） |
|---------|---------|--------|---------|-----------|
| REQ-01 | OPC UA 客户端订阅 | P0 | 阶段 B | 8 |
| REQ-02 | 时序数据持久化层 | P0 | 阶段 B | 6 |
| REQ-03 | 规则与告警引擎 | P0 | 阶段 B | 6 |
| REQ-04 | 告警智能分级（LLM 分析） | P0 | 阶段 C | 8.5 |
| REQ-05 | 设备与点位数据模型 | P0 | 阶段 B | 3.5 |
| REQ-06 | 多客户/多站点数据隔离 | P0 | 阶段 B+C | 3 |
| REQ-07 | 实时数据流通道（WebSocket） | P0 | 阶段 B | 3 |
| REQ-08 | 控制指令安全管道 | P1 | 阶段 D | 6 |
| REQ-09 | 工业级监控仪表盘 | P1 | 阶段 C | 11.5 |
| REQ-10 | 边缘离线运行与数据缓冲 | P1 | 阶段 B（隐含） | 2 |
| REQ-11 | 工业安全合规（OPC UA Security） | P1 | 阶段 D | 5 |
| REQ-12 | 运维报表自动生成 | P1 | 阶段 C | 6.5 |
| REQ-13 | RAG 现场知识库 | P1 | 阶段 C | 7.5 |
| REQ-14 | 协议转换辅助 | P2 | 未排期 | — |
| REQ-15 | 合规审计强化 | P1 | 阶段 A+D | 4.5 |
| REQ-16 | 投标/技术方案撰写辅助 | P2 | 未排期 | — |
| REQ-17 | 通知渠道扩展 | P1 | 阶段 C | 3 |
| REQ-18 | 备份与灾难恢复 | P2 | 阶段 D | 2 |

**未排期需求说明**：REQ-14（协议转换辅助）和 REQ-16（投标方案撰写辅助）为 P2 级，ROI 较低且非核心路径。建议在阶段 D 完成后根据客户实际需求再评估是否纳入。

---

## 10 关键里程碑与检查点

### 里程碑 M1：基座加固完成（第 4 周末）

| 项目 | 内容 |
|------|------|
| 预计完成 | 第 4 周末 |
| 验收条件 | 后端 P1 问题清零，审计日志持久化，安全头补全，KDF 强化，前端测试 ≥ 40% |
| Go/No-Go | **Go 条件**：审计日志 1,000 条上限已移除 + KDF 已强化。**No-Go 则**：阶段 B 推迟，继续修复基座 |

### 里程碑 M2：首个 OPC UA 数据端到端打通（第 8 周末）

| 项目 | 内容 |
|------|------|
| 预计完成 | 第 8 周末（阶段 B 结束） |
| 验收条件 | OPC UA Server → ModelSwitch → SQLite + WebSocket 实时推送，规则引擎触发告警 |
| Go/No-Go | **Go 条件**：能从 Ignition / KEPServerEX 采集 ≥ 100 点位并稳定运行 24 小时。**No-Go 则**：排查 OPC UA 连接稳定性，阶段 C 推迟 1-2 周 |

### 里程碑 M3：AI 分析闭环验证（第 11 周末）

| 项目 | 内容 |
|------|------|
| 预计完成 | 第 11 周末（阶段 C 中期） |
| 验收条件 | 告警触发后 LLM 在 30 秒内生成分析建议，MCP 工具注入闭环跑通 |
| Go/No-Go | **Go 条件**：AI 分析建议质量可接受（根因推测合理率 ≥ 70%）。**No-Go 则**：优化 prompt 工程，或降级为规则模板 + LLM 摘要 |

### 里程碑 M4：智能运维闭环交付（第 14 周末）

| 项目 | 内容 |
|------|------|
| 预计完成 | 第 14 周末（阶段 C 结束） |
| 验收条件 | 报表自动生成 + RAG 知识库 + 多渠道通知 + 前端仪表盘全部可用 |
| Go/No-Go | **Go 条件**：可向 retainer 客户演示完整工作流。**No-Go 则**：砍掉 RAG（阶段 C 最复杂部分），阶段 D 提前 |

### 里程碑 M5：合规硬化完成（第 18 周末）

| 项目 | 内容 |
|------|------|
| 预计完成 | 第 18 周末（阶段 D 结束） |
| 验收条件 | OPC UA 安全策略完整、控制指令四步流程、审计合规、灾备演练通过 |
| Go/No-Go | **Go 条件**：IEC 62443 SL2 自查清单通过。**No-Go 则**：合规风险较高的项目暂不承接，继续完善 |

### 里程碑 M6：首个 retainer 客户试点上线（第 20 周末）

| 项目 | 内容 |
|------|------|
| 预计完成 | 第 20 周末 |
| 验收条件 | 选定一家低风险 retainer 客户试点部署，稳定运行 1 周 |
| Go/No-Go | **Go 条件**：数据采集无丢点 + 告警通知无遗漏 + 客户认可 AI 分析价值 |

---

## 11 回退与降级策略

### 阶段 A 降级

| 优先级 | 任务 | 降级处理 |
|--------|------|----------|
| Must | A-01 审计日志持久化 | 不降级，是合规硬需求 |
| Must | A-02 KDF 强化 | 不降级，安全基线 |
| Should | A-10 前端测试 40% | 降到 30%，核心组件优先 |
| Nice | A-11 translate.rs 拆分 | 延后到阶段 B 空闲期 |

### 阶段 B 降级

| 优先级 | 任务 | 降级处理 |
|--------|------|----------|
| Must | B-01~05 OPC UA 客户端 | 不降级，是阶段 B 核心 |
| Must | B-06~09 SQLite 时序存储 | 不降级 |
| Must | B-12~18 规则引擎 | 先只做阈值规则，时间窗口/复合规则延后 |
| Should | B-20~21 WebSocket | 降级为 SSE 单向推送（复用现有 `proxy/stream.rs`） |
| Nice | B-08 数据保留策略 | 延后，手动清理即可 |

### 阶段 C 降级

| 优先级 | 任务 | 降级处理 |
|--------|------|----------|
| Must | C-01~05 AI 告警分析 | 不降级，是核心差异化 |
| Must | C-17~19 多渠道通知 | 不降级，工业场景必需 |
| Should | C-06~11 报表自动化 | 先只做 Markdown 格式，PDF 延后 |
| Should | C-20~25 前端仪表盘 | 先只做告警列表 + 实时曲线，设备拓扑延后 |
| Nice | C-12~16 RAG 知识库 | 整体延后到阶段 D 之后，先跑通 AI 分析 + 报表 |

### 阶段 D 降级

| 优先级 | 任务 | 降级处理 |
|--------|------|----------|
| Must | D-01~03 OPC UA 安全策略 | 不降级，合规准入 |
| Must | D-08~10 审计强化 | 不降级 |
| Should | D-04~07 控制指令管道 | 先只支持只读类指令（参数读取），写指令延后 |
| Nice | D-15 三权分立 | 延后，当前 RBAC 三角色（SuperAdmin/KeyManager/Auditor）基本够用 |

### 整体降级原则

1. **合规相关不可降级**：审计日志、安全策略、数据隔离
2. **AI 核心价值不可降级**：告警智能分级是多模型调度 + MCP 注入的最佳应用场景
3. **先跑通再完善**：MVP 优先于完美，每个阶段先交付 must-have，should/nice 视工时增减
4. **RAG 是最大降级点**：如果工时紧张，RAG 知识库（阶段 C 最复杂的部分）可以整体延后，因为它不影响核心数据流和告警链路

---

## 附录 A：OPC UA MCP Server 工具定义（阶段 C 参考）

```json
{
  "tools": [
    {
      "name": "read_tag",
      "description": "Read current value of an OPC UA tag",
      "inputSchema": {
        "type": "object",
        "properties": {
          "tag_id": { "type": "string", "description": "Tag identifier" }
        },
        "required": ["tag_id"]
      }
    },
    {
      "name": "history_read",
      "description": "Read historical values of a tag within a time range",
      "inputSchema": {
        "type": "object",
        "properties": {
          "tag_id": { "type": "string" },
          "start": { "type": "string", "format": "date-time" },
          "end": { "type": "string", "format": "date-time" }
        },
        "required": ["tag_id", "start", "end"]
      }
    },
    {
      "name": "browse_nodes",
      "description": "Browse child nodes of an OPC UA node",
      "inputSchema": {
        "type": "object",
        "properties": {
          "node_id": { "type": "string" }
        },
        "required": ["node_id"]
      }
    }
  ]
}
```

---

## 附录 B：SQLite 时序存储建表参考（阶段 B 参考）

```sql
-- 时序数据主表
CREATE TABLE IF NOT EXISTS tag_data (
    tenant_id  TEXT    NOT NULL,
    tag_id     TEXT    NOT NULL,
    ts_ns      INTEGER NOT NULL,
    value      REAL,
    str_value  TEXT,
    quality    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, tag_id, ts_ns)
) WITHOUT ROWID;

-- 按时间倒序索引（查询最常用路径）
CREATE INDEX IF NOT EXISTS idx_tag_time
    ON tag_data(tenant_id, tag_id, ts_ns DESC);

-- 设备表
CREATE TABLE IF NOT EXISTS devices (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    site_id     TEXT,
    name        TEXT NOT NULL,
    opcua_node_id TEXT,
    created_at  INTEGER NOT NULL
);

-- 点位表
CREATE TABLE IF NOT EXISTS tags (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    device_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    opcua_node_id TEXT NOT NULL,
    unit        TEXT,
    min_value   REAL,
    max_value   REAL,
    data_class  TEXT DEFAULT 'general',
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (device_id) REFERENCES devices(id)
);

-- 告警表
CREATE TABLE IF NOT EXISTS alarms (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    rule_id     TEXT NOT NULL,
    tag_id      TEXT,
    level       TEXT NOT NULL,  -- INFO / WARN / CRITICAL
    message     TEXT NOT NULL,
    value       REAL,
    triggered_at INTEGER NOT NULL,
    acknowledged_at INTEGER,
    acknowledged_by TEXT,
    ai_analysis TEXT,            -- AI 分析建议 JSON
    created_at  INTEGER NOT NULL
);

-- RAG 文档块表（阶段 C）
CREATE TABLE IF NOT EXISTS doc_chunks (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    doc_id      TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    text        TEXT NOT NULL,
    embedding   BLOB,            -- f32 数组序列化
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_doc_tenant
    ON doc_chunks(tenant_id, doc_id);
```

---

## 附录 C：关键 TOML 配置扩展参考（阶段 B-D 参考）

```toml
# OPC UA 客户端配置
[opcua]
enabled = true
endpoint_url = "opc.tcp://192.168.1.100:4840"
security_policy = "Basic256Sha256"  # None / Basic128Rsa15 / Basic256 / Basic256Sha256
cert_path = "./certs/client.pem"
private_key_path = "./certs/client.key"
auto_reconnect = true
reconnect_interval_ms = 5000

# 订阅配置
[opcua.subscription]
publishing_interval_ms = 500
lifetime_count = 10000
max_keep_alive_count = 3000

# 设备点位配置（支持热重载）
[[opcua.devices]]
id = "device-001"
tenant_id = "customer-a"
name = "产线 1 - PLC"
site_id = "factory-shanghai"

  [[opcua.devices.tags]]
  id = "temp_001"
  name = "主轴温度"
  opcua_node_id = "ns=2;s=Temperature.MainSpindle"
  unit = "°C"
  min_value = 0
  max_value = 150
  data_class = "important"

  [[opcua.devices.tags]]
  id = "vibration_001"
  name = "主轴振动"
  opcua_node_id = "ns=2;s=Vibration.MainSpindle"
  unit = "mm/s"
  data_class = "general"

# 规则配置
[[rules]]
id = "rule-temp-high"
name = "主轴温度过高"
tag_id = "temp_001"
condition = "value > 80"
window_secs = 30
cooldown_mins = 15
level = "CRITICAL"
actions = ["webhook", "dingtalk", "ai_analysis"]

[[rules]]
id = "rule-temp-warning"
name = "主轴温度预警"
tag_id = "temp_001"
condition = "value > 65 && value <= 80"
window_secs = 60
cooldown_mins = 30
level = "WARN"
actions = ["dingtalk"]

# 通知配置
[notification.dingtalk]
enabled = true
webhook_url = "https://oapi.dingtalk.com/robot/send?access_token=xxx"
secret = "xxx"
level_filter = ["WARN", "CRITICAL"]

[notification.wework]
enabled = true
webhook_url = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx"
level_filter = ["CRITICAL"]

# AI 分析配置
[ai_analysis]
enabled = true
trigger_levels = ["WARN", "CRITICAL"]
timeout_secs = 30
fallback_on_all_circuit_open = true  # LLM 全熔断时降级为原始告警
```

---

*方案结束。本方案基于 6 份输入材料编制，所有技术决策均有一人公司运营约束的 rationale 支撑。建议按阶段顺序执行，每个阶段完成后根据里程碑 Go/No-Go 决策调整后续计划。*
