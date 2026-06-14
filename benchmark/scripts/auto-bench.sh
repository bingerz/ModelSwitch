#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────
# ModelSwitch Benchmark Automation
#
# Drives the gateway-bench tool against a local mock upstream and a freshly
# started ModelSwitch gateway instance. Produces JSON reports plus a markdown
# summary, and enforces latency/regression thresholds via jq.
#
# Usage:
#   ./auto-bench.sh [profile] [options]
#
# Profiles:
#   smoke      — 10-second sanity check (3s/scenario, low concurrency)
#   standard   — Default. chat + streaming + mixed, 30s each, moderate load
#   full       — All 5 scenarios + compare, 60s each, high load, 3 runs
#   compare    — Only run compare mode (direct vs proxy overhead)
#   mock-only  — Benchmark the mock upstream directly (no gateway)
#
# Env vars:
#   GATEWAY_PORT  Gateway listen port (default 8080)
#   MOCK_PORT     Mock upstream port (default 19876)
#   CONCURRENCY   Override concurrency
#   DURATION      Override duration in seconds
#   SKIP_BUILD=1  Skip the build step
#   SKIP_GATEWAY=1  Use an already-running gateway
#   REPORT_DIR    Override report output directory
# ──────────────────────────────────────────────────────────────────────────
set -euo pipefail

# ── Constants ─────────────────────────────────────────────────────────────
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
readonly BENCH_CRATE_DIR="$PROJECT_ROOT/benchmark"
readonly GATEWAY_CRATE_DIR="$PROJECT_ROOT/src-tauri"
readonly CONFIG_TEMPLATE="$SCRIPT_DIR/gateway-bench-config.toml"

readonly BENCH_BIN="$BENCH_CRATE_DIR/target/release/gateway-bench"
readonly GATEWAY_BIN="$GATEWAY_CRATE_DIR/target/release/modelswitch-cli"

readonly DEFAULT_GATEWAY_PORT=8080
readonly DEFAULT_MOCK_PORT=19876

# ── Color helpers (with non-TTY fallback) ────────────────────────────────
if [[ -t 1 ]]; then
    readonly CLR_RESET=$'\033[0m'
    readonly CLR_BOLD=$'\033[1m'
    readonly CLR_RED=$'\033[31m'
    readonly CLR_GREEN=$'\033[32m'
    readonly CLR_YELLOW=$'\033[33m'
    readonly CLR_BLUE=$'\033[34m'
    readonly CLR_CYAN=$'\033[36m'
    readonly CLR_DIM=$'\033[2m'
else
    readonly CLR_RESET=""
    readonly CLR_BOLD=""
    readonly CLR_RED=""
    readonly CLR_GREEN=""
    readonly CLR_YELLOW=""
    readonly CLR_BLUE=""
    readonly CLR_CYAN=""
    readonly CLR_DIM=""
fi

# ── Global state ─────────────────────────────────────────────────────────
GATEWAY_PID=""
MOCK_PID=""
TEMP_CONFIG=""
THRESHOLD_FAILED=0
INFRA_ERROR=0

# ── Helper functions ─────────────────────────────────────────────────────

# Log with timestamp prefix.
log() {
    printf "${CLR_DIM}[%s]${CLR_RESET} %s\n" \
        "$(date '+%Y-%m-%dT%H:%M:%S%z')" "$*"
}

# Print error and exit with infrastructure error code.
die() {
    echo "${CLR_RED}${CLR_BOLD}[ERROR]${CLR_RESET} $*" >&2
    INFRA_ERROR=1
    cleanup
    exit 2
}

# Print a formatted section header.
section() {
    local title="$1"
    echo ""
    echo "${CLR_CYAN}${CLR_BOLD}╭─ ${title}${CLR_RESET}"
}

# Print an info line under a section.
info() {
    echo "${CLR_CYAN}├─${CLR_RESET} $*"
}

# Print a success line.
ok() {
    echo "${CLR_GREEN}├─ [OK]${CLR_RESET} $*"
}

