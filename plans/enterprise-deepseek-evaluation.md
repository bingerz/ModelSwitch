# ModelSwitch 企业场景评估 — 1000 员工 DeepSeek Token 分发

## Context

**场景**: 传统公司老板在公司一台主机上安装 ModelSwitch，购买按量付费 DeepSeek API 账号，通过 ModelSwitch 中转 token 服务，分发给约 1000 名员工使用。

**目标**: 评估当前软件在此企业场景下的不足，按优先级列出需要重构和迭代的功能。

**部署模式**: CLI 模式 (`modelswitch-cli serve`) 或 Docker 容器，单机部署，无集群。

---

## 当前能力评估

| 能力 | 现状 | 企业适用性 |
|------|------|-----------|
| DeepSeek 代理 | ✅ 完整支持 (OpenAI 兼容) | 可直接使用 |
| 虚拟密钥 | ✅ 有基础结构 (SHA-256, 预算, 模型限制) | 需大幅增强 |
| 预算控制 | ✅ 每密钥日/月预算 + 提供商预算 | 可用但需完善 |
| 通道管理 | ✅ 熔断器、健康检查、模型映射 | 可用 |
| 管理 API | ✅ 28 个端点 + 统一响应信封 | 可用 |
| 安全防护 | ✅ SSRF 防护、常数时间比较、IP 暴力破解保护 | 基础足够 |

**结论**: 核心代理通道基本可用，但**密钥管理、审计追踪、运维部署**三大领域存在严重不足，无法满足 1000 人规模的企业运营需求。

---

## 关键差距（按严重程度排列）

### 🔴 P0 — 阻塞性问题（不修复无法上线）

#### 1. DispatchLog 缺少 virtual_key_id 字段
**文件**: `src-tauri/src/log.rs`
**问题**: 调度日志结构体 `DispatchLog` 没有 `virtual_key_id` 字段。无法按员工查询使用历史、生成部门费用报表、或定位滥用者。
**影响**: 老板无法知道 1000 个员工谁用了多少、花了多少钱。
**修复**: 
- 在 `DispatchLog` 增加 `virtual_key_id: Option<String>` 字段
- 在 `dispatch()` 中从认证上下文提取 key_id 并写入日志
- Admin API 增加 `/api/logs?key_id=xxx` 按密钥过滤查询

#### 2. IP 白名单存在但从未执行
**文件**: `src-tauri/src/virtual_key/mod.rs:check_ip_allowed()`, `src-tauri/src/middleware/virtual_key.rs`
**问题**: `VirtualKey.allowed_ips` 字段存在，`check_ip_allowed()` 方法也写好了，但虚拟密钥认证中间件**从未调用**此方法。任何拿到密钥的人都可以从任何 IP 使用。
**影响**: 密钥泄露后无法通过 IP 限制减轻风险，老板无法将密钥绑定到公司内网。
**修复**: 在 `virtual_key.rs` 中间件的认证成功分支中调用 `check_ip_allowed()`，拒绝非白名单 IP。

#### 3. 审计日志仅内存存储，不持久化
**文件**: `src-tauri/src/admin/audit.rs`
**问题**: 审计日志是 1000 条容量的内存环形缓冲区，进程重启全部丢失。
**影响**: 无合规审计能力，无法追溯密钥创建/删除/变更操作历史。企业审计和合规要求无法满足。
**修复**: 将审计日志改为 `PersistedStore`，或写入 NDJSON 文件（参考 DispatchLogger 模式）。

#### 4. 无密钥批量创建/导入功能
**文件**: `src-tauri/src/admin/virtual_keys.rs`
**问题**: 只能逐个通过 API 创建密钥。为 1000 名员工创建密钥需要 1000 次 API 调用。
**影响**: 上线效率极低，管理成本不可接受。
**修复**: 
- 新增 `POST /api/virtual-keys/batch` 接受 `{count, prefix, daily_budget_cents, allowed_models, allowed_ips_range}`
- 支持 CSV 导入（员工姓名 → 密钥名映射）
- 前端增加批量创建向导 UI

#### 5. 密钥列表无分页
**文件**: `src-tauri/src/admin/virtual_keys.rs` — `list()` 返回所有密钥
**问题**: 1000 个密钥一次性返回，响应体可能 >100KB，前端渲染卡顿。
**修复**: `GET /api/virtual-keys?page=1&limit=50&search=xxx&enabled=true` 支持分页、搜索、过滤。

---

### 🟠 P1 — 高优先级（上线前应修复）

