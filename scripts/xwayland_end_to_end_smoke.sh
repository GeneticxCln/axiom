#!/usr/bin/env bash
# End-to-end X11-in-Axiom smoke test.
#
# What this verifies:
# 1. Axiom starts in nested/windowed mode.
# 2. Axiom announces its Wayland socket and wires compositor-side XWM support.
# 3. XWayland starts against Axiom's own Wayland socket.
# 4. A real X11 client connects through XWayland.
# 5. Axiom logs that the X11 window mapped as a compositor window.
# 6. Client teardown removes the compositor-managed X11 window cleanly.
# 7. Axiom shuts down cleanly.

set -euo pipefail

BINARY_PATH="${1:-${AXIOM_BIN:-./target/debug/axiom}}"
STARTUP_TIMEOUT_SECS="${AXIOM_X11_SMOKE_STARTUP_TIMEOUT_SECS:-40}"
MAP_TIMEOUT_SECS="${AXIOM_X11_SMOKE_MAP_TIMEOUT_SECS:-20}"
TEARDOWN_TIMEOUT_SECS="${AXIOM_X11_SMOKE_TEARDOWN_TIMEOUT_SECS:-15}"
KEEP_LOGS="${AXIOM_X11_SMOKE_KEEP_LOGS:-false}"
X11_CLIENT_RAW="${AXIOM_X11_SMOKE_CLIENT:-xmessage}"
EXPECTED_TITLE="${AXIOM_X11_SMOKE_TITLE:-Axiom X11 End-to-End Smoke}"
EXPECTED_INSTANCE="${AXIOM_X11_SMOKE_INSTANCE:-axiom-x11-e2e-smoke}"

TMP_ROOT="$(mktemp -d)"
AXIOM_LOG="$TMP_ROOT/axiom.log"
CLIENT_LOG="$TMP_ROOT/x11-client.log"
CONFIG_PATH="$TMP_ROOT/axiom-x11-smoke.toml"

AXIOM_PID=""
CLIENT_PID=""
SOCKET_NAME=""
X11_DISPLAY=""
ORIGINAL_XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-}"
ORIGINAL_WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-}"

log() {
    echo "[axiom-x11-smoke] $*"
}

print_log_tail() {
    local label="$1"
    local path="$2"
    if [[ -f "$path" ]]; then
        echo "----- $label (tail) -----" >&2
        tail -n 100 "$path" >&2 || true
    fi
}

fail() {
    local message="$1"
    echo "[axiom-x11-smoke] ERROR: $message" >&2
    print_log_tail "Axiom log" "$AXIOM_LOG"
    print_log_tail "X11 client log" "$CLIENT_LOG"
    exit 1
}

cleanup() {
    local exit_code=$?

    if [[ -n "$CLIENT_PID" ]] && kill -0 "$CLIENT_PID" 2>/dev/null; then
        kill -TERM "$CLIENT_PID" 2>/dev/null || true
        wait "$CLIENT_PID" 2>/dev/null || true
    fi

    if [[ -n "$AXIOM_PID" ]] && kill -0 "$AXIOM_PID" 2>/dev/null; then
        kill -TERM "$AXIOM_PID" 2>/dev/null || true
        wait "$AXIOM_PID" 2>/dev/null || true
    fi

    if [[ -n "$ORIGINAL_XDG_RUNTIME_DIR" ]]; then
        export XDG_RUNTIME_DIR="$ORIGINAL_XDG_RUNTIME_DIR"
    else
        unset XDG_RUNTIME_DIR || true
    fi
    if [[ -n "$ORIGINAL_WAYLAND_DISPLAY" ]]; then
        export WAYLAND_DISPLAY="$ORIGINAL_WAYLAND_DISPLAY"
    else
        unset WAYLAND_DISPLAY || true
    fi

    if [[ "$KEEP_LOGS" == "true" ]]; then
        log "Preserving smoke-test logs in $TMP_ROOT"
    else
        rm -rf "$TMP_ROOT"
    fi

    return "$exit_code"
}
trap cleanup EXIT

wait_for_pattern() {
    local file="$1"
    local pattern="$2"
    local timeout_secs="$3"
    local description="$4"
    local deadline=$((SECONDS + timeout_secs))

    while (( SECONDS < deadline )); do
        if [[ -f "$file" ]] && grep -Fq -- "$pattern" "$file"; then
            return 0
        fi

        if [[ -n "$AXIOM_PID" ]] && ! kill -0 "$AXIOM_PID" 2>/dev/null; then
            wait "$AXIOM_PID" || true
            fail "Axiom exited while waiting for: $description"
        fi

        sleep 0.2
    done

    fail "Timed out waiting for: $description"
}

wait_for_process_exit() {
    local pid="$1"
    local timeout_secs="$2"
    local description="$3"
    local deadline=$((SECONDS + timeout_secs))

    while (( SECONDS < deadline )); do
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" 2>/dev/null || true
            return 0
        fi
        sleep 0.2
    done

    fail "Timed out waiting for process exit: $description"
}

if [[ ! -x "$BINARY_PATH" ]]; then
    if ! command -v cargo >/dev/null 2>&1; then
        fail "Axiom binary not found at $BINARY_PATH and cargo is unavailable"
    fi
    if [[ "$BINARY_PATH" == *"/release/"* ]]; then
        log "Building release binary because $BINARY_PATH is missing"
        cargo build --release --bin axiom
    else
        log "Building debug binary because $BINARY_PATH is missing"
        cargo build --bin axiom
    fi
