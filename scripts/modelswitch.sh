#!/bin/bash
#
# ModelSwitch - 统一开发与构建工具
#
# 用法: ./scripts/modelswitch.sh <command> [options]
#
# 命令:
#   setup       初始化开发环境
#   build       编译项目 (debug/release)
#   dev         启动开发模式 (热重载)
#   run         运行已编译的应用
#   test        运行测试
#   lint        代码检查 (clippy + frontend)
#   clean       清理构建产物
#   ci          运行 CI 流程
#   release     创建发布版本
#   dist        打包分发应用
#   env         检查开发环境
#   help        显示帮助信息
#

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# 项目根目录
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND_DIR="$PROJECT_ROOT"
BACKEND_DIR="$PROJECT_ROOT/src-tauri"

# 打印函数
info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
error()   { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }
step()    { echo -e "${CYAN}==>${NC} $1"; }

# ─── 帮助 ──────────────────────────────────────────────

show_help() {
    cat << EOF
ModelSwitch - LLM 智能网关统一开发工具

用法: ./scripts/modelswitch.sh <command> [options]

命令:
  setup              初始化开发环境 (安装依赖、检查工具链)
  build [mode]       编译项目
                       - debug:   调试模式 (默认)
                       - release: 发布模式
  dev                启动开发模式 (Tauri 热重载)
  run [args...]      运行已编译的应用
  cli [args...]      运行 CLI 网关 (无 Tauri, 纯 API 代理)
  test [target]      运行测试
                       - (无参数): cargo test + pnpm 类型检查
                       - rust:    cargo test
                       - fe:      前端类型检查
                       - all:     全部
  lint               代码检查 (cargo clippy + tsc)
  clean              清理构建产物
  ci                 运行完整 CI 流程
  release            创建发布版本 (lint + test + build release)
  dist               打包分发应用 (cargo tauri build)
  env                检查开发环境
  help               显示此帮助信息

示例:
  ./scripts/modelswitch.sh setup              # 首次使用：初始化环境
  ./scripts/modelswitch.sh build              # 调试模式编译
  ./scripts/modelswitch.sh build release      # 发布模式编译
  ./scripts/modelswitch.sh dev                # 启动开发服务器
  ./scripts/modelswitch.sh cli                # 启动 CLI 网关 (无 UI)
  ./scripts/modelswitch.sh cli --port 9090    # 指定端口启动
  ./scripts/modelswitch.sh test               # 运行全部测试
  ./scripts/modelswitch.sh test rust          # 仅运行 Rust 测试
  ./scripts/modelswitch.sh lint               # 代码检查
  ./scripts/modelswitch.sh clean              # 清理构建产物
  ./scripts/modelswitch.sh dist               # 打包 macOS .dmg
  ./scripts/modelswitch.sh env                # 检查环境

项目结构:
  src-tauri/          - Rust 后端 (Tauri + Axum)
    ├── src/
    │   ├── proxy/    - 代理层 (OpenAI/Anthropic SSE)
    │   ├── router/   - 路由引擎 (Tier + 加权 + 熔断)
    │   ├── channel/  - 渠道管理 (CRUD + 凭证)
    │   └── ...
    └── Cargo.toml
  src/                - React 前端
    ├── components/   - UI 组件
    └── lib/          - API 客户端
  scripts/            - 脚本文件
  dist/               - 前端构建输出
EOF
}

# ─── 环境检查 ──────────────────────────────────────────

check_env() {
    step "检查开发环境..."

    local all_ok=true

    # Rust
    if command -v rustc &> /dev/null; then
        local rust_ver=$(rustc --version 2>/dev/null)
        success "Rust: $rust_ver"
    else
        error "未找到 rustc，请安装: https://rustup.rs"
        all_ok=false
    fi

    # Cargo
    if command -v cargo &> /dev/null; then
        local cargo_ver=$(cargo --version 2>/dev/null)
        success "Cargo: $cargo_ver"
    else
        warn "未找到 cargo"
        all_ok=false
    fi

    # Node.js
    if command -v node &> /dev/null; then
        local node_ver=$(node --version 2>/dev/null)
        success "Node.js: $node_ver"
    else
        warn "未找到 node，请安装 Node.js"
        all_ok=false
    fi

    # pnpm
    if command -v pnpm &> /dev/null; then
        local pnpm_ver=$(pnpm --version 2>/dev/null)
        success "pnpm: $pnpm_ver"
    else
        warn "未找到 pnpm，请安装: npm install -g pnpm"
        all_ok=false
    fi

    # Tauri CLI
    if command -v cargo-tauri &> /dev/null; then
        success "Tauri CLI 已安装"
    else
        warn "未找到 cargo-tauri，可通过 'cargo install tauri-cli' 安装"
        all_ok=false
    fi

    echo ""

    # 项目状态
    if [ -d "$FRONTEND_DIR/node_modules" ]; then
        success "前端依赖已安装"
    else
        info "前端依赖未安装，运行: ./scripts/modelswitch.sh setup"
    fi

    if [ -d "$BACKEND_DIR/target" ]; then
        success "Rust 构建缓存存在"
    else
        info "Rust 未编译过"
    fi

    if $all_ok; then
        success "环境检查通过"
    fi
}

