# ModelSwitch 部署指南

> 企业级 LLM 网关部署文档 — 涵盖 Docker 与 systemd 两种方案, 以及备份恢复流程。

## 目录

1. [部署方案对比](#部署方案对比)
2. [Docker 部署](#docker-部署)
3. [systemd 部署](#systemd-部署)
4. [DeepSeek 企业评估场景配置](#deepseek-企业评估场景配置)
5. [日志归档](#日志归档)
6. [备份与恢复](#备份与恢复)
7. [故障排查](#故障排查)

---

## 部署方案对比

| 维度 | Docker | systemd |
|------|--------|---------|
| 部署复杂度 | 低 (一条命令) | 中 (需编译二进制) |
| 资源隔离 | 强 (容器级别) | 弱 (进程级别) |
| 配置管理 | 通过 `.env` 与卷挂载 | 通过 `config.toml` |
| 日志管理 | `docker logs` 或 json-file | `journalctl` |
| 自动重启 | `restart: unless-stopped` | `Restart=always` |
| 适用场景 | 快速验证、容器化环境 | 生产服务器、裸机部署 |

**推荐:**
- 测试 / PoC 阶段: Docker
- 生产环境: systemd (更低的开销, 更精细的安全控制)

---

## Docker 部署

### 前置条件

- Docker 20.10+
- Docker Compose v2+

### 步骤

1. **准备配置文件**

   ```bash
   mkdir -p ~/.config/modelswitch
   cp config.example.toml ~/.config/modelswitch/config.toml
   # 编辑 config.toml, 填写真实 API 密钥
   vim ~/.config/modelswitch/config.toml
   ```

2. **设置环境变量**

   ```bash
   cd deploy/
   cat > .env << EOF
   MODELSWITCH_ADMIN_TOKEN=your-secure-admin-token
   RUST_LOG=info
   EOF
   chmod 600 .env  # 保护密钥文件
   ```

3. **构建并启动**

   ```bash
   docker compose up -d --build
   ```

4. **验证服务**

   ```bash
   # 健康检查
   curl http://localhost:8080/healthz

   # 查看日志
   docker compose logs -f modelswitch
   ```

### 常用运维命令

```bash
# 停止服务
docker compose down

# 重启服务
docker compose restart

# 查看服务状态
docker compose ps

# 更新镜像 (代码变更后)
docker compose up -d --build
```

---

## systemd 部署

### 前置条件

- Linux 服务器 (Ubuntu 20.04+ / CentOS 8+ / Debian 11+)
- Rust 工具链 (仅编译时需要)

### 步骤

1. **编译二进制**

   ```bash
   # 在开发机或服务器上编译
   cd src-tauri/
   cargo build --bin modelswitch-cli --release
   # 产物: target/release/modelswitch-cli
   ```

2. **运行安装脚本**

   ```bash
   sudo ./deploy/install-systemd.sh
   ```

   脚本会自动完成:
   - 创建 `modelswitch` 系统用户
   - 复制二进制到 `/opt/modelswitch/`
   - 创建 `/etc/modelswitch/` 和 `/var/lib/modelswitch/`
   - 安装并启动 systemd 服务

3. **放置配置文件**

   ```bash
   sudo cp config.example.toml /etc/modelswitch/config.toml
   sudo chown modelswitch:modelswitch /etc/modelswitch/config.toml
   sudo chmod 640 /etc/modelswitch/config.toml
   sudo vim /etc/modelswitch/config.toml
   ```

4. **启动服务**

   ```bash
   sudo systemctl start modelswitch
   sudo systemctl status modelswitch
   ```

### 常用运维命令

```bash
# 启动 / 停止 / 重启
sudo systemctl start modelswitch
sudo systemctl stop modelswitch
sudo systemctl restart modelswitch

# 查看状态
sudo systemctl status modelswitch

# 实时查看日志
sudo journalctl -u modelswitch -f

# 查看最近 100 行日志
sudo journalctl -u modelswitch -n 100
```

---

## DeepSeek 企业评估场景配置

### 架构概览

```
企业内部服务
    │
    ├── OpenAI 请求 ──► ModelSwitch Gateway ──► DeepSeek API (主)
    │                        │
    │                        ├── OpenAI (备用, 降级)
    │                        └── 本地 Ollama (兜底)
    │
    └── 管理 API ──► /api/* (Admin Token 鉴权)
```

### 配置示例

在 `config.toml` 中配置 DeepSeek 为主通道, 其他为降级:

```toml
[gateway]
host = "0.0.0.0"          # 监听所有网卡 (Docker / 远程访问)
port = 8080
routing_strategy = "weighted_random"
max_retries = 3

# ── DeepSeek 主通道 ──────────────────────────────────────
[[channels]]
id = "deepseek-primary"
name = "DeepSeek Enterprise"
provider = "deepseek"
priority = 1               # 最高优先级
weight = 100
credential_type = "api_key"
credential_ref = "deepseek_enterprise"
api_key = "sk-your-deepseek-enterprise-key"
base_url = "https://api.deepseek.com"
enabled = true
input_cost_per_mtok = 0.14 # $0.14/M tokens
output_cost_per_mtok = 0.28
rpm_limit = 100
tpm_limit = 50000

# 将 OpenAI 模型名映射到 DeepSeek
[channels.model_mapping]
"gpt-4" = "deepseek-chat"
"gpt-4o" = "deepseek-chat"
"gpt-4o-mini" = "deepseek-chat"

# ── OpenAI 降级通道 ──────────────────────────────────────
[[channels]]
id = "openai-fallback"
name = "OpenAI Fallback"
provider = "openai"
priority = 2               # 降级
weight = 50
credential_type = "api_key"
credential_ref = "openai_main"
api_key = "sk-your-openai-key"
base_url = "https://api.openai.com"
enabled = true

# ── 预算控制 ──────────────────────────────────────────────
[gateway.provider_budgets.deepseek]
daily_budget_cents = 1000    # $10/天
monthly_budget_cents = 20000 # $200/月
```

### 管理安全

- **务必**修改默认的 `MODELSWITCH_ADMIN_TOKEN`
- 使用强随机 Token: `openssl rand -hex 32`
- 配置防火墙, 仅允许可信来源访问 8080 端口

---

## 日志归档

### 概述

ModelSwitch 的调度日志 (`logs.ndjson`) 和审计日志 (`audit.ndjson`) 会自动轮转, 默认保留 5 个轮转文件 (约 500MB)。对于需要长期保存日志以满足合规要求的场景, 可以使用 `scripts/archive-logs.sh` 脚本将旧日志压缩归档到单独的目录。

### 归档脚本

**功能:**
- 将超过指定天数的 NDJSON 日志文件压缩 (gzip) 并移动到归档目录
- 自动清理超过归档保留期的旧归档文件
- 记录归档操作日志

**可配置环境变量:**

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `MODELSWITCH_CONFIG_DIR` | `~/.config/modelswitch` | 数据目录 |
| `LOG_RETENTION_DAYS` | `30` | 日志保留天数, 超过后归档 |
| `ARCHIVE_RETENTION_DAYS` | `180` | 归档保留天数, 超过后删除 |

### 手动执行

```bash
# 使用默认配置 (30 天归档, 180 天清理)
./scripts/archive-logs.sh

# 自定义保留期
LOG_RETENTION_DAYS=14 ARCHIVE_RETENTION_DAYS=365 ./scripts/archive-logs.sh
```

### 配置每日自动归档 (推荐)

使用 `scripts/cron-setup.sh` 一键安装 cron 任务, 每天凌晨 2 点自动执行归档:

```bash
./scripts/cron-setup.sh
# 输出: Cron job installed: daily log archiving at 2 AM
```

验证 cron 任务:

```bash
crontab -l
# 应包含: 0 2 * * * /path/to/scripts/archive-logs.sh
```

移除 cron 任务:

```bash
crontab -l | grep -v archive-logs.sh | crontab -
```

### 归档文件结构

```
~/.config/modelswitch/
├── logs.ndjson              # 当前活跃日志
├── audit.ndjson             # 当前审计日志
└── archive/                 # 归档目录
    ├── logs.ndjson.20260625-020000.gz
    ├── audit.ndjson.20260625-020000.gz
    └── archive.log          # 归档操作日志
```

### Docker 环境中的日志归档

Docker 部署时, 可以在宿主机上配置 cron 调用归档脚本:

```bash
# 指定容器数据目录
export MODELSWITCH_CONFIG_DIR=/var/lib/docker/volumes/modelswitch-data/_data
./scripts/archive-logs.sh
```

或者在 `docker-compose.yml` 中添加一个归档服务:

```yaml
services:
  log-archiver:
    image: alpine:latest
    volumes:
      - modelswitch-data:/data
      - ./scripts:/scripts:ro
    environment:
      - MODELSWITCH_CONFIG_DIR=/data
      - LOG_RETENTION_DAYS=30
    entrypoint: /bin/sh
    command:
      - -c
      - |
        apk add --no-cache gzip findutils
        echo "0 2 * * * /scripts/archive-logs.sh" > /etc/crontabs/root
        crond -f
    restart: unless-stopped
```

---

## 备份与恢复

### 定时备份 (推荐配置 cron)

```bash
# 编辑 root 的 crontab
sudo crontab -e

# 每天凌晨 3 点自动备份
0 3 * * * /path/to/scripts/backup.sh /var/backups/modelswitch

# 每周日凌晨 4 点完整备份 (可选)
0 4 * * 0 /path/to/scripts/backup.sh /var/backups/modelswitch/weekly
```

### 手动备份

```bash
./scripts/backup.sh /var/backups/modelswitch
```

备份内容包含:
- `virtual_keys.json` — 虚拟密钥
- `quota.json` — 配额数据
- `provider_budgets.json` — 提供商预算
- `logs.ndjson*` — 调度日志
- `audit.ndjson` — 审计日志
- `config.toml` — 主配置文件

备份文件命名格式: `modelswitch-backup-YYYYMMDD-HHMMSS.tar.gz`
保留策略: 最近 30 天

### 恢复流程

```bash
# 1. 停止服务后恢复
./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-20260625-030000.tar.gz

# 2. 脚本会自动:
#    - 停止运行中的服务
#    - 提示确认
#    - 解压文件到数据目录
#    - 询问是否重启服务

# 3. 恢复后验证
curl http://localhost:8080/healthz
```

---

## 故障排查

### 日志位置

| 部署方式 | 日志命令 |
|---------|---------|
| Docker | `docker compose logs -f modelswitch` |
| systemd | `journalctl -u modelswitch -f` |
| 数据日志 | 数据目录下的 `logs.ndjson` |
| 审计日志 | 数据目录下的 `audit.ndjson` |

### 健康检查

```bash
# 基础存活检查 (无鉴权)
curl http://localhost:8080/healthz
# 预期: {"status":"ok"}

# 完整健康检查
curl http://localhost:8080/health
# 返回: 通道状态、连接池信息等
```

### 常见问题

#### 1. 服务无法启动

**排查步骤:**

```bash
# 查看启动日志
sudo journalctl -u modelswitch --since "5 min ago"

# Docker
docker compose logs modelswitch | tail -50
```

**常见原因:**
- 配置文件语法错误 (检查 TOML 格式)
- 端口被占用: `lsof -i :8080`
- 权限不足: 确认 `modelswitch` 用户对 `/etc/modelswitch/` 有读权限

#### 2. 上游 API 不通

```bash
# 测试通道连通性 (需 Admin Token)
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
     http://localhost:8080/api/channels

# 查看通道熔断状态
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
     http://localhost:8080/api/channels/status
```

**处理方式:**
- 通道被熔断后会在 `circuit_breaker_minutes` (默认 30 分钟) 后自动恢复
- 手动启用: 通过管理 API 修改 `enabled` 状态

#### 3. 性能问题

```bash
# 查看实时指标
curl -H "Authorization: Bearer $ADMIN_TOKEN" \
     http://localhost:8080/api/metrics

# 检查资源使用
# Docker
docker stats modelswitch

# systemd
systemctl status modelswitch
```

**调优建议:**
- 提高 `http_pool_size` (默认 8, 高并发可设 16-32)
- 启用缓存: `cache_mode = "on"` (默认已启用)
- 增加资源限制 (docker-compose.yml)

#### 4. 数据丢失 / 恢复

```bash
# 1. 停止服务
sudo systemctl stop modelswitch

# 2. 从备份恢复
./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-YYYYMMDD-HHMMSS.tar.gz

# 3. 验证配置完整性
cat /etc/modelswitch/config.toml

# 4. 重启服务
sudo systemctl start modelswitch

# 5. 健康检查
curl http://localhost:8080/healthz
```

---

## 附录: 文件清单

| 文件 | 用途 |
|------|------|
| `deploy/docker-compose.yml` | Docker Compose 部署文件 |
| `deploy/modelswitch.service` | systemd 单元文件 |
| `deploy/install-systemd.sh` | systemd 一键安装脚本 |
| `scripts/backup.sh` | 数据备份脚本 |
| `scripts/restore.sh` | 数据恢复脚本 |
| `scripts/archive-logs.sh` | 日志归档脚本 |
| `scripts/cron-setup.sh` | cron 任务安装脚本 |
| `config.example.toml` | 配置文件模板 |
