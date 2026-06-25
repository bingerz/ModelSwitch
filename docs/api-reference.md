# ModelSwitch API 快速参考

> 管理端与代理端 API 端点速查, 包含示例 curl 命令。

## 目录

1. [认证方式](#认证方式)
2. [代理端点 (Proxy API)](#代理端点-proxy-api)
3. [密钥管理](#密钥管理)
4. [通道管理](#通道管理)
5. [预算与配额](#预算与配额)
6. [监控与统计](#监控与统计)
7. [Portal 端点](#portal-端点)
8. [报告端点](#报告端点)

---

## 认证方式

ModelSwitch 使用两种认证方式, 分别用于不同的场景。

### Admin Token (管理 API)

用于 `/api/*` 路径下的所有管理操作。

```bash
# 通过 HTTP Header
Authorization: Bearer <ADMIN_TOKEN>

# 示例
curl http://localhost:8080/api/channels \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

> Admin Token 在 `config.toml` 的 `[security] admin_token` 中配置, 或通过环境变量 `MODELSWITCH_ADMIN_TOKEN` 设置。

### Virtual Key (代理 API)

用于 `/v1/*` 路径下的 LLM 代理请求。

```bash
# 通过 HTTP Header (OpenAI 兼容格式)
Authorization: Bearer <VIRTUAL_KEY>

# 示例
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model": "deepseek-chat", "messages": [{"role": "user", "content": "Hello"}]}'
```

> Virtual Key 通过管理 API 创建, 支持模型限制、预算控制和速率限制。

---

## 代理端点 (Proxy API)

所有代理端点使用 Virtual Key 认证。

### Chat Completions (OpenAI 兼容)

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "deepseek-chat",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 100
  }'
```

### Responses (OpenAI 兼容)

```bash
curl -X POST http://localhost:8080/v1/responses \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "deepseek-chat",
    "input": "Hello"
  }'
```

### Embeddings

```bash
curl -X POST http://localhost:8080/v1/embeddings \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "text-embedding-3-small",
    "input": "Hello world"
  }'
```

### Images - Generations

```bash
curl -X POST http://localhost:8080/v1/images/generations \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "dall-e-3",
    "prompt": "A serene landscape",
    "n": 1,
    "size": "1024x1024"
  }'
```

### Images - Edits

```bash
curl -X POST http://localhost:8080/v1/images/edits \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -F image=@input.png \
  -F prompt="Add a sunset"
```

### Messages (Anthropic 兼容)

```bash
curl -X POST http://localhost:8080/v1/messages \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-3-5-sonnet-20241022",
    "max_tokens": 100,
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

### List Models

```bash
curl http://localhost:8080/v1/models \
  -H "Authorization: Bearer $VIRTUAL_KEY"
```

### Get Model

```bash
curl http://localhost:8080/v1/models/deepseek-chat \
  -H "Authorization: Bearer $VIRTUAL_KEY"
```

### Provider-Prefixed Routes

支持在路径中指定 provider:

```bash
# 语法: /api/provider/{provider}/v1/...
curl -X POST http://localhost:8080/api/provider/deepseek/v1/chat/completions \
  -H "Authorization: Bearer $VIRTUAL_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model": "deepseek-chat", "messages": [{"role": "user", "content": "Hi"}]}'
```

---

## 密钥管理

所有端点使用 Admin Token 认证。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/virtual-keys` | 列出所有虚拟密钥 |
| POST | `/api/virtual-keys` | 创建虚拟密钥 |
| POST | `/api/virtual-keys/batch` | 批量创建密钥 |
| PUT | `/api/virtual-keys/{id}` | 更新密钥 |
| DELETE | `/api/virtual-keys/{id}` | 删除密钥 |

### 创建密钥

```bash
curl -X POST http://localhost:8080/api/virtual-keys \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "研发密钥",
    "allowed_models": ["deepseek-chat"],
    "rpm_limit": 100,
    "daily_budget_cents": 500
  }'
```

### 批量创建

```bash
curl -X POST http://localhost:8080/api/virtual-keys/batch \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"count": 10, "name_prefix": "team-a", "allowed_models": ["deepseek-chat"]}'
```

---

## 通道管理

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/channels` | 列出所有通道 |
| POST | `/api/channels` | 创建通道 |
| PUT | `/api/channels/{id}` | 更新通道 |
| DELETE | `/api/channels/{id}` | 删除通道 |
| POST | `/api/channels/{id}/ping` | 连通性测试 |
| POST | `/api/channels/{id}/test` | 完整测试 (实际请求) |
| GET | `/api/channels/{id}/status` | 通道状态 |
| GET | `/api/channels/{id}/cooldown` | 冷却状态 |
| POST | `/api/channels/{id}/reset-circuit` | 重置熔断器 |
| POST | `/api/channels/test-all` | 测试所有通道 |
| POST | `/api/channels/batch/enable` | 批量启用 |
| POST | `/api/channels/batch/disable` | 批量禁用 |
| POST | `/api/channels/batch/delete` | 批量删除 |
| PUT | `/api/channels/batch/tags` | 批量更新标签 |
| GET | `/api/channels/{id}/payload-rules` | 查看 Payload 规则 |
| PUT | `/api/channels/{id}/payload-rules` | 设置 Payload 规则 |

### 创建通道

```bash
curl -X POST http://localhost:8080/api/channels \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "id": "deepseek-primary",
    "name": "DeepSeek",
    "provider": "deepseek",
    "priority": 1,
    "weight": 100,
    "api_key": "sk-xxx",
    "base_url": "https://api.deepseek.com",
    "enabled": true
  }'
```

---

## 预算与配额

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/provider-budgets` | 列出所有提供商预算 |
| PUT | `/api/provider-budgets/{provider}` | 设置提供商预算 |
| DELETE | `/api/provider-budgets/{provider}` | 删除提供商预算 |
| GET | `/api/quota` | 查看配额使用情况 |

### 设置提供商预算

```bash
curl -X PUT http://localhost:8080/api/provider-budgets/deepseek \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"daily_budget_cents": 1000, "monthly_budget_cents": 20000}'
```

---

## 监控与统计

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/stats` | 实时统计指标 |
| GET | `/api/stats/cost` | 成本统计 |
| GET | `/api/stats/usage` | 使用量历史 |
| GET | `/api/logs` | 调度日志 |
| GET | `/api/audit-log` | 审计日志 |
| GET | `/api/gateway/info` | 网关概要信息 |
| GET | `/api/cache/stats` | 缓存统计 |
| POST | `/api/cache/flush` | 清空缓存 |
| GET | `/api/model-registry` | 模型注册表 |
| GET | `/api/completion-ratios` | 补全比率 |
| PUT | `/api/completion-ratios` | 更新补全比率 |

### 配置管理

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/config/reload` | 热重载配置文件 |
| GET | `/api/guardrails` | 查看 Guardrails 配置 |
| PUT | `/api/guardrails` | 更新 Guardrails 配置 |

### 告警通知

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/notifications` | 查看通知配置 |
| PUT | `/api/notifications` | 更新通知配置 |

### MCP 服务器

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/mcp/servers` | 列出 MCP 服务器 |
| POST | `/api/mcp/servers` | 创建 MCP 服务器 |
| PUT | `/api/mcp/servers/{id}` | 更新 MCP 服务器 |
| DELETE | `/api/mcp/servers/{id}` | 删除 MCP 服务器 |
| POST | `/api/mcp/servers/{id}/start` | 启动 MCP 服务器 |
| POST | `/api/mcp/servers/{id}/stop` | 停止 MCP 服务器 |
| GET | `/api/mcp/servers/{id}/tools` | 列出 MCP 工具 |
| GET | `/api/mcp/tools` | 列出所有 MCP 工具 |
| GET | `/api/mcp/health` | MCP 健康检查 |

### 兑换码

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/redemption-codes` | 列出兑换码 |
| POST | `/api/redemption-codes` | 创建兑换码 |
| POST | `/api/redemption-codes/redeem` | 兑换码兑换 |
| DELETE | `/api/redemption-codes/{code}` | 删除兑换码 |

---

## Portal 端点

Portal 端点使用 Virtual Key 认证, 供最终用户查看自身使用情况。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/portal/usage` | 查看当前密钥的使用量 |
| GET | `/api/portal/logs` | 查看当前密钥的请求日志 |
| POST | `/api/portal/test` | 测试当前密钥连通性 |

### 查看个人用量

```bash
curl http://localhost:8080/api/portal/usage \
  -H "Authorization: Bearer $VIRTUAL_KEY"
```

### 查看个人日志

```bash
curl "http://localhost:8080/api/portal/logs?limit=20" \
  -H "Authorization: Bearer $VIRTUAL_KEY"
```

---

## 报告端点

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/reports/usage` | 使用报告 (JSON) |
| GET | `/api/reports/usage/csv` | 使用报告 (CSV 导出) |

### 获取使用报告

```bash
curl "http://localhost:8080/api/reports/usage?start=2026-06-01&end=2026-06-30" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 导出 CSV 报告

```bash
curl "http://localhost:8080/api/reports/usage/csv?start=2026-06-01&end=2026-06-30" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -o usage-report-202606.csv
```

---

## 健康检查

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/healthz` | 无 | 基础存活检查 |
| GET | `/health` | 无 | 完整健康检查 |

```bash
# 基础检查
curl http://localhost:8080/healthz
# 预期: {"status":"ok"}

# 完整检查
curl http://localhost:8080/health
# 返回通道状态、连接池等详细信息
```
