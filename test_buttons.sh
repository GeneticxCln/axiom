#!/bin/bash
# Test script for Axiom compositor button interactivity

set -e

echo "🚀 Starting Axiom Compositor Test"
echo "=================================="
echo ""

# Set unique Wayland display name for testing
export WAYLAND_DISPLAY=wayland-axiom-test-$$
export RUST_LOG=info

# Clean up any old sockets
rm -f /run/user/$UID/$WAYLAND_DISPLAY 2>/dev/null || true

echo "📍 Test display: $WAYLAND_DISPLAY"
echo "📂 Working directory: $(pwd)"
echo ""

# Start compositor in background
echo "▶️  Starting compositor..."
./target/release/axiom > /tmp/axiom-test-$$.log 2>&1 &
COMPOSITOR_PID=$!

echo "🔧 Compositor PID: $COMPOSITOR_PID"
echo "📝 Log file: /tmp/axiom-test-$$.log"

# Wait for compositor to initialize
echo "⏳ Waiting for compositor to start..."
sleep 3

# Check if compositor is still running
if ! kill -0 $COMPOSITOR_PID 2>/dev/null; then
    echo "❌ Compositor failed to start!"
    echo "Last 20 lines of log:"
    tail -20 /tmp/axiom-test-$$.log
    exit 1
fi

echo "✅ Compositor started successfully!"
echo ""

# Check for Wayland socket
if [ -S "/run/user/$UID/$WAYLAND_DISPLAY" ]; then
    echo "✅ Wayland socket created: /run/user/$UID/$WAYLAND_DISPLAY"
else
    echo "⚠️  Wayland socket not found (expected for headless mode)"
fi

echo ""
echo "📊 Compositor status:"
ps aux | grep -E "axiom.*$$" | grep -v grep || echo "  Process not found in ps"
echo ""

# Show last few lines of log
echo "📖 Recent log output:"
tail -10 /tmp/axiom-test-$$.log
echo ""

# Try to launch a test client
echo "🎨 Attempting to launch test client..."
echo "   (This may fail if compositor is headless)"
WAYLAND_DISPLAY=$WAYLAND_DISPLAY weston-simple-shm > /tmp/client-test-$$.log 2>&1 &
CLIENT_PID=$!

sleep 2

if kill -0 $CLIENT_PID 2>/dev/null; then
    echo "✅ Test client started (PID: $CLIENT_PID)"
else
    echo "⚠️  Test client failed (expected if headless)"
    echo "Client log:"
    cat /tmp/client-test-$$.log
fi

echo ""
echo "📋 Testing Instructions:"
echo "========================"
echo "1. If compositor is running in headless mode, check logs for button events"
echo "2. If you can see a window, test the following:"
echo "   • Move mouse over buttons → should change color"
echo "   • Click close button (red) → window should close"
echo "   • Click minimize button (gray, left) → window minimizes"
echo "   • Click maximize button (gray, middle) → window maximizes"
echo ""
echo "🛑 To stop the test:"
echo "   kill $COMPOSITOR_PID"
echo "   kill $CLIENT_PID"
echo ""
echo "📝 Monitor logs:"
echo "   tail -f /tmp/axiom-test-$$.log"
echo ""
echo "Press Ctrl+C to exit this script (compositor will keep running)"
echo ""

# Keep script alive to show compositor output
trap "echo ''; echo '🛑 Stopping compositor...'; kill $COMPOSITOR_PID 2>/dev/null; kill $CLIENT_PID 2>/dev/null; echo '✅ Stopped'; exit 0" INT

# Tail the log
tail -f /tmp/axiom-test-$$.log