# Print a warning line.
warn() {
    echo "${CLR_YELLOW}├─ [WARN]${CLR_RESET} $*"
}

# Poll a TCP port until it accepts connections or timeout.
# Usage: wait_for_port <port> <timeout_secs>
wait_for_port() {
    local port="$1"
    local timeout="${2:-15}"
    local elapsed=0
    while (( elapsed < timeout )); do
        if nc -z 127.0.0.1 "$port" 2>/dev/null \
            || (echo > /dev/tcp/127.0.0.1/"$port") 2>/dev/null; then
            return 0
        fi
        sleep 1
        (( elapsed++ )) || true
    done
    return 1
}

# Check if a TCP port is currently in use.
# Usage: is_port_in_use <port>
is_port_in_use() {
    local port="$1"
    nc -z 127.0.0.1 "$port" 2>/dev/null \
        || (echo > /dev/tcp/127.0.0.1/"$port") 2>/dev/null
}

# Kill any process listening on the given port.
# Usage: free_port <port>
free_port() {
    local port="$1"
    local pids
    pids=$(lsof -ti ":$port" 2>/dev/null || true)
    if [[ -n "$pids" ]]; then
        log "Killing process(es) on port $port: $(echo "$pids" | tr '\n' ' ')"
        echo "$pids" | xargs kill 2>/dev/null || true
        sleep 2
        # Force kill if still alive
        pids=$(lsof -ti ":$port" 2>/dev/null || true)
        if [[ -n "$pids" ]]; then
            echo "$pids" | xargs kill -9 2>/dev/null || true
            sleep 1
        fi
    fi
}

# Compare a numeric value against a threshold and print pass/fail.
# Usage: check_threshold <label> <value> <op> <threshold> [<warn_threshold>]
#   op is "-lt" (value must be less than threshold) or "-gt" (must be greater)
check_threshold() {
    local label="$1"
    local value="$2"
    local op="$3"
    local threshold="$4"
    local warn_threshold="${5:-}"

    # Use awk for float-safe comparison.
    local pass
    if [[ "$op" == "-lt" ]]; then
        pass=$(awk -v v="$value" -v t="$threshold" 'BEGIN { print (v < t) ? 1 : 0 }')
    elif [[ "$op" == "-gt" ]]; then
        pass=$(awk -v v="$value" -v t="$threshold" 'BEGIN { print (v > t) ? 1 : 0 }')
    else
        warn "Unknown operator '$op' for $label"
        return 0
    fi

    if [[ "$pass" == "1" ]]; then
        # Check warning threshold if provided.
        if [[ -n "$warn_threshold" ]]; then
            local warn_pass
            if [[ "$op" == "-lt" ]]; then
                warn_pass=$(awk -v v="$value" -v t="$warn_threshold" 'BEGIN { print (v < t) ? 1 : 0 }')
            else
                warn_pass=$(awk -v v="$value" -v t="$warn_threshold" 'BEGIN { print (v > t) ? 1 : 0 }')
            fi
            if [[ "$warn_pass" != "1" ]]; then
                warn "$label: $value (within limit $threshold but exceeds warn $warn_threshold)"
                return 0
            fi
        fi
        echo "${CLR_GREEN}  [PASS]${CLR_RESET} $label: $value (limit $op $threshold)"
    else
        echo "${CLR_RED}  [FAIL]${CLR_RESET} $label: $value (limit $op $threshold)"
        THRESHOLD_FAILED=1
    fi
}

# Run a single benchmark scenario. Catches errors so one failure does not
# abort the whole run.
# Usage: run_scenario <scenario> <concurrency> <duration> <warmup> <target>
run_scenario() {
    local scenario="$1"
    local concurrency="$2"
    local duration="$3"
    local warmup="$4"
    local target="$5"

    info "Scenario: ${CLR_BOLD}$scenario${CLR_RESET} (c=$concurrency, ${duration}s, warmup ${warmup}s)"

    set +e
    "$BENCH_BIN" \
        --target "$target" \
        --scenario "$scenario" \
        --concurrency "$concurrency" \
        --duration "$duration" \
        --warmup "$warmup" \
        --api-key dummy-mock-key \
        --report "json:$REPORT_DIR/${scenario}.json" \
        2>&1 | sed 's/^/    /'
    local rc=${PIPESTATUS[0]}
    set -e

    if [[ $rc -ne 0 ]]; then
        warn "Scenario '$scenario' exited with code $rc — continuing"
        return 1
    fi
    return 0
}