#### 6. 无每密钥速率限制 (RPM/TPM)
**问题**: 速率限制仅存在于通道层 (`channel.rpm_limit`, `channel.tpm_limit`)，虚拟密钥层无独立限制。一个员工可以耗尽整个通道的速率配额。
**修复**: 在 `VirtualKey` 增加 `rpm_limit` / `tpm_limit` 字段，在 dispatch 中间件中实施每密钥速率窗口。

#### 7. 无密钥过期机制
**问题**: `VirtualKey` 无 `expires_at` 字段。员工离职后密钥无法自动失效，只能手动删除。
**修复**: 
- 增加 `expires_at: Option<DateTime<Utc>>` 字段
- 认证中间件检查过期状态
- Admin API 支持"批量设置过期"功能

#### 8. O(N) 密钥验证性能
**文件**: `src-tauri/src/virtual_key/mod.rs`
**问题**: 每个请求遍历所有密钥做 SHA-256 比较。1000 个密钥意味着每次请求 1000 次哈希计算。
**影响**: 高并发下 CPU 瓶颈。假设 1000 人每人 10 请求/分钟 = 10,000 次哈希/分钟。
**修复**: 改为 `HashMap<String, VirtualKey>` 以 key_prefix 做索引，或使用布隆过滤器快速排除不匹配的密钥，将验证复杂度从 O(N) 降到 O(1)。

#### 9. 60 秒数据丢失窗口
**文件**: `src-tauri/src/persisted_store.rs`
**问题**: `PersistedStore` 每 60 秒刷盘一次。硬崩溃（断电、OOM）会丢失最近 60 秒的预算消费记录。
**影响**: 员工可能利用崩溃窗口超额使用。
**修复**: 
- 预算扣减时同步刷盘（write-through）
- 或缩短刷盘间隔到 5 秒
- 或引入 WAL (Write-Ahead Log)

#### 10. 无部署制品（systemd / docker-compose / k8s）
**问题**: 有 Dockerfile 但无 docker-compose.yml、systemd unit 文件、或运维部署文档。
**影响**: 传统公司 IT 人员缺乏开箱即用的部署方案。
**修复**: 
- 提供 `docker-compose.yml`（含 volumes 映射、健康检查、自动重启）
- 提供 `modelswitch.service` systemd unit 模板
- 编写部署文档（中文）

---

### 🟡 P2 — 中优先级（上线后迭代）

#### 11. 无员工自助门户
**问题**: 员工无法查看自己的剩余预算、使用历史、或测试密钥是否有效。所有查询需要找管理员。
**修复**: 新增 `/portal` 页面，员工输入自己的密钥即可查看：
- 今日/本月用量与剩余预算
- 最近请求记录（脱敏）
- 密钥有效性测试

#### 12. 无邮件通知
**问题**: 通知系统仅支持 Webhook 和 Bark。预算耗尽只能推送到技术系统，无法邮件通知员工或老板。
**修复**: 在 `NotificationConfig` 增加 SMTP 配置，新增 `notification::email` 模块。

#### 13. 无 SSO/LDAP/AD 集成
**问题**: 认证仅靠静态 admin token + 虚拟密钥。无法对接企业 Active Directory / LDAP。
**影响**: 无法将密钥与企业身份关联，员工离职后 IT 无法统一撤销。
**修复**: 
- Phase 1: 支持 LDAP/AD 认证虚拟密钥（密钥绑定 AD 用户）
- Phase 2: 支持 OIDC/OAuth2 SSO 登录管理面板

#### 14. 无部门/组管理
**问题**: 虚拟密钥是扁平结构，无法按部门分组管理和汇总预算。
**修复**: 增加 `group` / `department` 字段到 VirtualKey，支持组级预算汇总和报表。

#### 15. 无使用报表/导出
**问题**: 统计 API 只提供全局聚合数据，无法按密钥、部门、时间范围生成费用报表。
**修复**: 
- 增加 `/api/reports/usage?key_id=xxx&from=...&to=...&group_by=day` 
- 支持 CSV 导出供财务对账

#### 16. 无 SLA 监控/告警
**问题**: 有 Prometheus metrics 端点但无内置仪表盘或告警规则。
**修复**: 
- 提供 Grafana 仪表盘模板 JSON
- 增加延迟 P99/错误率告警规则

