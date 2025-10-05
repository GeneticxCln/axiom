#!/bin/bash
# Test script for Axiom SHM rendering validation
# This validates the complete rendering pipeline with a shared memory client

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR="$PROJECT_ROOT/tests"
LOG_DIR="$PROJECT_ROOT/test_logs_shm"
COMPOSITOR_LOG="$LOG_DIR/compositor.log"
CLIENT_LOG="$LOG_DIR/client.log"
BUILD_LOG="$LOG_DIR/build.log"
TEST_TIMEOUT=30

echo -e "${CYAN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║  Axiom SHM Rendering Test - Phase 6.3 Validation       ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════════════════╝${NC}"
echo ""

# Create log directory
mkdir -p "$LOG_DIR"

# Function to cleanup on exit
cleanup() {
    echo ""
    echo -e "${YELLOW}🧹 Cleaning up...${NC}"

    # Kill compositor if running
    if [ ! -z "$COMPOSITOR_PID" ]; then
        echo "   Stopping compositor (PID: $COMPOSITOR_PID)"
        kill $COMPOSITOR_PID 2>/dev/null || true
        wait $COMPOSITOR_PID 2>/dev/null || true
    fi

    # Kill client if running
    if [ ! -z "$CLIENT_PID" ]; then
        echo "   Stopping client (PID: $CLIENT_PID)"
        kill $CLIENT_PID 2>/dev/null || true
        wait $CLIENT_PID 2>/dev/null || true
    fi

    echo -e "${GREEN}✅ Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# Step 1: Build the SHM test client
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}Step 1: Building SHM Test Client${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

cd "$TEST_DIR"

echo "📦 Checking dependencies..."
if ! command -v wayland-scanner &> /dev/null; then
    echo -e "${RED}❌ wayland-scanner not found${NC}"
    echo "   Install with: sudo apt-get install wayland-protocols libwayland-dev"
    exit 1
fi
echo "   ✅ wayland-scanner found"

if ! pkg-config --exists wayland-client; then
    echo -e "${RED}❌ wayland-client not found${NC}"
    echo "   Install with: sudo apt-get install libwayland-dev"
    exit 1
fi
echo "   ✅ wayland-client found"

echo ""
echo "🔨 Building SHM test client..."
if make clean > "$BUILD_LOG" 2>&1 && make >> "$BUILD_LOG" 2>&1; then
    echo -e "   ${GREEN}✅ Build successful${NC}"
    if [ -f "shm_test_client" ]; then
        echo "   📍 Binary: $TEST_DIR/shm_test_client"
    else
        echo -e "   ${RED}❌ Binary not found after build${NC}"
        exit 1
    fi
else
    echo -e "   ${RED}❌ Build failed${NC}"
    echo "   📋 Build log: $BUILD_LOG"
    cat "$BUILD_LOG"
    exit 1
fi

# Step 2: Build the compositor
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}Step 2: Building Axiom Compositor${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

cd "$PROJECT_ROOT"

echo "🔨 Building compositor with wgpu-present feature..."
if cargo build --features wgpu-present --bin run_present_winit >> "$BUILD_LOG" 2>&1; then
    echo -e "   ${GREEN}✅ Compositor build successful${NC}"
else
    echo -e "   ${RED}❌ Compositor build failed${NC}"
    echo "   📋 Build log: $BUILD_LOG"
    tail -n 50 "$BUILD_LOG"
    exit 1
fi

# Step 3: Start the compositor
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}Step 3: Starting Axiom Compositor${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo "🚀 Starting compositor..."
RUST_LOG=info,axiom=debug \
    WAYLAND_DISPLAY=wayland-axiom-test \
    cargo run --features wgpu-present --bin run_present_winit \
    > "$COMPOSITOR_LOG" 2>&1 &

COMPOSITOR_PID=$!
echo "   ⚙️  Compositor PID: $COMPOSITOR_PID"
echo "   📋 Compositor log: $COMPOSITOR_LOG"

# Wait for compositor to initialize
echo "   ⏳ Waiting for compositor initialization..."
WAIT_COUNT=0
while [ $WAIT_COUNT -lt 10 ]; do
    if grep -q "Wayland server started" "$COMPOSITOR_LOG" 2>/dev/null || \
       grep -q "run_present_winit" "$COMPOSITOR_LOG" 2>/dev/null || \
       [ -S "/tmp/wayland-axiom-test" ]; then
        echo -e "   ${GREEN}✅ Compositor initialized${NC}"
        break
    fi

    # Check if compositor crashed
    if ! kill -0 $COMPOSITOR_PID 2>/dev/null; then
        echo -e "   ${RED}❌ Compositor crashed during startup${NC}"
        echo "   Last 20 lines of log:"
        tail -n 20 "$COMPOSITOR_LOG"
        exit 1
    fi

    sleep 1
    WAIT_COUNT=$((WAIT_COUNT + 1))
done

if [ $WAIT_COUNT -ge 10 ]; then
    echo -e "   ${YELLOW}⚠️  Compositor may not be fully ready, but continuing...${NC}"
fi

# Give compositor a bit more time to stabilize
sleep 2

# Step 4: Run the SHM test client
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}Step 4: Running SHM Test Client${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo "🎨 Starting test client..."
cd "$TEST_DIR"

WAYLAND_DISPLAY=wayland-axiom-test \
    timeout $TEST_TIMEOUT ./shm_test_client > "$CLIENT_LOG" 2>&1 &

CLIENT_PID=$!
echo "   ⚙️  Client PID: $CLIENT_PID"
echo "   📋 Client log: $CLIENT_LOG"

# Monitor client for success
echo "   ⏳ Monitoring client progress..."
WAIT_COUNT=0
SUCCESS=0

while [ $WAIT_COUNT -lt $TEST_TIMEOUT ]; do
    # Check if client completed successfully
    if grep -q "Window is now visible" "$CLIENT_LOG" 2>/dev/null; then
        echo -e "   ${GREEN}✅ Client successfully created window!${NC}"
        SUCCESS=1
        break
    fi

    # Check if client crashed
    if ! kill -0 $CLIENT_PID 2>/dev/null; then
        wait $CLIENT_PID
        EXIT_CODE=$?
        if [ $EXIT_CODE -eq 0 ]; then
            echo -e "   ${GREEN}✅ Client completed successfully${NC}"
            SUCCESS=1
        else
            echo -e "   ${RED}❌ Client exited with code $EXIT_CODE${NC}"
        fi
        break
    fi

    # Check for errors in client log
    if grep -q "Failed to" "$CLIENT_LOG" 2>/dev/null; then
        echo -e "   ${RED}❌ Client encountered errors${NC}"
        break
    fi

    sleep 1
    WAIT_COUNT=$((WAIT_COUNT + 1))
done

# Let it run for a few seconds to allow rendering
if [ $SUCCESS -eq 1 ]; then
    echo ""
    echo -e "${GREEN}✨ Test window should be visible on screen!${NC}"
    echo -e "${CYAN}   The window displays a red/blue checkerboard with gradients${NC}"
    echo -e "${CYAN}   Press Ctrl+C to exit or wait 10 seconds...${NC}"
    sleep 10
fi

# Step 5: Analyze results
echo ""
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
echo -e "${BLUE}Step 5: Results Analysis${NC}"
echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

echo ""
echo "📊 Client Output:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
cat "$CLIENT_LOG"
echo ""

echo "📊 Compositor Output (last 50 lines):"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
tail -n 50 "$COMPOSITOR_LOG"
echo ""

# Check for success indicators
echo "🔍 Success Indicators:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

CHECKS_PASSED=0
CHECKS_TOTAL=8

# Client checks
if grep -q "Connected to Wayland display" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ Client connected to Wayland${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ Client failed to connect${NC}"
fi

if grep -q "Bound wl_compositor" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ wl_compositor bound${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ wl_compositor not bound${NC}"
fi

if grep -q "Bound wl_shm" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ wl_shm bound${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ wl_shm not bound${NC}"
fi

if grep -q "Bound xdg_wm_base" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ xdg_wm_base bound${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ xdg_wm_base not bound${NC}"
fi

if grep -q "Created SHM buffer" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ SHM buffer created${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ SHM buffer creation failed${NC}"
fi

if grep -q "Drew test pattern" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ Test pattern drawn${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ Test pattern not drawn${NC}"
fi

if grep -q "XDG surface configured" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ XDG surface configured${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ XDG surface not configured${NC}"
fi

if grep -q "Attached buffer and committed" "$CLIENT_LOG"; then
    echo -e "   ${GREEN}✅ Buffer attached and committed${NC}"
    CHECKS_PASSED=$((CHECKS_PASSED + 1))
else
    echo -e "   ${RED}❌ Buffer not attached${NC}"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "   ${CYAN}Checks Passed: $CHECKS_PASSED / $CHECKS_TOTAL${NC}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Final result
echo ""
if [ $CHECKS_PASSED -eq $CHECKS_TOTAL ] && [ $SUCCESS -eq 1 ]; then
    echo -e "${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║                  ✅ TEST PASSED! ✅                      ║${NC}"
    echo -e "${GREEN}║                                                          ║${NC}"
    echo -e "${GREEN}║  SHM rendering pipeline validated successfully!          ║${NC}"
    echo -e "${GREEN}║  Phase 6.3 end-to-end rendering confirmed!               ║${NC}"
    echo -e "${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"
    EXIT_CODE=0
elif [ $CHECKS_PASSED -ge 6 ]; then
    echo -e "${YELLOW}╔══════════════════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║                 ⚠️  PARTIAL SUCCESS ⚠️                  ║${NC}"
    echo -e "${YELLOW}║                                                          ║${NC}"
    echo -e "${YELLOW}║  Most checks passed but some issues remain               ║${NC}"
    echo -e "${YELLOW}║  Review logs for details                                 ║${NC}"
    echo -e "${YELLOW}╚══════════════════════════════════════════════════════════╝${NC}"
    EXIT_CODE=1
else
    echo -e "${RED}╔══════════════════════════════════════════════════════════╗${NC}"
    echo -e "${RED}║                   ❌ TEST FAILED ❌                      ║${NC}"
    echo -e "${RED}║                                                          ║${NC}"
    echo -e "${RED}║  Review logs above for error details                     ║${NC}"
    echo -e "${RED}╚══════════════════════════════════════════════════════════╝${NC}"
    EXIT_CODE=1
fi

echo ""
echo -e "${CYAN}📁 Test artifacts saved to: $LOG_DIR${NC}"
echo ""

exit $EXIT_CODE
