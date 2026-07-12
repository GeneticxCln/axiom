#!/usr/bin/env bash
# Axiom security validation helper.
#
# Keeps local and CI security checks aligned around the repository's current
# supported tools: cargo-audit and cargo-deny.

set -euo pipefail

MODE="${1:-all}"

log() {
    echo "[security-check] $*"
}

fail() {
    echo "[security-check] ERROR: $*" >&2
    exit 1
}

require_tool() {
    local tool="$1"
    command -v "$tool" >/dev/null 2>&1 || fail "required tool missing: $tool"
}

usage() {
    cat <<EOF
Axiom security validation helper

Usage:
  $0 all
  $0 audit
  $0 deny
  $0 help

Modes:
  all    Run cargo-audit and cargo-deny
  audit  Run cargo-audit with warnings denied
  deny   Run cargo-deny using deny.toml
  help   Show this help text
EOF
}

run_audit() {
    require_tool cargo
    cargo audit --help >/dev/null 2>&1 || fail "cargo-audit is not installed (try: cargo install cargo-audit --locked)"
    log "Running cargo-audit"
    cargo audit --deny warnings
}

run_deny() {
    require_tool cargo
    cargo deny --help >/dev/null 2>&1 || fail "cargo-deny is not installed (try: cargo install cargo-deny --locked)"
    log "Running cargo-deny"
    cargo deny check
}

case "$MODE" in
    all)
        run_audit
        run_deny
        ;;
    audit)
        run_audit
        ;;
    deny)
        run_deny
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        fail "unknown mode: $MODE (try: $0 help)"
        ;;
esac
