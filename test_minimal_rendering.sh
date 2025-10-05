#!/bin/bash
# Test rendering with the minimal server that we know works

echo "🧪 Testing Axiom Rendering with Minimal Server"
echo "=============================================="
echo ""

# Start minimal server
echo "Starting minimal Wayland server..."
RUST_LOG=info ./target/debug/run_minimal_wayland > /tmp/axiom_minimal.log 2>&1 &
SERVER_PID=$!

sleep 3

# Get WAYLAND_DISPLAY
WAYLAND_DISPLAY=$(grep "WAYLAND_DISPLAY=" /tmp/axiom_minimal.log | tail -1 | sed -n 's/.*WAYLAND_DISPLAY=\([a-zA-Z0-9_-]*\).*/\1/p')
export WAYLAND_DISPLAY

echo "✅ Server running on $WAYLAND_DISPLAY"
echo ""
echo "Now we need a headless renderer that consumes from the"
echo "same SharedRenderState..."
echo ""
echo "The issue: minimal server doesn't have a presenter!"
echo "We need to use run_present_winit which HAS both."
echo ""

# Kill server
kill $SERVER_PID 2>/dev/null

echo "Solution: The run_present_winit DOES have both server AND renderer."
echo "The problem is clients are segfaulting."
echo ""
echo "Let's check the logs for WHY clients segfault..."

