# ModelSwitch 故障排查指南

> 常见问题的快速诊断与解决方案。

## 目录

1. [服务无法启动](#服务无法启动)
2. [高延迟问题](#高延迟问题)
3. [预算超限错误](#预算超限错误)
4. [熔断器频繁触发](#熔断器频繁触发)
5. [数据文件损坏](#数据文件损坏)
6. [常见错误消息](#常见错误消息)

---

## 服务无法启动

### 症状

服务启动后立即退出, 或 systemd 状态显示 `failed`。

### 诊断步骤

**1. 查看启动日志**

```bash
# systemd
sudo journalctl -u modelswitch --since "5 min ago" --no-pager

# Docker
docker compose logs modelswitch --tail 50
```

**2. 检查端口占用**

```bash
lsof -i :8080
# 如果端口被占用, 杀掉占用进程或修改配置中的 port
```

**3. 验证配置文件**

```bash
# 检查 TOML 语法
# 确认 config.toml 没有语法错误
cat /etc/modelswitch/config.toml
```

**4. 检查文件权限**

```bash
# systemd 部署
ls -la /etc/modelswitch/config.toml
ls -la /var/lib/modelswitch/
# 确认 modelswitch 用户有读写权限
```

### 常见原因与解决

| 原因 | 表现 | 解决方案 |
|------|------|----------|
| 端口被占用 | `Address already in use` | 修改 `port` 或释放端口 |
| 配置语法错误 | `TOML parse error` | 修正 TOML 语法 |
| 权限不足 | `Permission denied` | `chown modelswitch:modelswitch` |
| Admin Token 未设 | 启动警告 | 设置 `admin_token` 或环境变量 |
| 数据目录不存在 | `No such file or directory` | 创建数据目录并设置权限 |

---

## 高延迟问题

### 症状

请求响应时间明显高于预期, P99 延迟超过 500ms。

### 诊断步骤

**1. 查看实时指标**

```bash
curl http://localhost:8080/api/stats \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**2. 检查通道延迟**

```bash
# 测试单个通道
curl -X POST http://localhost:8080/api/channels/{id}/ping \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**3. 检查资源使用**

```bash
# Docker
docker stats modelswitch

# systemd
top -p $(pgrep modelswitch)
```

**4. 检查连接池**

```bash
curl http://localhost:8080/api/gateway/info \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 优化建议

| 场景 | 优化方案 |
|------|----------|
| 上游 API 延迟高 | 切换到延迟更低的通道或启用降级 |
| 连接池过小 | 提高 `http_pool_size` (默认 8, 建议 16-32) |
| 内存不足 | 增加 Docker 内存限制 |
| 缓存未启用 | 确认 `cache_mode = "on"` |
| RPM 限制过低 | 调高通道和密钥的 `rpm_limit` |

### 性能基准测试

运行基准测试以获取基线数据:

```bash
# 参见 benchmarks/README.md
k6 run benchmarks/enterprise-load-test.yml
```

---

## 预算超限错误

### 症状

API 返回 `402 Payment Required` 或 `429 Too Many Requests`, 错误信息包含 "budget"。

### 诊断步骤

**1. 检查提供商预算**

```bash
curl http://localhost:8080/api/provider-budgets \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**2. 检查密钥配额**

```bash
curl http://localhost:8080/api/quota \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**3. 查看成本统计**

```bash
curl http://localhost:8080/api/stats/cost \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 解决方案

| 场景 | 解决方案 |
|------|----------|
| 提供商日预算超限 | 提高 `daily_budget_cents` 或等待次日重置 |
| 提供商月预算超限 | 提高 `monthly_budget_cents` |
| 密钥日预算超限 | 更新密钥的 `daily_budget_cents` |
| 密钥 RPM 超限 | 提高 `rpm_limit` 或等待下一分钟 |

**调整预算示例:**

```bash
curl -X PUT http://localhost:8080/api/provider-budgets/deepseek \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"daily_budget_cents": 2000, "monthly_budget_cents": 50000}'
```

---

## 熔断器频繁触发

### 症状

通道被自动禁用, 返回 `503 Service Unavailable`, 或日志中出现 `circuit breaker` 字样。

### 诊断步骤

**1. 查看通道状态**

```bash
curl http://localhost:8080/api/channels/{id}/status \
  -H "Authorization: Bearer $ADMIN_TOKEN"

curl http://localhost:8080/api/channels/{id}/cooldown \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**2. 检查上游连通性**

```bash
curl -X POST http://localhost:8080/api/channels/{id}/ping \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**3. 查看调度日志**

```bash
curl "http://localhost:8080/api/logs?limit=50&channel={id}" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### 常见原因

| 原因 | 检查方式 |
|------|----------|
| API Key 失效 | Ping 通道, 确认返回 401/403 |
| 上游限速 (429) | 查看日志中是否有 429 状态码 |
| 上游服务不可用 | Ping 返回 5xx 或超时 |
| 网络问题 | 从服务器直接 curl 上游地址 |
| RPM 设置过低 | 对比实际流量与 `rpm_limit` |

### 解决方案

**手动重置熔断器:**

```bash
curl -X POST http://localhost:8080/api/channels/{id}/reset-circuit \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

**调整熔断参数:**

在 `config.toml` 中修改:

```toml
[gateway]
circuit_breaker_minutes = 15    # 默认 30 分钟, 可缩短
max_retries = 5                  # 默认 3 次
```

修改后热重载:

```bash
curl -X POST http://localhost:8080/api/config/reload \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

---

## 数据文件损坏

### 症状

服务启动时报告 JSON 解析错误, 或运行中出现数据读写异常。

### 诊断步骤

**1. 定位数据目录**

```bash
# systemd 默认
ls -la /var/lib/modelswitch/

# Docker (查看挂载卷)
docker inspect modelswitch | grep -A5 Mounts
```

**2. 验证 JSON 文件完整性**

```bash
# 检查每个 JSON 文件是否可解析
for f in /var/lib/modelswitch/*.json; do
  echo "=== $f ==="
  python3 -m json.tool "$f" > /dev/null 2>&1 || echo "CORRUPTED: $f"
done
```

### 修复方案

**方案 A: 从备份恢复**

```bash
# 停止服务
sudo systemctl stop modelswitch

# 恢复
./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-YYYYMMDD-HHMMSS.tar.gz

# 重启
sudo systemctl start modelswitch
```

**方案 B: 手动修复 JSON**

```bash
# 备份损坏文件
cp /var/lib/modelswitch/virtual_keys.json /var/lib/modelswitch/virtual_keys.json.bak

# 手动编辑修复语法错误
vim /var/lib/modelswitch/virtual_keys.json

# 验证
python3 -m json.tool /var/lib/modelswitch/virtual_keys.json
```

**方案 C: 重置损坏的文件**

如果单个文件无法修复且无备份, 可删除该文件让服务重建:

```bash
sudo systemctl stop modelswitch
rm /var/lib/modelswitch/quota.json   # 重置配额数据
sudo systemctl start modelswitch
```

> 注意: 删除文件将丢失对应数据。仅在确认影响可控时操作。

---

## 常见错误消息

### HTTP 错误码

| 状态码 | 含义 | 常见原因 |
|--------|------|----------|
| 400 | Bad Request | 请求体格式错误 |
| 401 | Unauthorized | Admin Token 或虚拟密钥无效 |
| 402 | Payment Required | 预算超限 |
| 403 | Forbidden | 密钥无权访问该模型 |
| 429 | Too Many Requests | RPM/TPM 超限 |
| 502 | Bad Gateway | 上游返回无效响应 |
| 503 | Service Unavailable | 所有通道被熔断 |
| 504 | Gateway Timeout | 上游请求超时 |

### 常见错误信息

#### `{"error": "invalid api key"}`

- **原因:** 虚拟密钥不存在或已被删除
- **解决:** 检查 `Authorization` 头中的密钥是否正确

#### `{"error": "model not allowed"}`

- **原因:** 密钥的 `allowed_models` 不包含请求的模型
- **解决:** 更新密钥的 `allowed_models` 列表

#### `{"error": "rate limit exceeded"}`

- **原因:** 超过 RPM 或 TPM 限制
- **解决:** 提高 `rpm_limit` 或降低请求频率

#### `{"error": "budget exceeded"}`

- **原因:** 超过日/月预算上限
- **解决:** 提高预算或等待周期重置

#### `{"error": "no available channels"}`

- **原因:** 所有通道被熔断或禁用
- **解决:** 重置熔断器或启用通道

#### `{"error": "upstream timeout"}`

- **原因:** 上游 API 响应超时
- **解决:** 检查上游服务状态, 或增加超时时间

#### `{"error": "circuit breaker open"}`

- **原因:** 通道因连续失败触发熔断
- **解决:** 等待自动恢复或手动重置

### 获取帮助

如果以上方案无法解决问题, 请收集以下信息后联系运维团队:

```bash
# 1. 版本信息
curl http://localhost:8080/api/gateway/info \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 2. 最近日志
sudo journalctl -u modelswitch --since "1 hour ago" > debug.log

# 3. 通道状态
curl http://localhost:8080/api/channels \
  -H "Authorization: Bearer $ADMIN_TOKEN" > channels.json

# 4. 配置文件 (脱敏)
cat /etc/modelswitch/config.toml | sed 's/api_key.*/api_key = "***REDACTED***"/' > config-sanitized.toml
```
