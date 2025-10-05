#!/bin/bash
# Simple rendering test for Phase 6.3

set -e

echo "═══════════════════════════════════════════════════════"
echo "  🎨 Axiom Phase 6.3 - Rendering Test"
echo "═══════════════════════════════════════════════════════"
echo ""

# Build first
echo "📦 Building with rendering support..."
cargo build --features wgpu-present --bin run_present_winit --quiet

echo "✅ Build successful!"
echo ""
echo "🚀 Starting Axiom with on-screen window..."
echo "   (A window titled 'Axiom Compositor' should appear)"
echo ""

# Run in background with logging
RUST_LOG=info ./target/debug/run_present_winit --backend auto > /tmp/axiom_render_test.log 2>&1 &
COMPOSITOR_PID=$!

echo "  Compositor PID: $COMPOSITOR_PID"
echo "  Waiting for initialization..."
sleep 5

# Check if it's still running
if ! kill -0 $COMPOSITOR_PID 2>/dev/null; then
    echo ""
    echo "❌ Compositor crashed! Log:"
    tail -50 /tmp/axiom_render_test.log
    exit 1
fi

echo "✅ Compositor is running!"
echo ""

# Extract WAYLAND_DISPLAY
if grep -q "WAYLAND_DISPLAY=" /tmp/axiom_render_test.log; then
    WAYLAND_DISPLAY=$(grep "WAYLAND_DISPLAY=" /tmp/axiom_render_test.log | tail -1 | sed -n 's/.*WAYLAND_DISPLAY=\([a-zA-Z0-9_-]*\).*/\1/p')
    echo "📡 Wayland socket: $WAYLAND_DISPLAY"
    export WAYLAND_DISPLAY
    
    echo ""
    echo "🧪 Launching test client (alacritty)..."
    timeout 10 alacritty -e bash -c "echo '✨ Axiom Rendering Test'; echo 'If you can read this, rendering works!'; sleep 8" 2>&1 &
    CLIENT_PID=$!
    
    echo "  Client PID: $CLIENT_PID"
    echo ""
    echo "👀 Check the Axiom window - you should see the terminal!"
    echo "   Waiting 10 seconds..."
    
    sleep 10
    
    # Check logs for texture updates
    echo ""
    echo "📊 Checking for texture uploads..."
    if grep -q "Updated texture" /tmp/axiom_render_test.log; then
        echo "✅ Texture uploads detected!"
        grep "Updated texture" /tmp/axiom_render_test.log | tail -5
    else
        echo "⚠️  No texture uploads found - rendering may not be working yet"
    fi
else
    echo "⚠️  Could not find WAYLAND_DISPLAY in logs"
fi

echo ""
echo "🛑 Stopping compositor..."
kill $COMPOSITOR_PID 2>/dev/null || true
wait 2>/dev/null || true

echo ""
echo "📝 Last 30 lines of compositor log:"
echo "───────────────────────────────────────────────────────"
tail -30 /tmp/axiom_render_test.log
echo "───────────────────────────────────────────────────────"
echo ""
echo "✅ Test complete!"
echo ""
echo "Full log saved to: /tmp/axiom_render_test.log"

