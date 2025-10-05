#!/bin/bash
# Detailed debugging test for Axiom rendering pipeline
# Traces data flow from client to GPU

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

LOG_DIR="test_logs_debug"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
SERVER_LOG="${LOG_DIR}/server_${TIMESTAMP}.log"
CLIENT_LOG="${LOG_DIR}/client_${TIMESTAMP}.log"

mkdir -p "${LOG_DIR}"

echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}   🔬 Axiom Rendering Pipeline - Debug Trace${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo ""

cleanup() {
    echo ""
    echo -e "${YELLOW}🧹 Cleaning up...${NC}"

    if [ ! -z "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill -TERM "$SERVER_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$SERVER_PID" 2>/dev/null || true
    fi

    if [ ! -z "$CLIENT_PID" ] && kill -0 "$CLIENT_PID" 2>/dev/null; then
        kill -TERM "$CLIENT_PID" 2>/dev/null || true
    fi

    echo -e "${GREEN}✓ Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# Step 1: Build
echo -e "${CYAN}📦 Step 1: Building with debug logging...${NC}"
cargo build --features wgpu-present --bin run_present_winit --quiet
echo -e "${GREEN}✓ Build complete${NC}"
echo ""

# Step 2: Start server with verbose logging
echo -e "${CYAN}🚀 Step 2: Starting compositor with RUST_LOG=debug...${NC}"
RUST_LOG=debug,wgpu_core=warn,wgpu_hal=warn ./target/debug/run_present_winit --backend auto > "${SERVER_LOG}" 2>&1 &
SERVER_PID=$!

echo "  Server PID: $SERVER_PID"
echo "  Log: $SERVER_LOG"
echo "  Waiting for initialization (10 seconds)..."
sleep 10

# Step 3: Check server health
echo ""
echo -e "${CYAN}🔍 Step 3: Checking server health...${NC}"

if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo -e "${RED}✗ Server crashed during startup!${NC}"
    echo ""
    echo -e "${YELLOW}Last 50 lines of log:${NC}"
    tail -50 "${SERVER_LOG}"
    exit 1
fi

echo -e "${GREEN}✓ Server is running${NC}"

# Extract WAYLAND_DISPLAY
if grep -q "WAYLAND_DISPLAY" "${SERVER_LOG}"; then
    WAYLAND_DISPLAY=$(grep "WAYLAND_DISPLAY=" "${SERVER_LOG}" | tail -1 | sed -n 's/.*WAYLAND_DISPLAY=\([a-zA-Z0-9_-]*\).*/\1/p')
    echo "  WAYLAND_DISPLAY: $WAYLAND_DISPLAY"
    export WAYLAND_DISPLAY
else
    echo -e "${RED}✗ Could not find WAYLAND_DISPLAY${NC}"
    exit 1
fi

# Step 4: Check initial renderer state
echo ""
echo -e "${CYAN}📊 Step 4: Initial renderer state...${NC}"

echo "  Checking for renderer initialization..."
if grep -q "Creating real GPU renderer" "${SERVER_LOG}"; then
    echo -e "${GREEN}  ✓ GPU renderer initialized${NC}"
else
    echo -e "${YELLOW}  ⚠ GPU renderer not found${NC}"
fi

echo "  Checking for placeholder state..."
PLACEHOLDER_COUNT=$(grep -c "push_placeholder_quad" "${SERVER_LOG}" || echo "0")
echo "  Placeholder calls so far: $PLACEHOLDER_COUNT"

# Step 5: Launch test client
echo ""
echo -e "${CYAN}🧪 Step 5: Launching test client...${NC}"
echo "  Using minimal Wayland server for testing"

# Use weston-terminal for a simple test
timeout 8 weston-terminal > "${CLIENT_LOG}" 2>&1 &
CLIENT_PID=$!

echo "  Client PID: $CLIENT_PID"
echo "  Waiting 8 seconds for client activity..."
sleep 8

# Step 6: Analyze data flow
echo ""
echo -e "${CYAN}🔬 Step 6: Analyzing data flow...${NC}"
echo ""

# Check 1: Window creation
echo -e "${BLUE}Check 1: Window Creation${NC}"
WINDOW_COUNT=$(grep -c "Adding window\|new_toplevel\|mapped window" "${SERVER_LOG}" || echo "0")
echo "  Windows created: $WINDOW_COUNT"
if [ "$WINDOW_COUNT" -gt 0 ]; then
    echo -e "${GREEN}  ✓ Windows are being created${NC}"
    grep "mapped window\|Adding window" "${SERVER_LOG}" | tail -3
else
    echo -e "${RED}  ✗ No windows created${NC}"
fi
echo ""

# Check 2: Placeholder quads
echo -e "${BLUE}Check 2: Placeholder Quads${NC}"
PLACEHOLDER_TOTAL=$(grep -c "push_placeholder_quad" "${SERVER_LOG}" || echo "0")
echo "  Total placeholder_quad calls: $PLACEHOLDER_TOTAL"
if [ "$PLACEHOLDER_TOTAL" -gt 0 ]; then
    echo -e "${GREEN}  ✓ Placeholders are being pushed${NC}"
    echo "  Last 3 placeholder calls:"
    grep "push_placeholder_quad" "${SERVER_LOG}" | tail -3 | sed 's/^/    /'
else
    echo -e "${RED}  ✗ No placeholders pushed${NC}"
fi
echo ""

# Check 3: Texture updates queued
echo -e "${BLUE}Check 3: Texture Update Queue${NC}"
TEXTURE_QUEUE=$(grep -c "queue_texture_update\|pending textures" "${SERVER_LOG}" || echo "0")
echo "  Texture queue operations: $TEXTURE_QUEUE"
if [ "$TEXTURE_QUEUE" -gt 0 ]; then
    echo -e "${GREEN}  ✓ Textures are being queued${NC}"
    grep "queue_texture_update\|pending textures" "${SERVER_LOG}" | tail -3 | sed 's/^/    /'
else
    echo -e "${YELLOW}  ⚠ No texture queue operations found${NC}"
fi
echo ""

# Check 4: Texture processing
echo -e "${BLUE}Check 4: Texture Processing${NC}"
TEXTURE_PROCESS=$(grep -c "Processing.*pending texture\|update_window_texture\|Updated texture" "${SERVER_LOG}" || echo "0")
echo "  Texture processing operations: $TEXTURE_PROCESS"
if [ "$TEXTURE_PROCESS" -gt 0 ]; then
    echo -e "${GREEN}  ✓ Textures are being processed${NC}"
    grep "Processing.*pending texture\|Updated texture" "${SERVER_LOG}" | tail -3 | sed 's/^/    /'
else
    echo -e "${YELLOW}  ⚠ No texture processing found${NC}"
fi
echo ""

# Check 5: sync_from_shared activity
echo -e "${BLUE}Check 5: Renderer Sync Activity${NC}"
SYNC_COUNT=$(grep -c "sync_from_shared" "${SERVER_LOG}" || echo "0")
echo "  sync_from_shared calls: $SYNC_COUNT"
if [ "$SYNC_COUNT" -gt 0 ]; then
    echo "  Last sync operation:"
    grep "sync_from_shared:" "${SERVER_LOG}" | tail -1 | sed 's/^/    /'

    # Check what it found
    LAST_SYNC=$(grep "sync_from_shared: found" "${SERVER_LOG}" | tail -1)
    echo "  $LAST_SYNC" | sed 's/^/    /'
else
    echo -e "${YELLOW}  ⚠ No sync operations${NC}"
fi
echo ""

# Check 6: Rendering activity
echo -e "${BLUE}Check 6: Rendering Activity${NC}"
RENDER_COUNT=$(grep -c "Rendering.*windows to surface\|Rendered.*windows" "${SERVER_LOG}" || echo "0")
echo "  Render calls: $RENDER_COUNT"
if [ "$RENDER_COUNT" -gt 0 ]; then
    echo -e "${GREEN}  ✓ Rendering is happening${NC}"
    grep "Rendering.*windows\|Rendered.*windows" "${SERVER_LOG}" | tail -3 | sed 's/^/    /'
else
    echo -e "${RED}  ✗ No rendering activity${NC}"
fi
echo ""

# Check 7: Errors
echo -e "${BLUE}Check 7: Error Detection${NC}"
ERROR_COUNT=$(grep -c "ERROR\|panic\|failed.*texture\|Validation Error" "${SERVER_LOG}" || echo "0")
echo "  Errors found: $ERROR_COUNT"
if [ "$ERROR_COUNT" -gt 0 ]; then
    echo -e "${RED}  ✗ Errors detected:${NC}"
    grep "ERROR\|panic\|failed.*texture\|Validation Error" "${SERVER_LOG}" | tail -5 | sed 's/^/    /'
else
    echo -e "${GREEN}  ✓ No errors detected${NC}"
fi
echo ""

# Step 7: Summary
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}   📋 Summary${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
echo ""

echo "Data Flow Analysis:"
echo "  1. Windows created:        $WINDOW_COUNT"
echo "  2. Placeholders pushed:    $PLACEHOLDER_TOTAL"
echo "  3. Textures queued:        $TEXTURE_QUEUE"
echo "  4. Textures processed:     $TEXTURE_PROCESS"
echo "  5. Sync operations:        $SYNC_COUNT"
echo "  6. Render operations:      $RENDER_COUNT"
echo "  7. Errors:                 $ERROR_COUNT"
echo ""

# Identify bottleneck
echo -e "${CYAN}🔍 Bottleneck Analysis:${NC}"
if [ "$WINDOW_COUNT" -eq 0 ]; then
    echo -e "${RED}  ⚠ BOTTLENECK: Windows not being created${NC}"
    echo "  Check Wayland protocol implementation"
elif [ "$PLACEHOLDER_TOTAL" -eq 0 ]; then
    echo -e "${RED}  ⚠ BOTTLENECK: Placeholders not being pushed${NC}"
    echo "  Check push_placeholder_quad calls in server.rs"
elif [ "$TEXTURE_QUEUE" -eq 0 ]; then
    echo -e "${YELLOW}  ⚠ BOTTLENECK: Textures not being queued${NC}"
    echo "  Check queue_texture_update calls and buffer processing"
elif [ "$TEXTURE_PROCESS" -eq 0 ]; then
    echo -e "${YELLOW}  ⚠ BOTTLENECK: Textures not being processed${NC}"
    echo "  Check process_pending_texture_updates implementation"
elif [ "$RENDER_COUNT" -eq 0 ]; then
    echo -e "${RED}  ⚠ BOTTLENECK: Rendering not happening${NC}"
    echo "  Check render loop and surface presentation"
else
    echo -e "${GREEN}  ✓ All pipeline stages are active!${NC}"
fi

echo ""
echo -e "${CYAN}📁 Detailed logs saved to:${NC}"
echo "  Server: ${SERVER_LOG}"
echo "  Client: ${CLIENT_LOG}"
echo ""

# Offer to show full log
read -t 5 -p "View full server log? [y/N] " -n 1 -r || echo ""
echo ""
if [[ $REPLY =~ ^[Yy]$ ]]; then
    less "${SERVER_LOG}"
fi

echo -e "${GREEN}✓ Debug trace complete${NC}"
