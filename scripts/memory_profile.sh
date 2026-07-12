#!/usr/bin/env bash
# Axiom memory safety runner
#
# This script provides honest, reproducible Valgrind-based checks around the
# currently supported local/CI flows. It deliberately does NOT claim support
# for heaptrack, custom allocator features, or demo/stress CLI flags unless the
# repository actually wires them.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
REPORT_DIR="$PROJECT_ROOT/memory_reports"
MODE="${1:-valgrind-tests}"
BINARY_PATH="${AXIOM_MEMORY_BINARY:-$PROJECT_ROOT/target/debug/axiom}"
TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
TEST_FILTERS=(workspace config effects)

mkdir -p "$REPORT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() {
    echo -e "[memory-check] $*"
}

fail() {
    echo -e "${RED}[memory-check] ERROR:${NC} $*" >&2
    exit 1
}

require_tool() {
    local tool="$1"
    command -v "$tool" >/dev/null 2>&1 || fail "required tool missing: $tool"
}

require_cargo_valgrind() {
    cargo valgrind --help >/dev/null 2>&1 || fail "cargo-valgrind is not installed (try: cargo install cargo-valgrind)"
}

usage() {
    cat <<EOF
Axiom memory safety runner

Usage:
  $0 valgrind-tests
  $0 valgrind-binary
  $0 help

Modes:
  valgrind-tests   Run selected library test suites under cargo-valgrind
  valgrind-binary  Run 'axiom --help' under Valgrind after a debug build
  help             Show this help text

Outputs:
  Logs and reports are written to:
    $REPORT_DIR
EOF
}

run_valgrind_test_suite() {
    local filter="$1"
    local log_file="$REPORT_DIR/valgrind_tests_${filter}_${TIMESTAMP}.log"

    log "${BLUE}Running cargo-valgrind test filter:${NC} $filter"
    log "Log file: $log_file"

    (
        cd "$PROJECT_ROOT"
        cargo valgrind test --lib "$filter" -- --test-threads=1
    ) | tee "$log_file"
}

run_valgrind_tests() {
    require_tool cargo
    require_cargo_valgrind

    for filter in "${TEST_FILTERS[@]}"; do
        run_valgrind_test_suite "$filter"
    done

    log "${GREEN}Selected cargo-valgrind test suites completed${NC}"
}

run_valgrind_binary() {
    require_tool cargo
    require_tool valgrind

    local log_file="$REPORT_DIR/valgrind_binary_${TIMESTAMP}.log"

    log "${BLUE}Building debug binary for Valgrind run${NC}"
    (
        cd "$PROJECT_ROOT"
        cargo build --bin axiom
    )

    [[ -x "$BINARY_PATH" ]] || fail "expected binary does not exist after build: $BINARY_PATH"

    log "${BLUE}Running Valgrind against:${NC} $BINARY_PATH --help"
    log "Log file: $log_file"

    valgrind \
        --leak-check=full \
        --show-leak-kinds=all \
        --track-origins=yes \
        --error-exitcode=1 \
        --log-file="$log_file" \
        "$BINARY_PATH" --help >/dev/null

    log "${GREEN}Valgrind binary check completed${NC}"
}

case "$MODE" in
    valgrind-tests)
        run_valgrind_tests
        ;;
    valgrind-binary)
        run_valgrind_binary
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        fail "unknown mode: $MODE (try: $0 help)"
        ;;
esac
