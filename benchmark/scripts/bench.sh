#!/usr/bin/env bash
# ════════════════════════════════════════════════════════════════════════════
#  bench.sh — ModelSwitch 性能测试一键脚本
#
#  用法:
#    ./bench.sh                  # 交互式菜单
#    ./bench.sh smoke            # 10 秒快速验证
#    ./bench.sh standard         # 标准测试（默认）
#    ./bench.sh full             # 完整测试（所有场景 + 对比 + 3 次方差）
#    ./bench.sh compare          # 仅对比模式（直连 vs 代理开销）
#    ./bench.sh mock             # 仅 Mock 直连测试（不经过网关）
#    ./bench.sh list             # 列出历史报告
#    ./bench.sh clean            # 清理历史报告
#
#  环境变量覆盖:
#    CONCURRENCY=50  ./bench.sh standard
#    DURATION=60     ./bench.sh full
#    SKIP_BUILD=1    ./bench.sh smoke        # 跳过编译
#    SKIP_GATEWAY=1  ./bench.sh standard     # 使用已运行的网关
#    GATEWAY_PORT=9090 MOCK_PORT=19877  ./bench.sh standard
#
#  Makefile 快捷方式:
#    make bench-smoke | bench-standard | bench-full | bench-compare | bench-mock
# ════════════════════════════════════════════════════════════════════════════
set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly AUTO_BENCH="$SCRIPT_DIR/auto-bench.sh"
readonly REPORTS_DIR="$SCRIPT_DIR/../reports"

# ── Colors ──────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
    B='\033[1m'; CY='\033[36m'; G='\033[32m'; Y='\033[33m'; R='\033[31m'; D='\033[2m'; X='\033[0m'
else
    B=''; CY=''; G=''; Y=''; R=''; D=''; X=''
fi

# ── Profiles ────────────────────────────────────────────────────────────────
PROFILES=(
    "smoke|10 秒快速验证|3s/场景, 并发 5, 3 个场景"
    "standard|标准测试 ⭐|30s/场景, 并发 20, 3 个场景 + 对比"
    "full|完整测试|60s/场景, 并发 50, 5 个场景 + 对比 + 3 次方差"
    "compare|对比模式|直连 vs 代理开销, 30s, 并发 50"
    "mock|Mock 直连|不经过网关, 直接测试 Mock, 30s"
)

# ── Help ────────────────────────────────────────────────────────────────────
usage() {
    cat << 'EOF'
ModelSwitch 性能测试工具

用法:
  bench.sh [命令]

命令:
  smoke       10 秒快速验证 (3s/场景, 并发 5)
  standard    标准测试 — chat + streaming + mixed + 对比 (默认)
  full        完整测试 — 所有 5 个场景 + 对比 + 3 次方差分析
  compare     仅对比模式 — 直连 vs 代理开销
  mock        仅 Mock 直连 — 不经过网关
  list        列出历史测试报告
  clean       清理历史报告 (保留最近 5 次)
  help        显示此帮助信息

环境变量:
  CONCURRENCY   覆盖并发数        (例: CONCURRENCY=100)
  DURATION      覆盖持续时间(秒)  (例: DURATION=120)
  SKIP_BUILD    跳过编译          (例: SKIP_BUILD=1)
  SKIP_GATEWAY  使用已运行网关    (例: SKIP_GATEWAY=1)
  GATEWAY_PORT  网关端口          (默认: 8080)
  MOCK_PORT     Mock 端口         (默认: 19876)

示例:
  ./bench.sh                        # 交互式菜单
  ./bench.sh smoke                  # 快速冒烟测试
  CONCURRENCY=100 ./bench.sh full   # 100 并发完整测试
  SKIP_BUILD=1 ./bench.sh standard  # 跳过编译直接测试
EOF
}

# ── Interactive menu ────────────────────────────────────────────────────────
interactive_menu() {
    echo ""
    echo -e "${B}${CY}╔═══════════════════════════════════════════════════════════╗${X}"
    echo -e "${B}${CY}║     ModelSwitch 性能测试 — 选择测试模式                  ║${X}"
    echo -e "${B}${CY}╚═══════════════════════════════════════════════════════════╝${X}"
    echo ""

    local i=1
    for p in "${PROFILES[@]}"; do
        IFS='|' read -r key label desc <<< "$p"
        echo -e "  ${B}${i})${X} ${B}${label}${X}  ${D}${desc}${X}"
        ((i++))
    done

    echo -e "  ${B}q)${X} 退出"
    echo ""
    read -rp "请选择 [1-5/q]: " choice

    case "$choice" in
        1) run_profile "smoke" ;;
        2) run_profile "standard" ;;
        3) run_profile "full" ;;
        4) run_profile "compare" ;;
        5) run_profile "mock" ;;
        q|Q) echo "已退出。" ; exit 0 ;;
        *) echo -e "${R}无效选择${X}" ; exit 1 ;;
    esac
}

