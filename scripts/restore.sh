#!/bin/bash
# ══════════════════════════════════════════════════════════════════════
# ModelSwitch — 数据恢复脚本
#
# 功能:
#   - 从备份归档恢复 ModelSwitch 数据
#   - 恢复前自动停止运行中的服务 (systemd 或 docker)
#   - 恢复前要求用户确认, 防止误操作
#   - 恢复完成后可选择自动重启服务
#
# 用法: ./scripts/restore.sh <备份文件路径>
# 示例: ./scripts/restore.sh /var/backups/modelswitch/modelswitch-backup-20260625-120000.tar.gz
# ══════════════════════════════════════════════════════════════════════
set -euo pipefail

# ── 颜色输出 ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
die()     { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# ── 参数检查 ────────────────────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
    echo "用法: $0 <备份文件路径>"
    echo "示例: $0 /var/backups/modelswitch/modelswitch-backup-20260625-120000.tar.gz"
    exit 1
fi

BACKUP_FILE="$1"
# 数据目录 (默认: ~/.config/modelswitch)
DATA_DIR="${MODELSWITCH_DATA_DIR:-$HOME/.config/modelswitch}"

# ── 前置检查 ────────────────────────────────────────────────────────────
# 备份文件必须存在且可读
if [[ ! -f "$BACKUP_FILE" ]]; then
    die "备份文件不存在: $BACKUP_FILE"
fi

# 验证文件扩展名
if [[ "$BACKUP_FILE" != *.tar.gz && "$BACKUP_FILE" != *.tgz ]]; then
    die "备份文件必须是 .tar.gz 或 .tgz 格式"
fi

# 验证归档完整性
info "验证备份文件完整性..."
if ! tar -tzf "$BACKUP_FILE" >/dev/null 2>&1; then
    die "备份文件损坏或格式错误: $BACKUP_FILE"
fi
success "备份文件验证通过"

# 列出备份内容
info "备份文件包含以下内容:"
tar -tzf "$BACKUP_FILE" | sed 's/^/  /'

# ── 确认提示 ────────────────────────────────────────────────────────────
echo ""
echo -e "${YELLOW}═══════════════════════════════════════════════════════${NC}"
echo -e "${YELLOW} 警告: 即将恢复数据${NC}"
echo -e "${YELLOW}═══════════════════════════════════════════════════════${NC}"
echo "备份文件: $BACKUP_FILE"
echo "目标目录: $DATA_DIR"
echo ""
echo "此操作将覆盖目标目录中的同名文件!"
echo ""
read -r -p $'确认恢复? (输入 yes 继续): ' CONFIRMATION

if [[ "$CONFIRMATION" != "yes" ]]; then
    info "用户取消, 未执行恢复"
    exit 0
fi

# ── 停止服务 ────────────────────────────────────────────────────────────
SERVICE_STOPPED=""
CONTAINER_STOPPED=""

info "检查并停止运行中的 ModelSwitch 服务..."

# 检查 systemd 服务
if systemctl is-active --quiet modelswitch 2>/dev/null; then
    info "停止 systemd 服务 modelswitch..."
    sudo systemctl stop modelswitch
    SERVICE_STOPPED="yes"
    success "systemd 服务已停止"
fi

# 检查 docker 容器
if command -v docker &>/dev/null; then
    if docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^modelswitch$"; then
        info "停止 docker 容器 modelswitch..."
        docker stop modelswitch
        CONTAINER_STOPPED="yes"
        success "docker 容器已停止"
    fi
fi

if [[ -z "$SERVICE_STOPPED" && -z "$CONTAINER_STOPPED" ]]; then
    info "未检测到运行中的服务"
fi

# 确保目录存在
mkdir -p "$DATA_DIR"

# ── 执行恢复 ────────────────────────────────────────────────────────────
info "开始恢复数据到 $DATA_DIR ..."
cd "$DATA_DIR"
tar -xzf "$BACKUP_FILE"
success "数据恢复完成"

# 列出恢复的文件
info "已恢复的文件:"
ls -la "$DATA_DIR"

# ── 重启服务 ────────────────────────────────────────────────────────────
# 恢复后询问是否重启服务
echo ""
if [[ -n "$SERVICE_STOPPED" || -n "$CONTAINER_STOPPED" ]]; then
    read -r -p $'是否重新启动服务? (输入 yes 启动): ' RESTART_CONFIRM

    if [[ "$RESTART_CONFIRM" == "yes" ]]; then
        if [[ -n "$SERVICE_STOPPED" ]]; then
            info "启动 systemd 服务 modelswitch..."
            sudo systemctl start modelswitch
            sleep 2
            if systemctl is-active --quiet modelswitch; then
                success "systemd 服务已启动"
            else
                warn "服务启动失败, 请检查: journalctl -u modelswitch -f"
            fi
        fi

        if [[ -n "$CONTAINER_STOPPED" ]]; then
            info "启动 docker 容器 modelswitch..."
            docker start modelswitch
            success "docker 容器已启动"
        fi
    else
        warn "服务未重启, 请手动启动"
    fi
fi

# ── 完成 ────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN} 恢复完成${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo "备份来源: $BACKUP_FILE"
echo "恢复目录: $DATA_DIR"
echo ""
echo "建议执行健康检查: curl http://127.0.0.1:8080/healthz"