fi

if [[ ! -x "$BINARY_PATH" ]]; then
    fail "Axiom binary is not executable: $BINARY_PATH"
fi

read -r -a X11_CLIENT <<< "$X11_CLIENT_RAW"
if [[ ${#X11_CLIENT[@]} -eq 0 ]]; then
    fail "No X11 smoke client configured"
fi
if ! command -v "${X11_CLIENT[0]}" >/dev/null 2>&1; then
    fail "Required X11 client is missing: ${X11_CLIENT[0]}"
fi
if ! command -v Xwayland >/dev/null 2>&1; then
    fail "Xwayland binary is required for this smoke test"
fi

if [[ -n "${DISPLAY:-}" ]]; then
    export WINIT_UNIX_BACKEND="${WINIT_UNIX_BACKEND:-x11}"
    export XDG_RUNTIME_DIR="$TMP_ROOT/runtime"
    mkdir -p "$XDG_RUNTIME_DIR"
    chmod 700 "$XDG_RUNTIME_DIR"
    log "Using X11 host backend via DISPLAY with isolated XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
elif [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
    export WINIT_UNIX_BACKEND="${WINIT_UNIX_BACKEND:-wayland}"
    if [[ -z "$ORIGINAL_XDG_RUNTIME_DIR" ]]; then
        fail "WAYLAND_DISPLAY is set but XDG_RUNTIME_DIR is missing"
    fi
    log "Using Wayland host backend with existing XDG_RUNTIME_DIR=$ORIGINAL_XDG_RUNTIME_DIR"
else
    fail "No DISPLAY or WAYLAND_DISPLAY found. Run inside a desktop session or wrap with xvfb-run -a."
fi

cat > "$CONFIG_PATH" <<'EOF'
[xwayland]
enabled = true
EOF

LAUNCH_CMD=("$BINARY_PATH" "--config" "$CONFIG_PATH" "--windowed" "--debug")
if command -v stdbuf >/dev/null 2>&1; then
    LAUNCH_CMD=(stdbuf -oL -eL "${LAUNCH_CMD[@]}")
fi

log "Launching Axiom: ${LAUNCH_CMD[*]}"
"${LAUNCH_CMD[@]}" >"$AXIOM_LOG" 2>&1 &
AXIOM_PID=$!

wait_for_pattern "$AXIOM_LOG" "Wayland socket:" "$STARTUP_TIMEOUT_SECS" "Wayland socket announcement"
wait_for_pattern "$AXIOM_LOG" "Wiring XWM into backend" "$STARTUP_TIMEOUT_SECS" "compositor-side XWM wiring"
wait_for_pattern "$AXIOM_LOG" "XWayland server started successfully on :" "$STARTUP_TIMEOUT_SECS" "XWayland startup"

SOCKET_NAME="$(sed -nE 's/.*Wayland socket: ([^[:space:]]+).*/\1/p' "$AXIOM_LOG" | tail -n 1)"
[[ -n "$SOCKET_NAME" ]] || fail "Could not extract Wayland socket name from Axiom logs"
log "Axiom announced nested Wayland socket: $SOCKET_NAME"

X11_DISPLAY="$(sed -nE 's/.*XWayland server started successfully on :([0-9]+).*/\1/p' "$AXIOM_LOG" | tail -n 1)"
[[ -n "$X11_DISPLAY" ]] || fail "Could not extract XWayland display number from Axiom logs"
log "Axiom/XWayland announced X11 display :$X11_DISPLAY"

log "Launching real X11 client: ${X11_CLIENT[*]}"
env DISPLAY=":$X11_DISPLAY" \
    "${X11_CLIENT[@]}" -name "$EXPECTED_INSTANCE" -title "$EXPECTED_TITLE" \
    "Axiom compositor X11 end-to-end smoke test" >"$CLIENT_LOG" 2>&1 &
CLIENT_PID=$!

wait_for_pattern "$AXIOM_LOG" "mapped as compositor window" "$MAP_TIMEOUT_SECS" "compositor-side X11 map event"
wait_for_pattern "$AXIOM_LOG" "$EXPECTED_TITLE" "$MAP_TIMEOUT_SECS" "expected X11 title in compositor logs"
log "Axiom observed a real X11 window mapped through XWayland"

if kill -0 "$CLIENT_PID" 2>/dev/null; then
    log "Stopping X11 client"
    kill -TERM "$CLIENT_PID" 2>/dev/null || true
fi
wait_for_process_exit "$CLIENT_PID" "$TEARDOWN_TIMEOUT_SECS" "X11 client shutdown"
CLIENT_PID=""

wait_for_pattern "$AXIOM_LOG" "removed (compositor window" "$TEARDOWN_TIMEOUT_SECS" "compositor-side X11 unmap/removal"
log "Axiom cleaned up the X11 compositor window"

log "Stopping Axiom"
kill -TERM "$AXIOM_PID"
wait_for_pattern "$AXIOM_LOG" "Axiom compositor shutdown complete" "$TEARDOWN_TIMEOUT_SECS" "compositor shutdown completion"
wait "$AXIOM_PID"
AXIOM_PID=""

log "X11-in-Axiom end-to-end smoke test passed"
