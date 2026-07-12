#!/usr/bin/env bash
# Real nested compositor smoke test using an actual Wayland client.
#
# What this verifies:
# 1. Axiom starts in nested/windowed mode.
# 2. It announces a Wayland socket.
# 3. A real client can connect to that socket.
# 4. A client surface maps (`New XDG toplevel` appears in logs).
# 5. Client teardown removes the window cleanly.
# 6. The compositor handles SIGTERM and shuts down cleanly.

set -euo pipefail

BINARY_PATH="${1:-${AXIOM_BIN:-./target/debug/axiom}}"
STARTUP_TIMEOUT_SECS="${AXIOM_SMOKE_STARTUP_TIMEOUT_SECS:-30}"
MAP_TIMEOUT_SECS="${AXIOM_SMOKE_MAP_TIMEOUT_SECS:-20}"
TEARDOWN_TIMEOUT_SECS="${AXIOM_SMOKE_TEARDOWN_TIMEOUT_SECS:-15}"
INFO_TIMEOUT_SECS="${AXIOM_SMOKE_INFO_TIMEOUT_SECS:-10}"

# Prefer a cheap registry/info probe when available, but the actual pass/fail
# signal is the toplevel client mapping inside Axiom.
DEFAULT_INFO_CLIENT=""
if command -v weston-info >/dev/null 2>&1; then
    DEFAULT_INFO_CLIENT="weston-info"
elif command -v wayland-info >/dev/null 2>&1; then
    DEFAULT_INFO_CLIENT="wayland-info"
fi

INFO_CLIENT_RAW="${AXIOM_SMOKE_INFO_CLIENT:-$DEFAULT_INFO_CLIENT}"
TOPLEVEL_CLIENT_RAW="${AXIOM_SMOKE_TOPLEVEL_CLIENT:-weston-terminal}"
EXTRA_ARGS_RAW="${AXIOM_SMOKE_EXTRA_ARGS:-}"
KEEP_LOGS="${AXIOM_SMOKE_KEEP_LOGS:-false}"

TMP_ROOT="$(mktemp -d)"
AXIOM_LOG="$TMP_ROOT/axiom.log"
INFO_LOG="$TMP_ROOT/info-client.log"
CLIENT_LOG="$TMP_ROOT/toplevel-client.log"
CONFIG_PATH="$TMP_ROOT/axiom-smoke.toml"

AXIOM_PID=""
CLIENT_PID=""
SOCKET_NAME=""

log() {
    echo "[axiom-smoke] $*"
}

print_log_tail() {
    local label="$1"
    local path="$2"
    if [[ -f "$path" ]]; then
        echo "----- $label (tail) -----" >&2
        tail -n 80 "$path" >&2 || true
    fi
}