# ─── 初始化 ────────────────────────────────────────────

setup_env() {
    step "初始化开发环境..."

    # 检查工具链
    check_env

    # 安装前端依赖
    if [ ! -d "$FRONTEND_DIR/node_modules" ]; then
        info "安装前端依赖 (pnpm install)..."
        cd "$FRONTEND_DIR" && pnpm install
        success "前端依赖安装完成"
    else
        info "前端依赖已安装"
    fi

    # 首次编译 Rust 后端
    if [ ! -d "$BACKEND_DIR/target" ]; then
        info "首次编译 Rust 后端 (cargo build)..."
        cd "$BACKEND_DIR" && cargo build
        success "Rust 后端编译完成"
    else
        info "Rust 后端已有构建缓存"
    fi

    echo ""
    success "开发环境初始化完成"
    info "运行 './scripts/modelswitch.sh dev' 启动开发服务器"
}

# ─── 编译 ──────────────────────────────────────────────

build_project() {
    local mode="${1:-debug}"
    info "编译项目 (mode: $mode)..."

    # 1. 构建前端
    step "[1/2] 构建前端..."
    cd "$FRONTEND_DIR"
    if [ ! -d "node_modules" ]; then
        info "安装前端依赖..."
        pnpm install
    fi
    pnpm build
    success "前端构建完成"

    # 2. 构建 Rust 后端
    step "[2/2] 构建 Rust 后端..."
    cd "$BACKEND_DIR"
    if [ "$mode" = "release" ]; then
        cargo build --release
    else
        cargo build
    fi
    success "Rust 后端编译完成 ($mode)"

    echo ""
    show_build_artifacts "$mode"
}

show_build_artifacts() {
    local mode="${1:-debug}"
    info "构建产物:"
    echo ""

    # 前端
    if [ -f "$FRONTEND_DIR/dist/index.html" ]; then
        local fe_size=$(du -sh "$FRONTEND_DIR/dist" | awk '{print $1}')
        echo "  前端: dist/ ($fe_size)"
    fi

    # 后端二进制
    local bin_name="model-switch"
    local bin_path="$BACKEND_DIR/target/$mode/$bin_name"
    if [ -f "$bin_path" ]; then
        local bin_size=$(ls -lh "$bin_path" | awk '{print $5}')
        echo "  后端: target/$mode/$bin_name ($bin_size)"
    fi

    echo ""
    info "运行: ./scripts/modelswitch.sh run"
}

# ─── 开发模式 ──────────────────────────────────────────

run_dev() {
    info "启动开发模式 (Tauri dev server)..."
    cd "$FRONTEND_DIR"

    # 确保依赖已安装
    if [ ! -d "node_modules" ]; then
        info "安装前端依赖..."
        pnpm install
    fi

    # 启动 Tauri 开发模式
    pnpm tauri dev
}

# ─── 运行 ──────────────────────────────────────────────

run_app() {
    local mode="debug"

    if [ "$1" = "--release" ]; then
        mode="release"
        shift
    fi

    local bin_path="$BACKEND_DIR/target/$mode/model-switch"

    if [ ! -f "$bin_path" ]; then
        error "未找到编译产物，请先运行: ./scripts/modelswitch.sh build"
    fi

    info "运行 ModelSwitch ($mode)..."
    exec "$bin_path" "$@"
}

# ─── 测试 ──────────────────────────────────────────────

run_tests() {
    local target="${1:-all}"

    case "$target" in
        rust)
            info "运行 Rust 测试..."
            cd "$BACKEND_DIR" && cargo test --lib --bins
            success "Rust 测试通过"
            ;;
        fe)
            info "运行前端类型检查..."
            cd "$FRONTEND_DIR" && pnpm build
            success "前端类型检查通过"
            ;;
        all|"")
            local failed=0

            step "[1/2] Rust 测试..."
            cd "$BACKEND_DIR" && cargo test --lib --bins || failed=$((failed + 1))

            step "[2/2] 前端类型检查..."
            cd "$FRONTEND_DIR" && pnpm build || failed=$((failed + 1))

            if [ "$failed" -gt 0 ]; then
                error "$failed/2 测试失败"
            else
                success "全部测试通过 (2/2)"
            fi
            ;;
        *)
            # 传递给 cargo test
            info "运行 Rust 测试: $target..."
            cd "$BACKEND_DIR" && cargo test --lib --bins "$target"
            success "测试完成"
            ;;
    esac
}

