#!/usr/bin/env bash
# Validate the repository's current XWayland coverage.
#
# This helper can run the lifecycle-only test or the stronger real-client test
# that launches a real X11 utility against the spawned XWayland server.
# If no parent Wayland compositor is available, it can start a temporary
# headless Weston instance when `weston` is installed.

set -euo pipefail

MODE="${1:-all}"
AXIOM_BINARY="${2:-./target/debug/axiom}"
TMP_ROOT=""
WESTON_PID=""
STARTED_WESTON="false"
ORIG_XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-}"
ORIG_WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-}"

log() {
    echo "[xwayland-check] $*"
}

cleanup() {
    local exit_code=$?
    if [[ -n "$WESTON_PID" ]] && kill -0 "$WESTON_PID" 2>/dev/null; then
        kill -TERM "$WESTON_PID" 2>/dev/null || true
        wait "$WESTON_PID" 2>/dev/null || true
    fi
    if [[ "$STARTED_WESTON" == "true" ]]; then
        if [[ -n "$ORIG_XDG_RUNTIME_DIR" ]]; then
            export XDG_RUNTIME_DIR="$ORIG_XDG_RUNTIME_DIR"
        else
            unset XDG_RUNTIME_DIR || true
        fi
        if [[ -n "$ORIG_WAYLAND_DISPLAY" ]]; then
            export WAYLAND_DISPLAY="$ORIG_WAYLAND_DISPLAY"
        else
            unset WAYLAND_DISPLAY || true
        fi
    fi
    if [[ -n "$TMP_ROOT" ]]; then
        rm -rf "$TMP_ROOT"
    fi
    return "$exit_code"
}
trap cleanup EXIT

usage() {
    cat <<EOF
Axiom XWayland validation helper

Usage:
  $0 lifecycle
  $0 real-client
  $0 metadata
  $0 xwm
  $0 end-to-end [axiom-binary]
  $0 all [axiom-binary]
  $0 help

Modes:
  lifecycle    Run the XWayland lifecycle-focused Rust test
  real-client  Run the real X11 client smoke test against XWayland
  metadata     Run the real X11 metadata smoke test against XWayland
  xwm          Run the compositor-side XWM wiring smoke test
  end-to-end   Run the full X11-in-Axiom smoke script
  all          Run lifecycle, real-client, metadata, XWM, and end-to-end checks
  help         Show this help text
EOF
}

require_tool() {
    local tool="$1"
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "[xwayland-check] ERROR: required tool missing: $tool" >&2
        exit 1
    fi
}

ensure_parent_wayland() {
    if [[ -n "${WAYLAND_DISPLAY:-}" ]] && [[ -n "${XDG_RUNTIME_DIR:-}" ]] && [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
        log "Using existing Wayland session: $WAYLAND_DISPLAY"
        return 0
    fi

    if ! command -v weston >/dev/null 2>&1; then
        log "No parent Wayland compositor available and weston is not installed; tests may skip"
        return 0
    fi

    TMP_ROOT="$(mktemp -d)"
    export XDG_RUNTIME_DIR="$TMP_ROOT/runtime"
    mkdir -p "$XDG_RUNTIME_DIR"
    chmod 700 "$XDG_RUNTIME_DIR"
    export WAYLAND_DISPLAY="weston-axiom-xwayland-$$"

    log "Starting temporary headless Weston on socket $WAYLAND_DISPLAY"
    weston \
        --backend=headless-backend.so \
        --socket="$WAYLAND_DISPLAY" \
        --idle-time=0 \
        --width=1024 \
        --height=768 >"$TMP_ROOT/weston.log" 2>&1 &
    WESTON_PID=$!
    STARTED_WESTON="true"

    local deadline=$((SECONDS + 10))
    while (( SECONDS < deadline )); do
        if [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
            log "Headless Weston is ready"
            return 0
        fi
        if ! kill -0 "$WESTON_PID" 2>/dev/null; then
            log "Weston exited before becoming ready"
            if [[ -f "$TMP_ROOT/weston.log" ]]; then
                echo "----- weston.log -----" >&2
                tail -n 80 "$TMP_ROOT/weston.log" >&2 || true
            fi
            return 0
        fi
        sleep 0.2
    done

    log "Timed out waiting for Weston socket; XWayland tests may skip"
}

run_named_test() {
    local test_name="$1"
    require_tool cargo
    if ! command -v Xwayland >/dev/null 2>&1; then
        log "Xwayland binary not found; skipping $test_name locally"
        return 0
    fi

    ensure_parent_wayland

    log "Running Rust test: $test_name"
    cargo test --lib "$test_name" --all-features -- --test-threads=1
}

run_end_to_end_smoke() {
    if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
        if command -v xvfb-run >/dev/null 2>&1; then
            log "No desktop session detected; using xvfb-run for end-to-end smoke"
            xvfb-run -a bash ./scripts/xwayland_end_to_end_smoke.sh "$AXIOM_BINARY"
            return 0
        fi
        log "No desktop session or xvfb-run available; skipping end-to-end smoke locally"
        return 0
    fi

    bash ./scripts/xwayland_end_to_end_smoke.sh "$AXIOM_BINARY"
}

case "$MODE" in
    lifecycle)
        run_named_test test_xwayland_manager_lifecycle
        ;;
    real-client)
        run_named_test test_xwayland_manager_accepts_real_x11_client
        ;;
    metadata)
        run_named_test test_xwayland_real_x11_client_metadata
        ;;
    xwm)
        run_named_test test_xwayland_manager_wired_xwm_receives_map_events
        ;;
    end-to-end)
        run_end_to_end_smoke
        ;;
    all)
        run_named_test test_xwayland_manager_lifecycle
        run_named_test test_xwayland_manager_accepts_real_x11_client
        run_named_test test_xwayland_real_x11_client_metadata
        run_named_test test_xwayland_manager_wired_xwm_receives_map_events
        run_end_to_end_smoke
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        echo "[xwayland-check] ERROR: unknown mode '$MODE'" >&2
        usage >&2
        exit 1
        ;;
esac
