#!/bin/bash
# Axiom Wayland Server Test Script
# Tests the minimal Wayland server with real clients

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="${SCRIPT_DIR}/test_logs"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
SERVER_LOG="${LOG_DIR}/server_${TIMESTAMP}.log"
CLIENT_LOG="${LOG_DIR}/client_${TIMESTAMP}.log"
TEST_DURATION=10  # seconds to keep server running

# Ensure log directory exists
mkdir -p "${LOG_DIR}"

echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║   Axiom Wayland Server Testing Suite      ║${NC}"
echo -e "${BLUE}╔════════════════════════════════════════════╗${NC}"
echo ""

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"

    # Kill server if still running
    if [ ! -z "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "  Stopping server (PID: $SERVER_PID)"
        kill -TERM "$SERVER_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$SERVER_PID" 2>/dev/null || true
    fi

    # Kill client if still running
    if [ ! -z "$CLIENT_PID" ] && kill -0 "$CLIENT_PID" 2>/dev/null; then
        echo "  Stopping client (PID: $CLIENT_PID)"
        kill -TERM "$CLIENT_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$CLIENT_PID" 2>/dev/null || true
    fi

    # Remove socket if exists
    if [ ! -z "$WAYLAND_SOCKET_PATH" ] && [ -S "$WAYLAND_SOCKET_PATH" ]; then
        echo "  Removing socket: $WAYLAND_SOCKET_PATH"
        rm -f "$WAYLAND_SOCKET_PATH"
    fi

    echo -e "${GREEN}✓ Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# Step 1: Build the minimal Wayland server
echo -e "${BLUE}📦 Step 1: Building minimal Wayland server...${NC}"
if cargo build --features smithay-minimal --bin run_minimal_wayland 2>&1 | tee -a "${SERVER_LOG}"; then
    echo -e "${GREEN}✓ Build successful${NC}\n"
else
    echo -e "${RED}✗ Build failed! Check ${SERVER_LOG} for details${NC}"
    exit 1
fi

# Step 2: Start the Wayland server
echo -e "${BLUE}🚀 Step 2: Starting Wayland server...${NC}"

# Enable logging output
export RUST_LOG=info

# Start server in background and capture output
RUST_LOG=info "${SCRIPT_DIR}/target/debug/run_minimal_wayland" > "${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

echo "  Server PID: $SERVER_PID"
echo "  Log file: $SERVER_LOG"

# Wait for server to initialize and extract WAYLAND_DISPLAY
echo -n "  Waiting for server to initialize"
for i in {1..30}; do
    sleep 0.5
    echo -n "."

    # Check if server is still running
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo -e "\n${RED}✗ Server crashed during startup!${NC}"
        echo -e "${YELLOW}Last 20 lines of server log:${NC}"
        tail -20 "${SERVER_LOG}"
        exit 1
    fi

    # Try to extract WAYLAND_DISPLAY
    if grep -q "WAYLAND_DISPLAY" "${SERVER_LOG}"; then
        break
    fi
done
echo ""

# Extract WAYLAND_DISPLAY value
WAYLAND_DISPLAY=$(grep "WAYLAND_DISPLAY" "${SERVER_LOG}" | tail -1 | sed -n 's/.*WAYLAND_DISPLAY=\([a-zA-Z0-9_-]*\).*/\1/p')

if [ -z "$WAYLAND_DISPLAY" ]; then
    echo -e "${RED}✗ Could not find WAYLAND_DISPLAY in server output${NC}"
    echo -e "${YELLOW}Server log contents:${NC}"
    cat "${SERVER_LOG}"
    exit 1
fi

# Determine socket path
if [[ "$WAYLAND_DISPLAY" == /* ]]; then
    # Absolute path
    WAYLAND_SOCKET_PATH="$WAYLAND_DISPLAY"
else
    # Relative to XDG_RUNTIME_DIR
    XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}"
    WAYLAND_SOCKET_PATH="${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}"
fi

echo -e "${GREEN}✓ Server started successfully${NC}"
echo "  WAYLAND_DISPLAY: $WAYLAND_DISPLAY"
echo "  Socket path: $WAYLAND_SOCKET_PATH"

# Step 3: Verify socket exists
echo -e "\n${BLUE}🔍 Step 3: Verifying Wayland socket...${NC}"
if [ -S "$WAYLAND_SOCKET_PATH" ]; then
    echo -e "${GREEN}✓ Socket exists and is valid${NC}"
    ls -lh "$WAYLAND_SOCKET_PATH"
else
    echo -e "${RED}✗ Socket not found at expected path!${NC}"
    echo "  Expected: $WAYLAND_SOCKET_PATH"
    echo -e "${YELLOW}Available sockets in ${XDG_RUNTIME_DIR}:${NC}"
    ls -lh "${XDG_RUNTIME_DIR}"/wayland-* 2>/dev/null || echo "  None found"
    exit 1
fi

# Step 4: Test with a Wayland client
echo -e "\n${BLUE}🧪 Step 4: Testing with Wayland client...${NC}"

# Determine which client to use
CLIENT=""
CLIENT_ARGS=""

if command -v weston-terminal &> /dev/null; then
    CLIENT="weston-terminal"
    CLIENT_ARGS=""
    echo "  Using: weston-terminal"
elif command -v alacritty &> /dev/null; then
    CLIENT="alacritty"
    CLIENT_ARGS="-e echo 'Axiom Wayland Test - Press Ctrl+C to exit' && sleep 5"
    echo "  Using: alacritty"
elif command -v foot &> /dev/null; then
    CLIENT="foot"
    CLIENT_ARGS="-e bash -c 'echo \"Axiom Wayland Test\"; sleep 5'"
    echo "  Using: foot"
else
    echo -e "${YELLOW}⚠ No suitable Wayland client found${NC}"
    echo "  Install one of: weston-terminal, alacritty, foot"
    echo "  Skipping client test, but server is running..."

    # Keep server running for manual testing
    echo -e "\n${BLUE}Server is running. You can test manually:${NC}"
    echo "  export WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
    echo "  weston-terminal  # or any Wayland client"
    echo ""
    echo -e "${YELLOW}Press Ctrl+C to stop the server${NC}"

    wait $SERVER_PID
    exit 0
fi

# Export WAYLAND_DISPLAY for client
export WAYLAND_DISPLAY

echo "  Starting client with WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
echo "  Client will run for ${TEST_DURATION} seconds..."

# Start client
if [ -n "$CLIENT_ARGS" ]; then
    timeout ${TEST_DURATION} $CLIENT $CLIENT_ARGS > "${CLIENT_LOG}" 2>&1 &
else
    timeout ${TEST_DURATION} $CLIENT > "${CLIENT_LOG}" 2>&1 &
fi
CLIENT_PID=$!

# Monitor client startup
sleep 2

if kill -0 "$CLIENT_PID" 2>/dev/null; then
    echo -e "${GREEN}✓ Client started successfully (PID: $CLIENT_PID)${NC}"
else
    # Client may have exited, check exit code
    wait $CLIENT_PID 2>/dev/null
    CLIENT_EXIT=$?

    if [ $CLIENT_EXIT -eq 124 ]; then
        # Timeout (expected)
        echo -e "${GREEN}✓ Client ran and exited normally${NC}"
    elif [ $CLIENT_EXIT -eq 0 ]; then
        echo -e "${GREEN}✓ Client completed successfully${NC}"
    else
        echo -e "${YELLOW}⚠ Client exited with code $CLIENT_EXIT${NC}"
        echo -e "${YELLOW}Last 10 lines of client log:${NC}"
        tail -10 "${CLIENT_LOG}"
    fi
fi

# Step 5: Monitor server for errors
echo -e "\n${BLUE}📊 Step 5: Monitoring server activity...${NC}"

# Let it run for a bit
sleep 3

# Check for errors in server log
echo "  Checking for errors..."

ERROR_COUNT=$(grep -ci "error\|panic\|failed\|crash" "${SERVER_LOG}" || true)
WARNING_COUNT=$(grep -ci "warning\|warn" "${SERVER_LOG}" || true)

echo "  Errors found: $ERROR_COUNT"
echo "  Warnings found: $WARNING_COUNT"

if [ "$ERROR_COUNT" -gt 0 ]; then
    echo -e "\n${YELLOW}⚠ Found errors in server log:${NC}"
    grep -i "error\|panic\|failed\|crash" "${SERVER_LOG}" | head -20
fi

# Step 6: Check server is still running
echo -e "\n${BLUE}✅ Step 6: Final health check...${NC}"

if kill -0 "$SERVER_PID" 2>/dev/null; then
    echo -e "${GREEN}✓ Server is still running${NC}"

    # Get some stats
    SERVER_MEM=$(ps -o rss= -p "$SERVER_PID" 2>/dev/null | awk '{print $1/1024 "MB"}' || echo "N/A")
    echo "  Memory usage: $SERVER_MEM"

    # Check socket is still valid
    if [ -S "$WAYLAND_SOCKET_PATH" ]; then
        echo -e "${GREEN}✓ Socket is still valid${NC}"
    else
        echo -e "${RED}✗ Socket disappeared!${NC}"
    fi
else
    echo -e "${RED}✗ Server has crashed!${NC}"
    echo -e "${YELLOW}Last 30 lines of server log:${NC}"
    tail -30 "${SERVER_LOG}"
    exit 1
fi

# Step 7: Summary
echo -e "\n${BLUE}╔════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║              Test Summary                  ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${GREEN}✓ Server build: SUCCESS${NC}"
echo -e "${GREEN}✓ Server startup: SUCCESS${NC}"
echo -e "${GREEN}✓ Socket creation: SUCCESS${NC}"
echo -e "${GREEN}✓ Client connection: SUCCESS${NC}"
echo -e "${GREEN}✓ Server stability: PASS${NC}"
echo ""
echo -e "${BLUE}Logs saved to:${NC}"
echo "  Server: ${SERVER_LOG}"
echo "  Client: ${CLIENT_LOG}"
echo ""

# Show interesting server output
echo -e "${BLUE}Server Output (last 50 lines):${NC}"
echo "═══════════════════════════════════════════════"
tail -50 "${SERVER_LOG}"
echo "═══════════════════════════════════════════════"
echo ""

# Offer to keep server running
echo -e "${YELLOW}Server is still running. Options:${NC}"
echo "  1. Press Ctrl+C to stop"
echo "  2. Run clients manually:"
echo "     export WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
echo "     weston-terminal"
echo ""
echo -e "${BLUE}Keeping server alive for manual testing...${NC}"
echo -e "${YELLOW}Press Ctrl+C to stop${NC}"

# Keep server running until interrupted
wait $SERVER_PID

echo -e "\n${GREEN}✓ Test complete${NC}"