# ── Run a profile ───────────────────────────────────────────────────────────
run_profile() {
    local profile="$1"

    echo ""
    echo -e "${B}${CY}═══════════════════════════════════════════════════════════════${X}"
    echo -e "${B}${CY}  启动测试: ${profile}${X}"
    echo -e "${B}${CY}═══════════════════════════════════════════════════════════════${X}"

    # Show active overrides
    local overrides=""
    [[ -n "${CONCURRENCY:-}" ]] && overrides+="  CONCURRENCY=${CONCURRENCY}"
    [[ -n "${DURATION:-}" ]] && overrides+="  DURATION=${DURATION}s"
    [[ "${SKIP_BUILD:-0}" == "1" ]] && overrides+="  跳过编译"
    [[ "${SKIP_GATEWAY:-0}" == "1" ]] && overrides+="  外部网关"
    [[ -n "${GATEWAY_PORT:-}" ]] && [[ "${GATEWAY_PORT}" != "8080" ]] && overrides+="  GW:${GATEWAY_PORT}"
    if [[ -n "$overrides" ]]; then
        echo -e "${D}覆盖参数:${overrides}${X}"
    fi
    echo ""

    exec "$AUTO_BENCH" "$profile"
}

# ── List reports ────────────────────────────────────────────────────────────
list_reports() {
    if [[ ! -d "$REPORTS_DIR" ]] || [[ -z "$(ls -A "$REPORTS_DIR" 2>/dev/null)" ]]; then
        echo -e "${Y}暂无测试报告${X}"
        return
    fi

    echo -e "${B}历史测试报告:${X}"
    echo ""

    local latest_shown=false
    while IFS= read -r dir; do
        local name="$(basename "$dir")"
        local summary="$dir/summary.md"
        local time_ago=""

        # Parse timestamp from dirname (YYYY-MM-DD_HHMMSS)
        if [[ "$name" =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})_([0-9]{6})$ ]]; then
            local ts="${BASH_REMATCH[1]}-${BASH_REMATCH[2]}-${BASH_REMATCH[3]} ${BASH_REMATCH[4]:0:2}:${BASH_REMATCH[4]:2:2}:${BASH_REMATCH[4]:4:2}"
            time_ago="$(get_time_ago "$ts")"
        fi

        # Extract key metrics from summary
        local profile="" chat_rps="" chat_p99=""
        if [[ -f "$summary" ]]; then
            profile=$(grep 'Profile' "$summary" 2>/dev/null | head -1 | sed 's/.*`\(.*\)`.*/\1/' || echo "?")
        fi

        local marker=" "
        if ! $latest_shown; then
            marker="${G}★${X}"
            latest_shown=true
        else
            marker=" "
        fi

        printf "  ${G}%-2s${X}  %-20s  %-10s  %s\n" "$marker" "$name" "[$profile]" "$time_ago"
    done < <(ls -1d "$REPORTS_DIR"/*/ 2>/dev/null | sort -r)

    echo ""
    echo -e "${D}  ★ = 最新    查看报告: cat $REPORTS_DIR/<timestamp>/summary.md${X}"
}

# ── Clean old reports ───────────────────────────────────────────────────────
clean_reports() {
    if [[ ! -d "$REPORTS_DIR" ]]; then
        echo "无报告需要清理"
        return
    fi

    local dirs=()
    while IFS= read -r d; do
        dirs+=("$d")
    done < <(ls -1d "$REPORTS_DIR"/*/ 2>/dev/null | sort -r)

    local total=${#dirs[@]}
    if [[ $total -le 5 ]]; then
        echo -e "${G}只有 ${total} 个报告，无需清理 (保留最近 5 个)${X}"
        return
    fi

    local remove_count=$((total - 5))
    echo -e "${Y}将删除 ${remove_count} 个旧报告 (保留最近 5 个):${X}"

    local i=0
    for dir in $(printf '%s\n' "${dirs[@]}" | tac); do
        if [[ $i -ge 5 ]]; then
            local name="$(basename "$dir")"
            echo "  删除: $name"
            rm -rf "$dir"
        fi
        ((i++))
    done

    echo -e "${G}清理完成${X}"
}

# ── Helper: human-readable time ago ─────────────────────────────────────────
get_time_ago() {
    local target="$1"
    local target_epoch
    target_epoch=$(date -j -f "%Y-%m-%d %H:%M:%S" "$target" "+%s" 2>/dev/null || date -d "$target" "+%s" 2>/dev/null || echo 0)
    [[ "$target_epoch" == "0" ]] && return

    local now_epoch
    now_epoch=$(date "+%s")
    local diff=$((now_epoch - target_epoch))

    if [[ $diff -lt 60 ]]; then
        echo "${diff} 秒前"
    elif [[ $diff -lt 3600 ]]; then
        echo "$((diff / 60)) 分钟前"
    elif [[ $diff -lt 86400 ]]; then
        echo "$((diff / 3600)) 小时前"
    else
        echo "$((diff / 86400)) 天前"
    fi
}

# ── Main ────────────────────────────────────────────────────────────────────
main() {
    local cmd="${1:-}"

    case "$cmd" in
        ""|menu)
            interactive_menu
            ;;
        smoke|standard|full|compare|mock-only|mock)
            # Normalize "mock" -> "mock-only"
            [[ "$cmd" == "mock" ]] && cmd="mock-only"
            run_profile "$cmd"
            ;;
        list|ls|reports)
            list_reports
            ;;
        clean|cleanup)
            clean_reports
            ;;
        help|-h|--help|h)
            usage
            ;;
        *)
            echo -e "${R}未知命令: $cmd${X}"
            echo ""
            usage
            exit 1
            ;;
    esac
}

main "$@"
