#!/bin/bash
# ══════════════════════════════════════════════════════════════════════
# ModelSwitch — systemd 安装脚本
#
# 功能:
#   1. 创建 modelswitch 系统用户与组
#   2. 复制二进制文件到 /opt/modelswitch/
#   3. 创建配置与数据目录
#   4. 安装 systemd 单元文件
#   5. 启用并启动服务
#
# 用法: sudo ./deploy/install-systemd.sh [binary-path]
#   binary-path: 编译好的 modelswitch-cli 路径 (默认: ../target/release/modelswitch-cli)
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

# ── 前置检查 ────────────────────────────────────────────────────────────
# 必须以 root 身份运行
if [[ $EUID -ne 0 ]]; then
    die "此脚本必须以 root 身份运行 (请使用 sudo)"
fi

# 脚本所在目录 (用于定位 unit 文件)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 二进制文件路径 (支持参数覆盖)
BINARY_PATH="${1:-$(realpath "$SCRIPT_DIR/../target/release/modelswitch-cli")}"

if [[ ! -f "$BINARY_PATH" ]]; then
    die "未找到二进制文件: $BINARY_PATH
请先编译: cargo build --bin modelswitch-cli --release
或通过参数指定路径: sudo $0 /path/to/modelswitch-cli"
fi

INSTALL_DIR="/opt/modelswitch"
CONFIG_DIR="/etc/modelswitch"
DATA_DIR="/var/lib/modelswitch"
UNIT_FILE="/etc/systemd/system/modelswitch.service"
UNIT_SOURCE="$SCRIPT_DIR/modelswitch.service"

# ── 步骤 1: 创建系统用户与组 ────────────────────────────────────────────
info "步骤 1/5: 创建系统用户 modelswitch..."
if id "modelswitch" &>/dev/null; then
    warn "用户 modelswitch 已存在, 跳过创建"
else
    # --system: 系统账户 (无登录 shell, 无 home 目录)
    # --shell /usr/sbin/nologin: 禁止交互登录
    useradd --system --no-create-home --shell /usr/sbin/nologin modelswitch
    success "已创建系统用户 modelswitch"
fi

# ── 步骤 2: 安装二进制文件 ──────────────────────────────────────────────
info "步骤 2/5: 安装二进制文件到 $INSTALL_DIR/..."
mkdir -p "$INSTALL_DIR"
cp "$BINARY_PATH" "$INSTALL_DIR/modelswitch-cli"
chmod 755 "$INSTALL_DIR/modelswitch-cli"
chown modelswitch:modelswitch "$INSTALL_DIR/modelswitch-cli"
success "二进制文件已安装"

# ── 步骤 3: 创建配置与数据目录 ──────────────────────────────────────────
info "步骤 3/5: 创建配置与数据目录..."
# 配置目录: 存放 config.toml
mkdir -p "$CONFIG_DIR"
# 数据目录: 存放运行时数据 (虚拟密钥、配额、预算等)
mkdir -p "$DATA_DIR"

# 设置目录权限
chown -R modelswitch:modelswitch "$CONFIG_DIR" "$DATA_DIR"
chmod 750 "$CONFIG_DIR" "$DATA_DIR"

# 如果配置文件不存在, 创建空配置提示
if [[ ! -f "$CONFIG_DIR/config.toml" ]]; then
    warn "配置文件 $CONFIG_DIR/config.toml 不存在"
    warn "请稍后从 config.example.toml 复制并修改"
fi
success "目录已创建"

# ── 步骤 4: 安装 systemd 单元 ───────────────────────────────────────────
info "步骤 4/5: 安装 systemd 单元文件..."
if [[ ! -f "$UNIT_SOURCE" ]]; then
    die "未找到 unit 文件: $UNIT_SOURCE"
fi
cp "$UNIT_SOURCE" "$UNIT_FILE"
chmod 644 "$UNIT_FILE"

# 重新加载 systemd 配置
systemctl daemon-reload
success "systemd 单元已安装"

# ── 步骤 5: 启用并启动服务 ──────────────────────────────────────────────
info "步骤 5/5: 启用并启动 modelswitch 服务..."
systemctl enable modelswitch

# 仅在配置文件存在时启动, 否则提示用户
if [[ -f "$CONFIG_DIR/config.toml" ]]; then
    systemctl restart modelswitch
    sleep 2
    if systemctl is-active --quiet modelswitch; then
        success "modelswitch 服务已启动"
    else
        warn "服务启动失败, 请检查日志: journalctl -u modelswitch -f"
    fi
else
    warn "配置文件不存在, 未自动启动服务"
    warn "请放置配置后手动启动: sudo systemctl start modelswitch"
fi

# ── 完成 ────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo -e "${GREEN} ModelSwitch 安装完成${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
echo ""
echo "常用命令:"
echo "  启动服务:   sudo systemctl start modelswitch"
echo "  停止服务:   sudo systemctl stop modelswitch"
echo "  重启服务:   sudo systemctl restart modelswitch"
echo "  查看状态:   sudo systemctl status modelswitch"
echo "  查看日志:   sudo journalctl -u modelswitch -f"
echo "  健康检查:   curl http://127.0.0.1:8080/healthz"
echo ""
echo "配置文件: $CONFIG_DIR/config.toml"
echo "数据目录: $DATA_DIR"
