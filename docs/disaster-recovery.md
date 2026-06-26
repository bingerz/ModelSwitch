# ModelSwitch 灾难恢复计划 (DR Plan)

## 概述

本文档定义 ModelSwitch LLM 网关在单机企业部署场景下的灾难恢复策略，
包括 RTO/RPO 目标、故障场景分类、恢复流程和验证步骤。

---

## RTO / RPO 目标

| 指标 | 目标 | 说明 |
|------|------|------|
| **RTO** (恢复时间目标) | ≤ 30 分钟 | 从故障发生到服务恢复的最长允许时间 |
| **RPO** (恢复点目标) | ≤ 10 分钟 | 最多丢失 10 分钟的数据（基于 10 秒刷盘间隔 + 最近备份） |

---

## 故障场景与恢复策略

### 场景 1: 进程崩溃 (最常见)

**症状**: ModelSwitch 进程异常退出 (OOM、panic、被 kill)

**恢复步骤**:
1. systemd 自动重启（配置了 `Restart=always`）
2. 若未自动恢复，手动执行:
   ```bash
   sudo systemctl restart modelswitch
   ```
3. 验证健康检查:
   ```bash
   curl http://127.0.0.1:8080/healthz
   ```

**预期恢复时间**: < 1 分钟（自动）或 < 5 分钟（手动）

**数据影响**: 最多丢失 10 秒的预算消费记录（刷盘间隔）

---

### 场景 2: 硬盘故障

**症状**: 数据文件损坏或不可读

**恢复步骤**:
1. 更换硬盘并挂载新存储
2. 从最近备份恢复:
   ```bash
   # 停止服务
   sudo systemctl stop modelswitch

   # 恢复最近一次完整备份
   ./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-latest.tar.gz

   # 启动服务
   sudo systemctl start modelswitch
   ```
3. 验证数据完整性:
   ```bash
   # 检查审计日志哈希链
   curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/api/audit-log | head

   # 检查虚拟密钥数量
   curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/api/virtual-keys?page=1&limit=1
   ```

**预期恢复时间**: 15-30 分钟（取决于备份大小和恢复速度）

**数据影响**: 最多丢失自上次备份以来的变更（建议每小时备份一次）

---

### 场景 3: 网络中断 (上游 API 不可达)

**症状**: DeepSeek API 无法访问，所有请求返回 502/503

**恢复步骤**:
1. 检查网络连通性:
   ```bash
   curl -I https://api.deepseek.com/v1
   ```
2. 若为公司内部网络问题，联系 IT 部门修复
3. ModelSwitch 的熔断器会自动保护，避免雪崩式重试
4. 网络恢复后，熔断器自动重置，服务恢复

**预期恢复时间**: 取决于网络故障持续时间

**数据影响**: 无数据丢失（仅请求失败）

---

### 场景 4: 数据损坏 (配置文件/密钥文件)

**症状**: JSON 解析错误，服务启动失败

**恢复步骤**:
1. 检查损坏的文件:
   ```bash
   # 尝试启动并查看错误
   journalctl -u modelswitch -f

   # 验证 JSON 完整性
   python3 -c "import json; json.load(open('~/.config/modelswitch/virtual_keys.json'))"
   ```
2. 从备份恢复损坏的文件:
   ```bash
   sudo systemctl stop modelswitch
   ./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-latest.tar.gz
   sudo systemctl start modelswitch
   ```
3. 若仅单个文件损坏，可手动替换:
   ```bash
   cp /var/backups/modelswitch/virtual_keys.json ~/.config/modelswitch/
   ```

**预期恢复时间**: 10-15 分钟

---

### 场景 5: 主机完全失效 (硬件故障)

**症状**: 主机无法启动

**恢复步骤**:
1. 在新主机上安装 ModelSwitch:
   ```bash
   # 安装二进制
   curl -L https://github.com/your-org/modelswitch/releases/latest/download/modelswitch-linux-amd64 -o /usr/local/bin/modelswitch
   chmod +x /usr/local/bin/modelswitch

   # 或使用 Docker
   docker pull modelswitch:${MODELSWITCH_VERSION}
   ```
2. 从异地备份恢复数据:
   ```bash
   # 假设备份已传输到新主机
   mkdir -p ~/.config/modelswitch
   ./scripts/restore.sh /tmp/modelswitch-backup-latest.tar.gz
   ```
3. 更新配置文件中的 API 密钥和通知设置
4. 启动服务并验证:
   ```bash
   sudo systemctl start modelswitch
   curl http://127.0.0.1:8080/healthz
   ```

**预期恢复时间**: 30-60 分钟

---

## 备份策略

### 自动备份配置

使用 cron 定时备份（已提供 `scripts/cron-setup.sh`）:

```bash
# 安装 cron 任务（每小时备份一次）
./scripts/cron-setup.sh
```

备份内容:
- `~/.config/modelswitch/virtual_keys.json` — 虚拟密钥
- `~/.config/modelswitch/quota.json` — 预算消费记录
- `~/.config/modelswitch/provider_budgets.json` — 提供商预算
- `~/.config/modelswitch/config.toml` — 网关配置
- `~/.config/modelswitch/logs/` — 调度日志 (NDJSON)
- `~/.config/modelswitch/audit.ndjson` — 审计日志

### 异地备份

建议将备份文件同步到异地存储:
- NAS / 公司文件服务器
- 云存储 (阿里云 OSS、AWS S3)
- rsync 到另一台主机

```bash
# 示例: rsync 到远程主机
rsync -avz /var/backups/modelswitch/ backup-server:/backups/modelswitch/
```

### 备份保留策略

| 类型 | 保留期限 | 说明 |
|------|---------|------|
| 每小时备份 | 24 份 | 最近 24 小时 |
| 每日备份 | 30 份 | 最近 30 天 |
| 每周备份 | 12 份 | 最近 12 周 |

`scripts/backup.sh` 自动清理过期备份（默认保留 7 天）。

---

## 验证清单

每次灾难恢复后，执行以下验证:

- [ ] `curl http://127.0.0.1:8080/healthz` 返回 200
- [ ] 管理面板可正常登录
- [ ] 虚拟密钥列表完整（数量正确）
- [ ] 测试虚拟密钥可成功调用 API
- [ ] 审计日志哈希链完整（无篡改告警）
- [ ] 预算余额正确（对比恢复前后）
- [ ] Prometheus metrics 正常暴露 (`/metrics`)
- [ ] 通知系统测试（发送测试通知）

---

## 故障转移考虑 (远期)

当前为单机部署，无自动故障转移。远期可考虑:

1. **主备模式**: 两台主机 + 共享存储 (NFS) + keepalived VIP 切换
2. **Docker Swarm / K8s**: 容器编排平台自动重启和滚动更新
3. **数据复制**: 实时复制 virtual_keys.json 和 quota.json 到备机

这些方案超出当前单机部署范围，在业务规模扩大时评估实施。
