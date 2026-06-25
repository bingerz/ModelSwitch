# ModelSwitch 版本管理与回滚指南

> 涵盖版本标签策略、升级流程、回滚方案与数据库迁移安全注意事项。

## 目录

1. [版本标签策略](#版本标签策略)
2. [升级流程](#升级流程)
3. [回滚方案](#回滚方案)
4. [数据库迁移安全](#数据库迁移安全)

---

## 版本标签策略

### 版本号格式

ModelSwitch 采用语义化版本 (Semantic Versioning):

```
v<MAJOR>.<MINOR>.<PATCH>
# 例: v1.2.3
```

| 段 | 说明 | 示例 |
|----|------|------|
| MAJOR | 不兼容的 API 变更 | v**2**.0.0 |
| MINOR | 向后兼容的新功能 | v1.**3**.0 |
| PATCH | 向后兼容的修复 | v1.2.**4** |

### Docker 镜像标签

每个正式发布版本在构建时注入版本号:

```bash
# CI/CD 构建命令
docker build --build-arg VERSION=v1.2.3 -t modelswitch:v1.2.3 .

# 验证版本标签
docker inspect modelswitch:v1.2.3 | grep -i '"version"'
```

### 查看当前运行版本

```bash
# 通过 API
curl http://localhost:8080/api/gateway/info \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 通过 Docker 标签
docker inspect modelswitch | grep -i '"version"'
```

---

## 升级流程

### Docker 升级

**1. 拉取新版本镜像**

```bash
# 拉取指定版本
docker pull modelswitch:v1.3.0

# 或通过 docker-compose
export MODELSWITCH_VERSION=v1.3.0
docker compose pull
```

**2. 备份当前状态**

```bash
# 创建升级前备份
./scripts/backup.sh /var/backups/modelswitch/pre-upgrade
```

**3. 停止旧版本**

```bash
cd deploy/
docker compose down
```

**4. 启动新版本**

```bash
export MODELSWITCH_VERSION=v1.3.0
docker compose up -d
```

**5. 验证**

```bash
# 健康检查
curl http://localhost:8080/healthz

# 版本确认
curl http://localhost:8080/api/gateway/info \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# 通道连通性
curl -X POST http://localhost:8080/api/channels/test-all \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

### systemd 升级

**1. 编译新版本二进制**

```bash
cd src-tauri/
git fetch && git checkout v1.3.0
cargo build --bin modelswitch-cli --release
```

**2. 备份**

```bash
./scripts/backup.sh /var/backups/modelswitch/pre-upgrade
```

**3. 停止服务**

```bash
sudo systemctl stop modelswitch
```

**4. 替换二进制**

```bash
sudo cp target/release/modelswitch-cli /opt/modelswitch/modelswitch
sudo chown modelswitch:modelswitch /opt/modelswitch/modelswitch
sudo chmod +x /opt/modelswitch/modelswitch
```

**5. 启动并验证**

```bash
sudo systemctl start modelswitch
sudo systemctl status modelswitch
curl http://localhost:8080/healthz
```

---

## 回滚方案

### Docker 回滚

**快速回滚 (仅改变运行容器):**

```bash
cd deploy/

# 回滚到旧版本
export MODELSWITCH_VERSION=v1.2.2
docker compose up -d

# 验证
curl http://localhost:8080/healthz
```

**完整回滚 (含数据恢复):**

```bash
cd deploy/

# 1. 停止当前版本
docker compose down

# 2. 恢复升级前数据备份
./scripts/restore.sh /var/backups/modelswitch/pre-upgrade/modelswitch-backup-YYYYMMDD-HHMMSS.tar.gz

# 3. 回滚到旧版本镜像
export MODELSWITCH_VERSION=v1.2.2
docker compose up -d

# 4. 验证
curl http://localhost:8080/healthz
```

### systemd 回滚

**快速回滚:**

```bash
# 1. 停止服务
sudo systemctl stop modelswitch

# 2. 恢复旧版本二进制
sudo cp /opt/modelswitch/modelswitch.bak /opt/modelswitch/modelswitch

# 3. 启动服务
sudo systemctl start modelswitch

# 4. 验证
curl http://localhost:8080/healthz
```

> 建议: 每次升级前将旧二进制备份为 `modelswitch.bak`, 以便快速回滚。

**完整回滚 (含数据恢复):**

```bash
# 1. 停止服务
sudo systemctl stop modelswitch

# 2. 恢复数据
./scripts/restore.sh /var/backups/modelswitch/pre-upgrade/modelswitch-backup-YYYYMMDD-HHMMSS.tar.gz

# 3. 恢复旧二进制
sudo cp /opt/modelswitch/modelswitch.bak /opt/modelswitch/modelswitch

# 4. 启动并验证
sudo systemctl start modelswitch
curl http://localhost:8080/healthz
```

### 回滚决策矩阵

| 场景 | 需要数据恢复 | 操作 |
|------|-------------|------|
| 功能 bug, 无数据变更 | 否 | 仅回滚镜像/二进制 |
| 数据格式不兼容 | 是 | 回滚镜像 + 恢复备份 |
| 配置不兼容 | 否 | 回滚镜像 + 恢复 config.toml |
| 完全失败 | 是 | 回滚镜像 + 恢复完整备份 |

---

## 数据库迁移安全

ModelSwitch 使用文件存储 (JSON/NDJSON), 迁移安全性需特别注意。

### 迁移兼容性原则

1. **向前兼容**: 新版本始终能读取旧版本的数据格式
2. **向后不兼容**: 旧版本可能无法读取新版本写入的数据
3. **自动迁移**: 服务启动时自动检测并升级数据格式

### 升级前检查

**1. 确认数据格式版本**

```bash
# 查看当前数据目录
ls -la ~/.config/modelswitch/

# 查看数据格式 (如有 version 字段)
head -1 ~/.config/modelswitch/virtual_keys.json | python3 -m json.tool
```

**2. 创建完整备份**

```bash
# 必须在升级前执行
./scripts/backup.sh /var/backups/modelswitch/pre-upgrade
```

**3. 验证备份完整性**

```bash
tar tzf /var/backups/modelswitch/pre-upgrade/modelswitch-backup-*.tar.gz | head -10
```

### 迁移失败处理

如果升级后数据迁移失败:

```bash
# 1. 服务日志会显示迁移错误
sudo journalctl -u modelswitch --since "10 min ago" | grep -i migrat

# 2. 立即回滚
sudo systemctl stop modelswitch
./scripts/restore.sh /var/backups/modelswitch/pre-upgrade/modelswitch-backup-*.tar.gz

# 3. 重启旧版本
export MODELSWITCH_VERSION=v1.2.2  # Docker
# 或恢复旧二进制 (systemd)
sudo systemctl start modelswitch
```

### 最佳实践

| 实践 | 说明 |
|------|------|
| 先备份再升级 | 每次升级前创建完整备份 |
| 先测试再生产 | 先在测试环境验证升级 |
| 逐步升级 | 不要跳过多个大版本 (如 v1.x 直接到 v3.x) |
| 监控启动日志 | 确认迁移成功后再放流量 |
| 保留旧二进制 | systemd 升级时备份旧二进制 |
| 固定版本号 | 生产环境 docker-compose 中固定 `MODELSWITCH_VERSION` |