#### 17. 审计日志无防篡改机制
**问题**: 审计日志持久化后仍是普通 JSON/NDJSON 文件，管理员可修改或删除。
**影响**: 不满足 SOC 2 / ISO 27001 审计追踪的完整性要求。
**修复**: 
- 每条审计记录附加前一条记录的 SHA-256 哈希，形成哈希链
- 启动时验证哈希链完整性
- 文件权限设为只追加 (append-only)

#### 18. 无数据备份与恢复策略
**问题**: `virtual_keys.json`、`logs.ndjson`、`audit.ndjson` 等数据文件无自动备份机制。硬盘故障导致全部数据丢失。
**修复**: 
- 提供 `scripts/backup.sh` 脚本：定时打包 data 目录到指定位置
- docker-compose 中配置 volume 备份 sidecar
- 文档说明恢复流程：停止服务 → 恢复文件 → 重启

#### 19. 无灾难恢复计划 (DR)
**问题**: 单机单进程，无 RTO/RPO 定义，无故障切换方案。
**修复**: 
- 定义 RTO ≤ 30 分钟、RPO ≤ 5 分钟（基于刷盘间隔）
- 文档化故障恢复流程
- Phase 3 考虑主备模式（共享存储 + keepalived VIP 切换）

---

### 🟢 P3 — 远期优化（企业成熟期）

#### 20. 无细粒度权限控制 (RBAC)
**问题**: 管理面板仅支持单一 admin token，无角色分离。
**修复**: 
- 支持角色：超级管理员、密钥管理员（仅 CRUD 虚拟密钥）、审计员（只读）
- 基于 JWT 或多 token 的角色识别
- 管理操作的二次确认（删除、批量操作）

#### 21. 无具体监控告警规则
**问题**: 有 Prometheus metrics 但无告警阈值定义和升级策略。
**修复**: 
- 告警规则：错误率 > 5%、P99 延迟 > 2s、预算使用 > 80%、通道熔断
- 告警渠道：邮件 + 企业微信/钉钉 Webhook
- 告警升级：5 分钟未确认 → 升级到上级

#### 22. 无数据结构迁移策略
**问题**: 新增字段（如 `virtual_key_id`、`expires_at`）时依赖 serde `#[serde(default)]` 被动兼容，无显式迁移脚本。
**修复**: 
- 数据文件版本号 (`schema_version` 字段)
- 启动时自动迁移脚本（v1 → v2 → v3）
- 迁移前自动备份

#### 23. 无性能基准测试
**问题**: 未定义 1000 并发用户场景下的性能指标。
**修复**: 
- 定义基线：1000 RPS 下 P99 < 500ms、CPU < 70%、内存 < 512MB
- 提供 `benchmarks/enterprise-load-test.yml` (k6 或 wrk 脚本)
- CI 中运行性能回归测试

#### 24. 文档体系不完整
**问题**: 仅 README 和 config.example.toml，无运维手册。
**修复**: 
- `docs/admin-guide.md` — 管理员操作手册（中文）
- `docs/deployment.md` — 部署指南
- `docs/troubleshooting.md` — 故障排查
- `docs/api-reference.md` — API 文档（含示例）

#### 25. 无部署版本管理与回滚
**问题**: 无灰度发布和快速回滚机制。
**修复**: 
- Docker 镜像版本标签（非 latest）
- docker-compose 支持版本回滚
- 文档化回滚流程

#### 26. 无日志长期归档
**问题**: NDJSON 日志轮换仅保留 5 个文件（~500MB），无冷存储归档。
**修复**: 
- 归档脚本：30 天以上日志压缩移到归档目录
- 合规保留期限配置（默认 180 天）
- 归档日志支持按日期范围检索

---

## 实施路线图

### Phase 1: 阻塞性修复（1-2 周）— 满足最小可上线条件

| # | 任务 | 关键文件 | 工时 |
|---|------|---------|------|
| 1 | DispatchLog 增加 virtual_key_id | `log.rs`, `proxy/mod.rs` | 2天 |
| 2 | 虚拟密钥中间件启用 IP 白名单 | `middleware/virtual_key.rs` | 0.5天 |
| 3 | 审计日志持久化 | `admin/audit.rs` → PersistedStore | 1天 |
| 4 | 批量密钥创建 API + CSV 导入 | `admin/virtual_keys.rs`, 前端 | 2天 |
| 5 | 密钥列表分页/搜索/过滤 | `admin/virtual_keys.rs`, 前端 | 1天 |

**验证**: 
- `cargo test` 全部通过
- 手动创建 1000 个密钥，验证列表分页和搜索性能
- 验证日志按 key_id 过滤查询正常
- 验证 IP 白名单拒绝非白名单请求
- 重启进程后审计日志仍然存在