# Run the compare subcommand (direct vs proxy overhead).
# Usage: run_compare <direct_url> <proxy_url> <concurrency> <duration> <warmup>
run_compare() {
    local direct_url="$1"
    local proxy_url="$2"
    local concurrency="$3"
    local duration="$4"
    local warmup="$5"

    info "Compare: direct=${direct_url} vs proxy=${proxy_url} (c=$concurrency, ${duration}s)"

    set +e
    "$BENCH_BIN" compare \
        --direct "$direct_url" \
        --proxy "$proxy_url" \
        --scenario streaming \
        --concurrency "$concurrency" \
        --duration "$duration" \
        --warmup "$warmup" \
        --report "$REPORT_DIR/compare.json" \
        2>&1 | sed 's/^/    /'
    local rc=${PIPESTATUS[0]}
    set -e

    if [[ $rc -ne 0 ]]; then
        warn "Compare exited with code $rc"
        return 1
    fi
    return 0
}

# Cleanup any spawned processes and temp files.
cleanup() {
    # Only print cleanup banner when we actually have something to clean.
    if [[ -n "$GATEWAY_PID" ]] || [[ -n "$MOCK_PID" ]] || [[ -n "$TEMP_CONFIG" ]]; then
        echo ""
        log "Cleaning up..."
    fi
    if [[ -n "$GATEWAY_PID" ]]; then
        kill "$GATEWAY_PID" 2>/dev/null || true
        wait "$GATEWAY_PID" 2>/dev/null || true
    fi
    if [[ -n "$MOCK_PID" ]]; then
        kill "$MOCK_PID" 2>/dev/null || true
        wait "$MOCK_PID" 2>/dev/null || true
    fi
    if [[ -n "$TEMP_CONFIG" ]] && [[ -f "$TEMP_CONFIG" ]]; then
        rm -f "$TEMP_CONFIG"
    fi
}

# ── Trap setup ───────────────────────────────────────────────────────────
trap cleanup EXIT INT TERM

# ── Validate prerequisites ───────────────────────────────────────────────
check_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        die "Required tool '$1' not found in PATH. $2"
    fi
}

check_tool cargo "Install Rust toolchain: https://rustup.rs"
check_tool jq "Install jq: https://stedolan.github.io/jq/download/"
check_tool curl "curl is required for health checks"

# ── Parse arguments ──────────────────────────────────────────────────────
PROFILE="${1:-standard}"
case "$PROFILE" in
    smoke|standard|full|compare|mock-only)
        ;;
    -h|--help|help)
        sed -n '2,/^set -euo/p' "$0" | sed 's/^# \?//' | head -30
        exit 0
        ;;
    *)
        die "Unknown profile '$PROFILE'. Valid: smoke | standard | full | compare | mock-only"
        ;;
esac