fail() {
    local message="$1"
    echo "[axiom-smoke] ERROR: $message" >&2
    print_log_tail "Axiom log" "$AXIOM_LOG"
    print_log_tail "Info client log" "$INFO_LOG"
    print_log_tail "Toplevel client log" "$CLIENT_LOG"
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

wait_for_socket_file() {
    local socket_path="$1"
    local timeout_secs="$2"
    local deadline=$((SECONDS + timeout_secs))

    while (( SECONDS < deadline )); do
        if [[ -S "$socket_path" ]]; then
            return 0
        fi

        if [[ -n "$AXIOM_PID" ]] && ! kill -0 "$AXIOM_PID" 2>/dev/null; then
            wait "$AXIOM_PID" || true
            fail "Axiom exited before Wayland socket file appeared: $socket_path"
        fi

        sleep 0.2
    done

    fail "Timed out waiting for Wayland socket file: $socket_path"
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

read -r -a TOPLEVEL_CLIENT <<< "$TOPLEVEL_CLIENT_RAW"
if [[ ${#TOPLEVEL_CLIENT[@]} -eq 0 ]]; then
    fail "No toplevel smoke client configured"
fi
if ! command -v "${TOPLEVEL_CLIENT[0]}" >/dev/null 2>&1; then
    fail "Required toplevel client is missing: ${TOPLEVEL_CLIENT[0]}"
fi

INFO_CLIENT=()
if [[ -n "$INFO_CLIENT_RAW" ]]; then
    read -r -a INFO_CLIENT <<< "$INFO_CLIENT_RAW"
    if ! command -v "${INFO_CLIENT[0]}" >/dev/null 2>&1; then
        fail "Configured info client is missing: ${INFO_CLIENT[0]}"
    fi
fi

EXTRA_ARGS=()
if [[ -n "$EXTRA_ARGS_RAW" ]]; then
    read -r -a EXTRA_ARGS <<< "$EXTRA_ARGS_RAW"
fi

ORIGINAL_XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-}"

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
enabled = false
EOF

LAUNCH_CMD=("$BINARY_PATH" "--config" "$CONFIG_PATH" "--windowed" "--debug")
if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
    LAUNCH_CMD+=("${EXTRA_ARGS[@]}")
fi
if command -v stdbuf >/dev/null 2>&1; then
    LAUNCH_CMD=(stdbuf -oL -eL "${LAUNCH_CMD[@]}")
fi

log "Launching Axiom: ${LAUNCH_CMD[*]}"
"${LAUNCH_CMD[@]}" >"$AXIOM_LOG" 2>&1 &
AXIOM_PID=$!

wait_for_pattern "$AXIOM_LOG" "Wayland socket:" "$STARTUP_TIMEOUT_SECS" "Wayland socket announcement"
SOCKET_NAME="$(sed -nE 's/.*Wayland socket: ([^[:space:]]+).*/\1/p' "$AXIOM_LOG" | tail -n 1)"
if [[ -z "$SOCKET_NAME" ]]; then
    fail "Could not extract Wayland socket name from Axiom logs"
fi
log "Axiom announced nested Wayland socket: $SOCKET_NAME"

wait_for_socket_file "$XDG_RUNTIME_DIR/$SOCKET_NAME" "$STARTUP_TIMEOUT_SECS"

if [[ ${#INFO_CLIENT[@]} -gt 0 ]]; then
    log "Running registry/info probe: ${INFO_CLIENT[*]}"
    if ! timeout "$INFO_TIMEOUT_SECS" env \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
        WAYLAND_DISPLAY="$SOCKET_NAME" \
        "${INFO_CLIENT[@]}" >"$INFO_LOG" 2>&1; then
        fail "Wayland registry/info probe failed"
    fi
fi

log "Launching real toplevel client: ${TOPLEVEL_CLIENT[*]}"
env \
    XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
    WAYLAND_DISPLAY="$SOCKET_NAME" \
    "${TOPLEVEL_CLIENT[@]}" >"$CLIENT_LOG" 2>&1 &
CLIENT_PID=$!

wait_for_pattern "$AXIOM_LOG" "New XDG toplevel:" "$MAP_TIMEOUT_SECS" "real client surface mapping"
log "Axiom observed a real mapped client surface"

if kill -0 "$CLIENT_PID" 2>/dev/null; then
    log "Stopping toplevel client"
    kill -TERM "$CLIENT_PID" 2>/dev/null || true
fi
wait_for_process_exit "$CLIENT_PID" "$TEARDOWN_TIMEOUT_SECS" "toplevel client shutdown"
CLIENT_PID=""

wait_for_pattern "$AXIOM_LOG" "Destroying window" "$TEARDOWN_TIMEOUT_SECS" "window teardown after client exit"
log "Axiom cleaned up the client window"

log "Stopping Axiom"
kill -TERM "$AXIOM_PID"
wait_for_pattern "$AXIOM_LOG" "Axiom compositor shutdown complete" "$TEARDOWN_TIMEOUT_SECS" "compositor shutdown completion"
wait "$AXIOM_PID"
AXIOM_PID=""

log "Nested smoke test passed"
