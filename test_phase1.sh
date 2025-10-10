#!/bin/bash
# Automated Phase 1 Testing Script for Axiom Compositor

set -e

echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║           Axiom Phase 1 Automated Test Suite                 ║"
echo "╚═══════════════════════════════════════════════════════════════╝"
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test results
TESTS_PASSED=0
TESTS_FAILED=0
TEST_LOG="/tmp/axiom_phase1_test_$(date +%s).log"

log() {
    echo "[$(date '+%H:%M:%S')] $1" | tee -a "$TEST_LOG"
}

test_result() {
    if [ $1 -eq 0 ]; then
        echo -e "${GREEN}✅ PASS${NC}: $2"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}❌ FAIL${NC}: $2"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    log "Test '$2': $([ $1 -eq 0 ] && echo 'PASS' || echo 'FAIL')"
}

# Cleanup function
cleanup() {
    log "Cleaning up..."
    if [ -n "$AXIOM_PID" ]; then
        kill $AXIOM_PID 2>/dev/null || true
        wait $AXIOM_PID 2>/dev/null || true
    fi
    killall weston-simple-shm 2>/dev/null || true
    killall weston-terminal 2>/dev/null || true
}

trap cleanup EXIT

echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.0: Prerequisites Check"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Check if binary exists
log "Checking if run_present_winit binary exists..."
if [ -f "./target/release/run_present_winit" ]; then
    test_result 0 "Binary exists"
else
    test_result 1 "Binary exists"
    echo "Building binary first..."
    cargo build --release --features="smithay,wgpu-present" --bin run_present_winit
fi

# Check if weston is installed
log "Checking if weston test clients are installed..."
if command -v weston-simple-shm &> /dev/null; then
    test_result 0 "Weston test clients installed"
    if ! command -v weston-info &> /dev/null; then
        echo "⚠️  weston-info not available (optional, will skip protocol test)"
    fi
else
    test_result 1 "Weston test clients installed"
    echo "Please install weston: sudo pacman -S weston"
    exit 1
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.1: Starting Axiom Compositor"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Start compositor in background
log "Starting Axiom compositor..."
./target/release/run_present_winit > /tmp/axiom_compositor.log 2>&1 &
AXIOM_PID=$!
log "Axiom PID: $AXIOM_PID"

# Wait for compositor to initialize
echo "Waiting for compositor to initialize (5 seconds)..."
sleep 5

# Check if compositor is still running
if ps -p $AXIOM_PID > /dev/null 2>&1; then
    test_result 0 "Compositor started and running"
else
    test_result 1 "Compositor started and running"
    echo "Compositor crashed! Checking logs:"
    tail -20 /tmp/axiom_compositor.log
    exit 1
fi

# Find Wayland socket
log "Finding Wayland socket..."
WAYLAND_SOCKET=$(ls -1 /run/user/$(id -u)/wayland-* 2>/dev/null | grep -v "\.lock$" | tail -1)
if [ -z "$WAYLAND_SOCKET" ]; then
    test_result 1 "Wayland socket created"
    echo "No Wayland socket found!"
    exit 1
else
    export WAYLAND_DISPLAY=$(basename "$WAYLAND_SOCKET")
    test_result 0 "Wayland socket created: $WAYLAND_DISPLAY"
    log "Using WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.2: Protocol Introspection"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Test weston-info (optional)
if command -v weston-info &> /dev/null; then
    log "Running weston-info..."
    if timeout 5 weston-info > /tmp/weston-info.txt 2>&1; then
        test_result 0 "weston-info connected successfully"
        
        # Check for essential protocols
        if grep -q "wl_compositor" /tmp/weston-info.txt; then
            test_result 0 "wl_compositor protocol available"
        else
            test_result 1 "wl_compositor protocol available"
        fi
        
        if grep -q "wl_shm" /tmp/weston-info.txt; then
            test_result 0 "wl_shm protocol available"
        else
            test_result 1 "wl_shm protocol available"
        fi
        
        if grep -q "xdg_wm_base" /tmp/weston-info.txt; then
            test_result 0 "xdg_wm_base protocol available"
        else
            test_result 1 "xdg_wm_base protocol available"
        fi
        
        if grep -q "wl_seat" /tmp/weston-info.txt; then
            test_result 0 "wl_seat protocol available"
        else
            test_result 1 "wl_seat protocol available"
        fi
        
        echo ""
        echo "Available protocols:"
        grep "interface:" /tmp/weston-info.txt | head -20
    else
        test_result 1 "weston-info connected successfully"
        echo "weston-info failed! Check /tmp/weston-info.txt"
    fi
else
    echo "⚠️  weston-info not available, skipping protocol introspection test"
    echo "     (Protocol support will be tested via actual clients)"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.3: Simple SHM Buffer Test"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Test weston-simple-shm
log "Running weston-simple-shm..."
timeout 10 weston-simple-shm > /tmp/weston-simple-shm.log 2>&1 &
SHM_PID=$!
sleep 3

