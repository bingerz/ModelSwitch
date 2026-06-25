# ModelSwitch 管理员操作手册

> 面向运维管理员的日常操作指南 — 涵盖密钥管理、预算配置、通道管理、监控告警、备份恢复与常见问题排查。

## 目录

1. [密钥管理](#密钥管理)
2. [预算配置](#预算配置)
3. [通道管理](#通道管理)
4. [监控与告警](#监控与告警)
5. [备份与恢复](#备份与恢复)
6. [常见问题](#常见问题)

---

## 密钥管理

所有管理操作通过 `/api` 前缀的管理 API 完成, 需携带 Admin Token。

### 1.1 创建虚拟密钥

```bash
curl -X POST http://localhost:8080/api/virtual-keys \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "研发团队密钥",
    "allowed_models": ["deepseek-chat", "deepseek-reasoner"],
    "expires_at": "2026-12-31T23:59:59Z",
    "rpm_limit": 100,
    "daily_budget_cents": 500
  }'
```

**关键字段说明:**

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 密钥名称 (便于识别) |
| `allowed_models` | string[] | 允许调用的模型列表 |
| `expires_at` | ISO 8601 | 过期时间 (null 表示永不过期) |
| `rpm_limit` | number | 每分钟请求数上限 |
| `daily_budget_cents` | number | 每日预算上限 (单位: 美分) |

### 1.2 批量创建虚拟密钥

适用于为大量用户一次性生成密钥的场景。

```bash
curl -X POST http://localhost:8080/api/virtual-keys/batch \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "count": 50,
    "name_prefix": "eval-team",
    "allowed_models": ["deepseek-chat"],
    "expires_at": "2026-09-30T23:59:59Z",
    "rpm_limit": 60,
    "daily_budget_cents": 200
  }'
```

响应将返回所有生成的密钥列表, 请妥善保存。

### 1.3 查看所有密钥

```bash
curl http://localhost:8080/api/virtual-keys \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 1.4 更新密钥

```bash
curl -X PUT http://localhost:8080/api/virtual-keys/{key_id} \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "rpm_limit": 200,
    "daily_budget_cents": 1000
  }'
```

### 1.5 删除密钥

```bash
curl -X DELETE http://localhost:8080/api/virtual-keys/{key_id} \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

> 删除后该密钥立即失效, 所有使用该密钥的请求将被拒绝。

### 1.6 设置密钥过期

通过更新密钥的 `expires_at` 字段实现定时过期:

```bash
curl -X PUT http://localhost:8080/api/virtual-keys/{key_id} \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"expires_at": "2026-07-01T00:00:00Z"}'
```

---

## 预算配置

### 2.1 查看提供商预算

```bash
curl http://localhost:8080/api/provider-budgets \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 2.2 设置提供商预算

```bash
curl -X PUT http://localhost:8080/api/provider-budgets/deepseek \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "daily_budget_cents": 1000,
    "monthly_budget_cents": 20000
  }'
```

| 字段 | 说明 |
|------|------|
| `daily_budget_cents` | 每日预算上限 (美分, 1000 = $10) |
| `monthly_budget_cents` | 每月预算上限 (美分, 20000 = $200) |

### 2.3 删除提供商预算

```bash
curl -X DELETE http://localhost:8080/api/provider-budgets/deepseek \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 2.4 密钥级预算

除提供商级别预算外, 每个虚拟密钥也可独立设置 `daily_budget_cents`, 实现双重预算控制。

预算优先级: 密钥级预算 > 提供商级预算。任一预算达到上限即拒绝请求。

---

## 通道管理

### 3.1 查看通道列表

```bash
curl http://localhost:8080/api/channels \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 3.2 创建通道

```bash
curl -X POST http://localhost:8080/api/channels \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "id": "deepseek-primary",
    "name": "DeepSeek Enterprise",
    "provider": "deepseek",
    "priority": 1,
    "weight": 100,
    "api_key": "sk-your-api-key",
    "base_url": "https://api.deepseek.com",
    "enabled": true,
    "input_cost_per_mtok": 0.14,
    "output_cost_per_mtok": 0.28,
    "rpm_limit": 100,
    "tpm_limit": 50000
  }'
```

### 3.3 测试通道连通性

```bash
# 单个通道测试
curl -X POST http://localhost:8080/api/channels/{id}/ping \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 完整测试 (发送实际请求)
curl -X POST http://localhost:8080/api/channels/{id}/test \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 测试所有通道
curl -X POST http://localhost:8080/api/channels/test-all \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 3.4 查看通道状态 (含熔断/冷却)

```bash
# 单个通道状态
curl http://localhost:8080/api/channels/{id}/status \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 单个通道冷却状态
curl http://localhost:8080/api/channels/{id}/cooldown \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 3.5 重置熔断器

通道因连续错误触发熔断后, 可手动重置:

```bash
curl -X POST http://localhost:8080/api/channels/{id}/reset-circuit \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 3.6 批量操作

```bash
# 批量启用
curl -X POST http://localhost:8080/api/channels/batch/enable \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"ids": ["channel-1", "channel-2"]}'

# 批量禁用
curl -X POST http://localhost:8080/api/channels/batch/disable \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"ids": ["channel-1", "channel-2"]}'

# 批量删除
curl -X POST http://localhost:8080/api/channels/batch/delete \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"ids": ["channel-1", "channel-2"]}'

# 批量更新标签
curl -X PUT http://localhost:8080/api/channels/batch/tags \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"ids": ["channel-1"], "tags": ["production", "deepseek"]}'
```

### 3.7 配置 Payload 规则

Payload 规则用于对特定通道的请求体进行定制化处理:

```bash
curl http://localhost:8080/api/channels/{id}/payload-rules \
  -H "Authorization: Bearer $ADMIN_TOKEN"

curl -X PUT http://localhost:8080/api/channels/{id}/payload-rules \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"rules": [...]}'
```

---

## 监控与告警

### 4.1 实时指标

```bash
curl http://localhost:8080/api/stats \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 4.2 成本统计

```bash
curl http://localhost:8080/api/stats/cost \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 4.3 使用量历史

```bash
curl "http://localhost:8080/api/stats/usage?days=7" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 4.4 调度日志

```bash
curl "http://localhost:8080/api/logs?limit=100" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 4.5 审计日志

```bash
curl http://localhost:8080/api/audit-log \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 4.6 缓存统计

```bash
curl http://localhost:8080/api/cache/stats \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 4.7 通知配置

配置告警通知渠道 (Webhook):

```bash
curl -X PUT http://localhost:8080/api/notifications \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "webhook_url": "https://hooks.slack.com/services/...",
    "events": ["budget_exceeded", "circuit_breaker_tripped"]
  }'
```

### 4.8 Prometheus + Grafana

项目自带 Prometheus 与 Grafana 配置, 位于 `deploy/` 目录:

```bash
# 启动完整监控栈
cd deploy/
docker compose up -d
```

Grafana 默认地址: `http://localhost:3000` (admin/admin)

### 4.9 MCP 服务器健康

```bash
curl http://localhost:8080/api/mcp/health \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

---

## 备份与恢复

### 5.1 手动备份

```bash
./scripts/backup.sh /var/backups/modelswitch
```

备份内容包含:

| 文件 | 用途 |
|------|------|
| `virtual_keys.json` | 虚拟密钥 |
| `quota.json` | 配额数据 |
| `provider_budgets.json` | 提供商预算 |
| `logs.ndjson*` | 调度日志 |
| `audit.ndjson` | 审计日志 |
| `config.toml` | 主配置文件 |

### 5.2 定时备份

```bash
# 编辑 crontab
sudo crontab -e

# 每天凌晨 3 点自动备份
0 3 * * * /path/to/scripts/backup.sh /var/backups/modelswitch
```

### 5.3 恢复

```bash
# 停止服务后恢复
./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-YYYYMMDD-HHMMSS.tar.gz

# 恢复后验证
curl http://localhost:8080/healthz
```

### 5.4 配置热重载

修改 `config.toml` 后无需重启即可生效:

```bash
curl -X POST http://localhost:8080/api/config/reload \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

---

## 常见问题

### Q: 忘记 Admin Token 怎么办?

Admin Token 在 `config.toml` 的 `[security]` 段中配置, 或通过环境变量 `MODELSWITCH_ADMIN_TOKEN` 设置。检查配置文件或 Docker 环境变量即可找回。

### Q: 虚拟密钥的配额突然归零?

查看是否触发了预算限制:
```bash
curl http://localhost:8080/api/quota \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### Q: 通道频繁被熔断?

1. 检查上游 API 是否正常: `POST /api/channels/{id}/ping`
2. 确认 API Key 是否有效
3. 检查 RPM/TPM 限制是否过低
4. 手动重置: `POST /api/channels/{id}/reset-circuit`

### Q: 如何清空缓存?

```bash
curl -X POST http://localhost:8080/api/cache/flush \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### Q: 如何查看网关整体信息?

```bash
curl http://localhost:8080/api/gateway/info \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

返回版本号、运行时间、通道数量、密钥数量等概要信息。