### Phase 2: 企业增强（2-3 周）— 生产级安全与运维

| # | 任务 | 关键文件 | 工时 |
|---|------|---------|------|
| 6 | 每密钥速率限制 | `virtual_key/mod.rs`, `proxy/mod.rs` | 2天 |
| 7 | 密钥过期机制 | `virtual_key/mod.rs`, 中间件 | 1天 |
| 8 | O(1) 密钥验证 (HashMap 索引) | `virtual_key/mod.rs` | 2天 |
| 9 | 预算刷盘缩短/WAL | `persisted_store.rs` | 1.5天 |
| 10 | docker-compose + systemd 部署制品 | 新文件 | 1天 |
| 11 | 员工自助门户 | 新 `src/Portal.tsx` + API | 3天 |
| 12 | 审计日志防篡改（哈希链） | `admin/audit.rs` | 1.5天 |
| 13 | 数据备份脚本 + 恢复流程 | `scripts/backup.sh`, 文档 | 1天 |
| 14 | 灾难恢复计划文档 | `docs/dr.md` | 0.5天 |

### Phase 3: 深度集成（3-4 周）— 企业 IT 对接

| # | 任务 | 关键文件 | 工时 |
|---|------|---------|------|
| 12 | SMTP 邮件通知 | `notification/` | 2天 |
| 13 | LDAP/AD 集成 | 新 `auth/ldap.rs` | 4天 |
| 14 | 部门/组管理 | `virtual_key/mod.rs`, 前端 | 3天 |
| 15 | 使用报表 + CSV 导出 | 新 `admin/reports.rs`, 前端 | 3天 |
| 16 | Grafana 仪表盘模板 + 告警规则 | 新 `deploy/grafana/` | 1天 |

### Phase 4: 企业成熟期（4-6 周）— RBAC + 可观测性 + 文档

| # | 任务 | 关键文件 | 工时 |
|---|------|---------|------|
| 20 | 细粒度 RBAC | `middleware/auth.rs`, 前端 | 4天 |
| 21 | 监控告警规则 + 升级策略 | `deploy/prometheus/rules.yml` | 2天 |
| 22 | 数据结构迁移框架 | `persisted_store.rs` | 2天 |
| 23 | 性能基准测试 (k6/wrk) | `benchmarks/` | 2天 |
| 24 | 完整文档体系（4 份文档） | `docs/` | 3天 |
| 25 | 版本管理与回滚机制 | Docker, 文档 | 1天 |
| 26 | 日志归档脚本 | `scripts/archive-logs.sh` | 1天 |

---

## 推荐的 DeepSeek 配置方案

```toml
# config.toml — 企业单通道 DeepSeek 配置
[[channels]]
id = "deepseek-main"
name = "DeepSeek 企业主通道"
provider = "deepseek"
priority = 1
weight = 100
base_url = "https://api.deepseek.com/v1"
api_keys = ["sk-deepseek-xxx"]
enabled = true
rpm_limit = 3000        # DeepSeek 官方 RPM 限制
tpm_limit = 2000000     # TPM 限制

[provider_budgets.deepseek]
daily_budget_cents = 5000    # 日预算 50 元
monthly_budget_cents = 100000 # 月预算 1000 元

[notifications]
budget_threshold_pct = 80    # 80% 时通知
webhook_url = "https://hooks.example.com/modelswitch"
```

---

## 验证清单

完成 Phase 1 后需验证：
1. `cargo build --no-default-features` — CLI 模式编译通过
2. `cargo build --features tauri` — Tauri 模式编译通过
3. `cargo test` — 所有测试通过
4. `cargo clippy -- -D warnings` — 无警告
5. 创建 1000 个密钥，验证列表 API 分页响应 < 100ms
6. 验证日志包含 virtual_key_id 且可按密钥过滤
7. 验证非白名单 IP 请求被拒绝（403）
8. 重启进程后审计日志仍然存在
9. 模拟员工使用流程：创建密钥 → 分发 → 调用 `/v1/chat/completions` → 查看用量
10. 验证审计日志哈希链完整性检测（篡改后报告错误）
11. 验证数据备份脚本正确打包所有数据文件
12. 验证数据恢复流程：备份 → 停服 → 恢复 → 重启 → 数据完整
13. 性能测试：1000 并发 RPS，P99 < 500ms
14. 验证 RBAC：审计员无法修改密钥，密钥管理员无法修改通道