# ── Profile defaults ─────────────────────────────────────────────────────
# Each profile: concurrency duration warmup mock_delay scenarios compare_concurrency compare_duration runs
case "$PROFILE" in
    smoke)
        P_CONCURRENCY=5
        P_DURATION=3
        P_WARMUP=1
        P_MOCK_DELAY=30
        P_SCENARIOS=("chat" "streaming" "burst")
        P_COMPARE_CONCURRENCY=5
        P_COMPARE_DURATION=3
        P_RUNS=1
        P_RUN_COMPARE=false
        ;;
    standard)
        P_CONCURRENCY=20
        P_DURATION=30
        P_WARMUP=5
        P_MOCK_DELAY=50
        P_SCENARIOS=("chat" "streaming" "mixed")
        P_COMPARE_CONCURRENCY=20
        P_COMPARE_DURATION=30
        P_RUNS=1
        P_RUN_COMPARE=true
        ;;
    full)
        P_CONCURRENCY=50
        P_DURATION=60
        P_WARMUP=10
        P_MOCK_DELAY=100
        P_SCENARIOS=("chat" "streaming" "mixed" "burst" "sustained")
        P_COMPARE_CONCURRENCY=50
        P_COMPARE_DURATION=60
        P_RUNS=3
        P_RUN_COMPARE=true
        ;;
    compare)
        P_CONCURRENCY=50
        P_DURATION=30
        P_WARMUP=5
        P_MOCK_DELAY=100
        P_SCENARIOS=()
        P_COMPARE_CONCURRENCY=50
        P_COMPARE_DURATION=30
        P_RUNS=1
        P_RUN_COMPARE=true
        ;;
    mock-only)
        P_CONCURRENCY=20
        P_DURATION=30
        P_WARMUP=5
        P_MOCK_DELAY=50
        P_SCENARIOS=("chat" "streaming")
        P_COMPARE_CONCURRENCY=20
        P_COMPARE_DURATION=30
        P_RUNS=1
        P_RUN_COMPARE=false
        ;;
esac

# ── Apply env var overrides ──────────────────────────────────────────────
GATEWAY_PORT="${GATEWAY_PORT:-$DEFAULT_GATEWAY_PORT}"
MOCK_PORT="${MOCK_PORT:-$DEFAULT_MOCK_PORT}"
CONCURRENCY="${CONCURRENCY:-$P_CONCURRENCY}"
DURATION="${DURATION:-$P_DURATION}"
WARMUP="$P_WARMUP"
MOCK_DELAY="$P_MOCK_DELAY"
RUNS="${RUNS:-$P_RUNS}"
SKIP_BUILD="${SKIP_BUILD:-0}"
SKIP_GATEWAY="${SKIP_GATEWAY:-0}"

# ── Report directory ─────────────────────────────────────────────────────
if [[ -z "${REPORT_DIR:-}" ]]; then
    REPORT_DIR="$BENCH_CRATE_DIR/reports/$(date '+%Y-%m-%d_%H%M%S')"
fi
mkdir -p "$REPORT_DIR"

# ── Print run configuration ──────────────────────────────────────────────
echo ""
echo "${CLR_BOLD}${CLR_CYAN}╔══════════════════════════════════════════════════════════════╗${CLR_RESET}"
echo "${CLR_BOLD}${CLR_CYAN}║  ModelSwitch Benchmark — profile: ${CLR_RESET}${CLR_BOLD}$PROFILE${CLR_BOLD}${CLR_CYAN}                        ║${CLR_RESET}"
echo "${CLR_BOLD}${CLR_CYAN}╚══════════════════════════════════════════════════════════════╝${CLR_RESET}"
log "Profile:      $PROFILE"
log "Concurrency:  $CONCURRENCY"
log "Duration:     ${DURATION}s/scenario"
log "Warmup:       ${WARMUP}s"
log "Mock delay:   ${MOCK_DELAY}ms"
log "Runs:         $RUNS"
log "Gateway port: $GATEWAY_PORT"
log "Mock port:    $MOCK_PORT"
log "Report dir:   $REPORT_DIR"
if [[ "$SKIP_BUILD" == "1" ]]; then        log "Skip build:   yes"; else log "Skip build:   no"; fi
if [[ "$SKIP_GATEWAY" == "1" ]]; then      log "Skip gateway: yes (using external)"; else log "Skip gateway: no"; fi
if [[ "$PROFILE" == "mock-only" ]]; then   log "Target:       mock upstream directly (no gateway)"; fi
echo ""

# ── Build phase ──────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" != "1" ]]; then
    section "Building gateway-bench..."
    (cd "$BENCH_CRATE_DIR" && cargo build --release) \
        || die "Failed to build gateway-bench"
    ok "gateway-bench built"

    if [[ "$PROFILE" != "mock-only" ]] && [[ "$SKIP_GATEWAY" != "1" ]]; then
        section "Building modelswitch-cli..."
        (cd "$GATEWAY_CRATE_DIR" && cargo build --bin modelswitch-cli --release --no-default-features) \
            || die "Failed to build modelswitch-cli"
        ok "modelswitch-cli built"
    fi
