#!/usr/bin/env bash
# Axiom benchmark runner
#
# This script wraps the real Criterion benchmark target that lives in
# benches/compositor_benchmarks.rs. It intentionally does NOT claim to perform
# synthetic stress tests or automatic regression detection outside Criterion's
# own baseline support.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BENCH_NAME="compositor_benchmarks"
CRITERION_DIR="$PROJECT_DIR/target/criterion"
LOG_DIR="$CRITERION_DIR/logs"
MODE="${1:-run}"
BASELINE_NAME="${2:-}"

mkdir -p "$LOG_DIR"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() {
    echo -e "[benchmark] $*"
}

fail() {
    echo -e "${RED}[benchmark] ERROR:${NC} $*" >&2
    exit 1
}

require_tool() {
    local tool="$1"
    command -v "$tool" >/dev/null 2>&1 || fail "required tool missing: $tool"
}

usage() {
    cat <<EOF
Axiom benchmark runner

Usage:
  $0 run
  $0 save-baseline <name>
  $0 compare <name>
  $0 help

Modes:
  run                  Run the Criterion benchmark suite once
  save-baseline <name> Run benches and save a named Criterion baseline
  compare <name>       Run benches and compare against a saved baseline
  help                 Show this help text

Examples:
  $0 run
  $0 save-baseline local-main
  $0 compare local-main

Outputs:
  - Criterion artifacts: target/criterion/
  - Wrapper logs:        target/criterion/logs/
EOF
}

run_bench() {
    local label="$1"
    shift

    local timestamp
    timestamp="$(date +%Y%m%d_%H%M%S)"
    local log_file="$LOG_DIR/${label}_${timestamp}.log"

    local cmd=(cargo bench --bench "$BENCH_NAME" -- "$@")

    log "${BLUE}Running benchmark target:${NC} $BENCH_NAME"
    log "Command: ${cmd[*]}"
    log "Log file: $log_file"

    (
        cd "$PROJECT_DIR"
        "${cmd[@]}"
    ) | tee "$log_file"

    log "${GREEN}Benchmark run complete${NC}"
    log "Criterion results: $CRITERION_DIR"
}

case "$MODE" in
    run)
        require_tool cargo
        run_bench run --noplot
        ;;
    save-baseline)
        require_tool cargo
        [[ -n "$BASELINE_NAME" ]] || fail "save-baseline requires a baseline name"
        run_bench "save_baseline_${BASELINE_NAME}" --noplot --save-baseline "$BASELINE_NAME"
        ;;
    compare)
        require_tool cargo
        [[ -n "$BASELINE_NAME" ]] || fail "compare requires a baseline name"
        run_bench "compare_${BASELINE_NAME}" --noplot --baseline "$BASELINE_NAME"
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        fail "unknown mode: $MODE (try: $0 help)"
        ;;
esac