# ─── 代码检查 ──────────────────────────────────────────

run_lint() {
    info "运行代码检查..."
    local failed=0

    # Rust: cargo clippy
    step "[1/2] Rust clippy..."
    cd "$BACKEND_DIR"
    if cargo clippy -- -D warnings 2>&1; then
        success "Rust clippy 通过"
    else
        warn "Rust clippy 有警告/错误"
        failed=$((failed + 1))
    fi

    # Frontend: TypeScript 检查
    step "[2/2] 前端 TypeScript 检查..."
    cd "$FRONTEND_DIR"
    if npx tsc --noEmit 2>&1; then
        success "TypeScript 检查通过"
    else
        warn "TypeScript 有类型错误"
        failed=$((failed + 1))
    fi

    if [ "$failed" -gt 0 ]; then
        warn "代码检查发现 $failed 个问题"
    else
        success "代码检查全部通过"
    fi
}

# ─── 清理 ──────────────────────────────────────────────

clean_build() {
    info "清理构建产物..."

    # 前端
    if [ -d "$FRONTEND_DIR/dist" ]; then
        rm -rf "$FRONTEND_DIR/dist"
        info "已清理 dist/"
    fi

    # Rust
    if [ -d "$BACKEND_DIR/target" ]; then
        cd "$BACKEND_DIR" && cargo clean 2>/dev/null
        info "已清理 target/"
    fi

    # Tauri 生成
    if [ -d "$BACKEND_DIR/gen" ]; then
        rm -rf "$BACKEND_DIR/gen"
        info "已清理 gen/"
    fi

    success "清理完成"
}

# ─── CI ────────────────────────────────────────────────

run_ci() {
    step "运行 CI 流程..."

    clean_build
    check_env
    run_lint
    run_tests all
    build_project release

    success "CI 流程完成"
}

# ─── 发布 ──────────────────────────────────────────────

create_release() {
    step "创建发布版本..."

    clean_build
    run_lint
    run_tests all
    build_project release

    success "发布版本创建完成"
}

# ─── 分发打包 ──────────────────────────────────────────

create_dist() {
    step "打包分发应用 (cargo tauri build)..."

    # 确保依赖已安装
    if [ ! -d "$FRONTEND_DIR/node_modules" ]; then
        info "安装前端依赖..."
        cd "$FRONTEND_DIR" && pnpm install
    fi

    # Tauri 构建
    cd "$FRONTEND_DIR"
    pnpm tauri build

    # 显示产物
    echo ""
    info "分发产物:"

    local bundle_dir="$BACKEND_DIR/target/release/bundle"
    if [ -d "$bundle_dir" ]; then
        # macOS
        if [ -d "$bundle_dir/dmg" ]; then
            for f in "$bundle_dir/dmg/"*.dmg; do
                if [ -f "$f" ]; then
                    local size=$(ls -lh "$f" | awk '{print $5}')
                    echo "  DMG: $(basename "$f") ($size)"
                fi
            done
        fi
        if [ -d "$bundle_dir/macos" ]; then
            for f in "$bundle_dir/macos/"*.app; do
                if [ -d "$f" ]; then
                    echo "  App: $(basename "$f")"
                fi
            done
        fi
        # 二进制
        local bin="$BACKEND_DIR/target/release/model-switch"
        if [ -f "$bin" ]; then
            local bin_size=$(ls -lh "$bin" | awk '{print $5}')
            echo "  Binary: model-switch ($bin_size)"
        fi
    fi

    success "分发打包完成"
}

# ─── 主入口 ────────────────────────────────────────────

main() {
    local command="${1:-help}"
    shift 2>/dev/null || true

    case "$command" in
        setup)
            setup_env
            ;;
        build)
            build_project "$@"
            ;;
        dev)
            run_dev
            ;;
        run)
            run_app "$@"
            ;;
        cli)
            info "运行 CLI 网关模式 (无 Tauri)..."
            cd "$BACKEND_DIR" && cargo run --bin modelswitch-cli --no-default-features -- "$@"
            ;;
        test)
            run_tests "$@"
            ;;
        lint)
            run_lint
            ;;
        clean)
            clean_build
            ;;
        ci)
            run_ci
            ;;
        release)
            create_release
            ;;
        dist)
            create_dist
            ;;
        env)
            check_env
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            error "未知命令: $command\n运行 './scripts/modelswitch.sh help' 查看帮助"
            ;;
    esac
}

main "$@"