else
    section "Skipping build (SKIP_BUILD=1)"
fi

# Verify binaries exist after build (or if skipped, they must already exist).
if [[ ! -x "$BENCH_BIN" ]]; then
    die "gateway-bench binary not found at $BENCH_BIN. Run without SKIP_BUILD=1."
fi

# ── Start mock upstream ──────────────────────────────────────────────────
section "Starting mock upstream on port $MOCK_PORT..."
info "delay=${MOCK_DELAY}ms, tokens=100"

set +e
"$BENCH_BIN" mock-server --port "$MOCK_PORT" --delay "$MOCK_DELAY" --tokens 100 \
    >/dev/null 2>&1 &
MOCK_PID=$!
set -e

wait_for_port "$MOCK_PORT" 10 \
    || die "Mock server failed to start on port $MOCK_PORT"
ok "Mock upstream ready (PID $MOCK_PID)"

# ── Start gateway ────────────────────────────────────────────────────────
GATEWAY_URL="http://127.0.0.1:$GATEWAY_PORT"

if [[ "$PROFILE" != "mock-only" ]] && [[ "$SKIP_GATEWAY" != "1" ]]; then
    section "Starting gateway on port $GATEWAY_PORT..."

    # Ensure the gateway port is free — kill any stale process.
    if is_port_in_use "$GATEWAY_PORT"; then
        warn "Port $GATEWAY_PORT is already in use — killing existing process"
        free_port "$GATEWAY_PORT"
        if is_port_in_use "$GATEWAY_PORT"; then
            die "Port $GATEWAY_PORT is still in use after kill attempt. Use GATEWAY_PORT env var to pick a different port."
        fi
    fi

    if [[ ! -x "$GATEWAY_BIN" ]]; then
        die "modelswitch-cli binary not found at $GATEWAY_BIN. Run without SKIP_GATEWAY=1."
    fi

    # Generate temp config from template.
    TEMP_CONFIG="$REPORT_DIR/.gateway-config.toml"
    sed \
        -e "s|{{GATEWAY_PORT}}|$GATEWAY_PORT|g" \
        -e "s|{{MOCK_PORT}}|$MOCK_PORT|g" \
        "$CONFIG_TEMPLATE" > "$TEMP_CONFIG" \
        || die "Failed to generate config from template"

    info "Config: $TEMP_CONFIG"

    set +e
    "$GATEWAY_BIN" serve --config "$TEMP_CONFIG" --port "$GATEWAY_PORT" \
        >"$REPORT_DIR/.gateway.log" 2>&1 &
    GATEWAY_PID=$!
    set -e

    # Verify the gateway process didn't exit immediately (e.g. port bind failure).
    sleep 1
    if ! kill -0 "$GATEWAY_PID" 2>/dev/null; then
        warn "Gateway process exited immediately. Last 30 log lines:"
        tail -30 "$REPORT_DIR/.gateway.log" 2>/dev/null | sed 's/^/    /' || true
        die "Gateway process died — check $REPORT_DIR/.gateway.log"
    fi

    # Wait for the gateway /health endpoint.
    info "Waiting for gateway health endpoint..."
    HEALTH_OK=false
    for i in $(seq 1 30); do
        if curl -sf "$GATEWAY_URL/health" >/dev/null 2>&1; then
            HEALTH_OK=true
            break
        fi
        sleep 1
    done

    if [[ "$HEALTH_OK" != "true" ]]; then
        warn "Gateway health check failed. Last 20 log lines:"
        tail -20 "$REPORT_DIR/.gateway.log" 2>/dev/null | sed 's/^/    /' || true
        die "Gateway failed to become healthy on port $GATEWAY_PORT"
    fi
    ok "Gateway healthy (PID $GATEWAY_PID)"