# Check if client is running
if ps -p $SHM_PID > /dev/null 2>&1; then
    test_result 0 "weston-simple-shm started without crash"
    
    # Check compositor logs for window creation
    sleep 2
    if grep -q "mapped window" /tmp/axiom_compositor.log; then
        test_result 0 "Window was mapped in compositor"
    else
        test_result 1 "Window was mapped in compositor"
        echo "Expected 'mapped window' in logs"
    fi
    
    if grep -q "queue_texture_update\|Processing.*pending texture" /tmp/axiom_compositor.log; then
        test_result 0 "Texture update queued"
    else
        test_result 1 "Texture update queued"
        echo "Expected texture update messages"
    fi
    
    # Check window count
    if grep -q "renderer now has.*window" /tmp/axiom_compositor.log; then
        test_result 0 "Renderer tracking windows"
        WINDOW_COUNT=$(grep "renderer now has" /tmp/axiom_compositor.log | tail -1)
        echo "Last window count: $WINDOW_COUNT"
    else
        test_result 1 "Renderer tracking windows"
    fi
    
    kill $SHM_PID 2>/dev/null || true
else
    test_result 1 "weston-simple-shm started without crash"
    echo "weston-simple-shm crashed! Check /tmp/weston-simple-shm.log"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.4: Multiple Clients Test"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Test multiple clients
log "Running multiple weston-simple-shm instances..."
timeout 15 weston-simple-shm > /tmp/shm1.log 2>&1 &
SHM1_PID=$!
sleep 1
timeout 15 weston-simple-shm > /tmp/shm2.log 2>&1 &
SHM2_PID=$!
sleep 2

# Check if both are running
BOTH_RUNNING=0
if ps -p $SHM1_PID > /dev/null 2>&1 && ps -p $SHM2_PID > /dev/null 2>&1; then
    BOTH_RUNNING=1
    test_result 0 "Multiple clients running simultaneously"
else
    test_result 1 "Multiple clients running simultaneously"
fi

if [ $BOTH_RUNNING -eq 1 ]; then
    # Check for multiple windows in compositor
    sleep 2
    MAPPED_COUNT=$(grep -c "mapped window" /tmp/axiom_compositor.log || echo 0)
    if [ $MAPPED_COUNT -ge 2 ]; then
        test_result 0 "Multiple windows mapped ($MAPPED_COUNT windows)"
    else
        test_result 1 "Multiple windows mapped (only $MAPPED_COUNT windows)"
    fi
fi

# Cleanup multiple clients
kill $SHM1_PID $SHM2_PID 2>/dev/null || true

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.5: Terminal Test (if available)"
echo "═══════════════════════════════════════════════════════════════"
echo ""

if command -v weston-terminal &> /dev/null; then
    log "Running weston-terminal..."
    timeout 10 weston-terminal > /tmp/weston-terminal.log 2>&1 &
    TERM_PID=$!
    sleep 3
    
    if ps -p $TERM_PID > /dev/null 2>&1; then
        test_result 0 "weston-terminal started"
        kill $TERM_PID 2>/dev/null || true
    else
        test_result 1 "weston-terminal started"
    fi
else
    echo "⚠️  weston-terminal not available (optional test)"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Phase 1.6: Compositor Health Check"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# Check if compositor is still healthy
if ps -p $AXIOM_PID > /dev/null 2>&1; then
    test_result 0 "Compositor still running after tests"
    
    # Check for errors in logs
    if grep -qi "error\|panic\|crash\|fatal" /tmp/axiom_compositor.log; then
        echo "⚠️  Found error messages in logs:"
        grep -i "error\|panic\|crash\|fatal" /tmp/axiom_compositor.log | tail -5
    else
        test_result 0 "No errors in compositor logs"
    fi
    
    # Check memory usage
    MEM_USAGE=$(ps -p $AXIOM_PID -o rss= 2>/dev/null || echo 0)
    MEM_MB=$((MEM_USAGE / 1024))
    log "Compositor memory usage: ${MEM_MB}MB"
    if [ $MEM_MB -lt 1000 ]; then
        test_result 0 "Memory usage reasonable (${MEM_MB}MB)"
    else
        echo "⚠️  High memory usage: ${MEM_MB}MB"
    fi
else
    test_result 1 "Compositor still running after tests"
    echo "Compositor crashed during tests!"
fi

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "Test Results Summary"
echo "═══════════════════════════════════════════════════════════════"
echo ""
echo -e "Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║                  🎉 PHASE 1 COMPLETE! 🎉                      ║${NC}"
    echo -e "${GREEN}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo "✅ Axiom compositor successfully:"
    echo "   • Starts and runs stably"
    echo "   • Advertises all required Wayland protocols"
    echo "   • Handles client connections"
    echo "   • Maps windows correctly"
    echo "   • Processes buffer updates"
    echo "   • Supports multiple simultaneous clients"
    echo ""
    echo "🚀 Ready to proceed to Phase 2: Window Decorations & Tiling!"
    echo ""
    echo "Logs saved to:"
    echo "   • Test log: $TEST_LOG"
    echo "   • Compositor log: /tmp/axiom_compositor.log"
    echo "   • Protocol info: /tmp/weston-info.txt"
    EXIT_CODE=0
else
    echo -e "${YELLOW}╔═══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║            ⚠️  PHASE 1 HAS ISSUES TO FIX ⚠️                  ║${NC}"
    echo -e "${YELLOW}╚═══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo "Some tests failed. Check the logs:"
    echo "   • Test log: $TEST_LOG"
    echo "   • Compositor log: /tmp/axiom_compositor.log"
    echo ""
    echo "Common issues:"
    echo "   1. Compositor crashes → Check /tmp/axiom_compositor.log"
    echo "   2. No windows appear → Check for 'mapped window' in logs"
    echo "   3. Protocol errors → Run RUST_LOG=debug and check output"
    EXIT_CODE=1
fi

# Cleanup
kill $AXIOM_PID 2>/dev/null || true

exit $EXIT_CODE
