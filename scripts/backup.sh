#!/bin/bash
# ══════════════════════════════════════════════════════════════════════
# ModelSwitch — 数据备份脚本
#
# 功能:
#   - 将 ModelSwitch 数据目录打包为带时间戳的 tar.gz 归档
#   - 包含: 虚拟密钥、配额、提供商预算、日志、审计、配置文件
#   - 自动清理 30 天前的旧备份
#   - 记录备份操作日志
#
# 用法: ./scripts/backup.sh <备份目标目录>
# 示例: ./scripts/backup.sh /var/backups/modelswitch
# ══════════════════════════════════════════════════════════════════════
set -euo pipefail

# ── 颜色输出 ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
die()     { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# ── 参数检查 ────────────────────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
    echo "用法: $0 <备份目标目录>"
    echo "示例: $0 /var/backups/modelswitch"
    exit 1
fi

BACKUP_DEST="$1"
# 数据目录 (默认: ~/.config/modelswitch)
DATA_DIR="${MODELSWITCH_DATA_DIR:-$HOME/.config/modelswitch}"
# 备份保留天数
RETENTION_DAYS=30
# 日志文件路径
LOG_FILE="${MODELSWITCH_BACKUP_LOG:-$BACKUP_DEST/backup.log}"

# 时间戳 (格式: YYYYMMDD-HHMMSS)
TIMESTAMP=$(date +"%Y%m%d-%H%M%S")
BACKUP_FILE="$BACKUP_DEST/modelswitch-backup-${TIMESTAMP}.tar.gz"

# ── 前置检查 ────────────────────────────────────────────────────────────
# 数据目录必须存在
if [[ ! -d "$DATA_DIR" ]]; then
    die "数据目录不存在: $DATA_DIR"
fi

# 创建备份目标目录
mkdir -p "$BACKUP_DEST"

info "开始备份 ModelSwitch 数据"
info "数据目录: $DATA_DIR"
info "备份文件: $BACKUP_FILE"
info "保留策略: ${RETENTION_DAYS} 天"

# ── 执行备份 ────────────────────────────────────────────────────────────
# 需要备份的文件模式:
#   - virtual_keys.json:    虚拟密钥数据
#   - quota.json:           配额数据
#   - provider_budgets.json: 提供商预算
#   - logs.ndjson*:          调度日志 (支持轮转后缀)
#   - audit.ndjson:          审计日志
#   - config.toml:           主配置文件
BACKUP_PATTERNS=(
    "virtual_keys.json"
    "quota.json"
    "provider_budgets.json"
    "logs.ndjson"
    "logs.ndjson.*"
    "audit.ndjson"
    "audit.ndjson.*"
    "config.toml"
)

# 切换到数据目录, 使 tar 使用相对路径
cd "$DATA_DIR"

# 收集实际存在的文件
FILES_TO_BACKUP=()
for pattern in "${BACKUP_PATTERNS[@]}"; do
    # shellcheck disable=SC2045
    for f in $(ls -1 $pattern 2>/dev/null || true); do
        FILES_TO_BACKUP+=("$f")
    done
done

if [[ ${#FILES_TO_BACKUP[@]} -eq 0 ]]; then
    warn "数据目录中没有找到可备份的文件: $DATA_DIR"
    die "无数据可备份, 请确认数据目录路径"
fi

info "待备份文件 (${#FILES_TO_BACKUP[@]} 个): ${FILES_TO_BACKUP[*]}"

# 打包压缩
# -c: 创建归档  -z: gzip 压缩  -f: 指定文件名
tar -czf "$BACKUP_FILE" "${FILES_TO_BACKUP[@]}"

# 验证归档完整性
if ! tar -tzf "$BACKUP_FILE" >/dev/null 2>&1; then
    die "备份文件验证失败, 归档可能损坏: $BACKUP_FILE"
fi

# 获取备份文件大小 (人类可读)
BACKUP_SIZE=$(du -h "$BACKUP_FILE" | cut -f1)
success "备份完成: $BACKUP_FILE ($BACKUP_SIZE)"

# ── 清理旧备份 ──────────────────────────────────────────────────────────
info "清理 ${RETENTION_DAYS} 天前的旧备份..."
DELETED_COUNT=0
# find 查找超过保留期的备份文件并删除
while IFS= read -r -d '' old_file; do
    rm -f "$old_file"
    info "已删除旧备份: $(basename "$old_file")"
    DELETED_COUNT=$((DELETED_COUNT + 1))
done < <(find "$BACKUP_DEST" -name "modelswitch-backup-*.tar.gz" -mtime +"$RETENTION_DAYS" -print0)

if [[ $DELETED_COUNT -eq 0 ]]; then
    info "没有需要清理的旧备份"
else
    success "已清理 $DELETED_COUNT 个旧备份"
fi

# ── 记录日志 ────────────────────────────────────────────────────────────
LOG_TIMESTAMP=$(date +"%Y-%m-%d %H:%M:%S")
echo "[$LOG_TIMESTAMP] 备份成功 文件=$BACKUP_FILE 大小=$BACKUP_SIZE 文件数=${#FILES_TO_BACKUP[@]} 清理旧备份=$DELETED_COUNT" >> "$LOG_FILE"

# ── 输出摘要 ────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN} 备份完成${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo "备份文件:   $BACKUP_FILE"
echo "文件大小:   $BACKUP_SIZE"
echo "包含文件数: ${#FILES_TO_BACKUP[@]}"
echo "清理旧备份: $DELETED_COUNT"
echo "日志文件:   $LOG_FILE"