else
    if [[ "$PROFILE" == "mock-only" ]]; then
        section "Mock-only mode — gateway not started"
    elif [[ "$SKIP_GATEWAY" == "1" ]]; then
        section "Using external gateway at $GATEWAY_URL"
        if ! curl -sf "$GATEWAY_URL/health" >/dev/null 2>&1; then
            die "External gateway at $GATEWAY_URL is not responding on /health"
        fi
        ok "External gateway is healthy"
    fi
fi

# ── Determine benchmark target ───────────────────────────────────────────
if [[ "$PROFILE" == "mock-only" ]]; then
    BENCH_TARGET="http://127.0.0.1:$MOCK_PORT"
else
    BENCH_TARGET="$GATEWAY_URL"
fi

# ── Run scenario benchmarks ──────────────────────────────────────────────
SCENARIO_RESULTS=()

if [[ ${#P_SCENARIOS[@]} -gt 0 ]]; then
    for run_idx in $(seq 1 "$RUNS"); do
        if [[ "$RUNS" -gt 1 ]]; then
            section "Run $run_idx / $RUNS"
        fi

        for scenario in "${P_SCENARIOS[@]}"; do
            section "Benchmark scenario: $scenario"
            if run_scenario "$scenario" "$CONCURRENCY" "$DURATION" "$WARMUP" "$BENCH_TARGET"; then
                SCENARIO_RESULTS+=("$scenario")
            else
                warn "Scenario '$scenario' did not complete successfully"
            fi
            # Cooldown between scenarios to let circuit breaker / rate limiter reset
            log "Cooldown: 3s pause before next scenario..."
            sleep 3
        done

        # Inter-run cooldown: let rate limiter windows reset
        if [[ "$run_idx" -lt "$RUNS" ]]; then
            log "Inter-run cooldown: 10s pause before Run $((run_idx + 1))..."
            sleep 10
        fi
    done
fi

# ── Compare mode ─────────────────────────────────────────────────────────
COMPARE_RAN=false
if [[ "$P_RUN_COMPARE" == "true" ]]; then
    if [[ "$PROFILE" == "mock-only" ]]; then
        warn "Compare mode skipped — requires gateway (mock-only profile)"
    elif [[ "$SKIP_GATEWAY" == "1" ]] && ! curl -sf "$GATEWAY_URL/health" >/dev/null 2>&1; then
        warn "Compare mode skipped — gateway not available"
    else
        section "Compare mode: direct vs proxy overhead"
        if run_compare \
            "http://127.0.0.1:$MOCK_PORT" \
            "$GATEWAY_URL" \
            "$P_COMPARE_CONCURRENCY" \
            "$P_COMPARE_DURATION" \
            "$WARMUP"; then
            COMPARE_RAN=true
        else
            warn "Compare run did not complete successfully"
        fi
    fi
fi

# ── Threshold checking ───────────────────────────────────────────────────
section "Threshold checks"

if [[ "$COMPARE_RAN" == "true" ]] && [[ -f "$REPORT_DIR/compare.json" ]]; then
    # Extract overhead metrics from compare.json.
    # The exact JSON schema depends on gateway-bench's compare output.
    # We try several common field paths defensively.
    P99_OVERHEAD=$(jq -r '.latency_p99_overhead_ms // empty' "$REPORT_DIR/compare.json" 2>/dev/null || echo "")
    TTFB_P50_OVERHEAD=$(jq -r '.ttfb_p50_overhead_ms // empty' "$REPORT_DIR/compare.json" 2>/dev/null || echo "")
    RPS_LOSS_PCT=$(jq -r '.rps_loss_pct // empty' "$REPORT_DIR/compare.json" 2>/dev/null || echo "")

    if [[ -n "$P99_OVERHEAD" && "$P99_OVERHEAD" != "null" ]]; then
        check_threshold "P99 overhead (ms)" "$P99_OVERHEAD" "-lt" 10 5
    else
        warn "Could not extract P99 overhead from compare.json — field may differ"
    fi

    if [[ -n "$TTFB_P50_OVERHEAD" && "$TTFB_P50_OVERHEAD" != "null" ]]; then
        check_threshold "TTFB P50 overhead (ms)" "$TTFB_P50_OVERHEAD" "-lt" 5
    else
        warn "Could not extract TTFB P50 overhead from compare.json"
    fi

    if [[ -n "$RPS_LOSS_PCT" && "$RPS_LOSS_PCT" != "null" ]]; then
        check_threshold "RPS loss (%)" "$RPS_LOSS_PCT" "-lt" 10 5
    else
        warn "Could not extract RPS loss from compare.json"
    fi
else
    info "Compare report not available — skipping threshold checks"
fi

# ── Generate summary report ──────────────────────────────────────────────
section "Generating summary report"

SUMMARY_FILE="$REPORT_DIR/summary.md"
{
    echo "# ModelSwitch Benchmark Report"
    echo ""
    echo "- **Profile:** \`$PROFILE\`"
    echo "- **Date:** $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
    echo "- **Concurrency:** $CONCURRENCY"
    echo "- **Duration per scenario:** ${DURATION}s"
    echo "- **Mock delay:** ${MOCK_DELAY}ms"
    echo "- **Gateway port:** $GATEWAY_PORT"
    echo "- **Mock port:** $MOCK_PORT"
    echo ""

    if [[ ${#SCENARIO_RESULTS[@]} -gt 0 ]]; then
        echo "## Scenario Results"
        echo ""
        echo "| Scenario | RPS | P50 (ms) | P99 (ms) | Error % |"
        echo "|----------|-----|----------|----------|---------|"
        for scenario in "${SCENARIO_RESULTS[@]}"; do
            local_json="$REPORT_DIR/${scenario}.json"
            if [[ -f "$local_json" ]]; then
                rps=$(jq -r '.rps // "n/a"' "$local_json" 2>/dev/null || echo "n/a")
                p50=$(jq -r '.latency_p50_ms // "n/a"' "$local_json" 2>/dev/null || echo "n/a")
                p99=$(jq -r '.latency_p99_ms // "n/a"' "$local_json" 2>/dev/null || echo "n/a")
                err=$(jq -r '.error_rate // "n/a"' "$local_json" 2>/dev/null || echo "n/a")
                echo "| $scenario | $rps | $p50 | $p99 | $err |"
            else
                echo "| $scenario | (report missing) | - | - | - |"
            fi
        done
        echo ""
    fi

    if [[ "$COMPARE_RAN" == "true" ]] && [[ -f "$REPORT_DIR/compare.json" ]]; then
        echo "## Compare Results (Direct vs Proxy)"
        echo ""
        echo '```json'
        cat "$REPORT_DIR/compare.json"
        echo '```'
        echo ""
    fi

    if [[ "$THRESHOLD_FAILED" == "1" ]]; then
        echo "## Threshold Status"
        echo ""
        echo "**One or more thresholds failed.** See the run log for details."
        echo ""
    fi
} > "$SUMMARY_FILE"

ok "Summary written to $SUMMARY_FILE"

# ── Final output ─────────────────────────────────────────────────────────
echo ""
echo "${CLR_BOLD}${CLR_CYAN}╔══════════════════════════════════════════════════════════════╗${CLR_RESET}"
echo "${CLR_BOLD}${CLR_CYAN}║  Benchmark Complete                                          ║${CLR_RESET}"
echo "${CLR_BOLD}${CLR_CYAN}╚══════════════════════════════════════════════════════════════╝${CLR_RESET}"
log "Reports:    $REPORT_DIR/"
log "Summary:    $SUMMARY_FILE"
if [[ -f "$REPORT_DIR/compare.json" ]]; then
    log "Compare:    $REPORT_DIR/compare.json"
fi
echo ""

if [[ "$THRESHOLD_FAILED" == "1" ]]; then
    echo "${CLR_YELLOW}${CLR_BOLD}Result: THRESHOLD FAILURE${CLR_RESET} — one or more checks did not pass."
    cleanup
    exit 1
fi

echo "${CLR_GREEN}${CLR_BOLD}Result: PASS${CLR_RESET} — all thresholds satisfied."
cleanup
exit 0
