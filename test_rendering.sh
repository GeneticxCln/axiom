#!/bin/bash
# Test Axiom compositor with visual rendering

echo "🚀 Starting Axiom Compositor with visual rendering..."

# Start the presenter
./target/release/run_present_winit --backend auto 2>&1 | tee /tmp/axiom_present.log &
PRESENTER_PID=$!

echo "⏳ Waiting for compositor to initialize..."
sleep 3

# Extract WAYLAND_DISPLAY from logs
WAYLAND_DISPLAY=$(grep "WAYLAND_DISPLAY=" /tmp/axiom_present.log | tail -1 | cut -d'=' -f2)
if [ -z "$WAYLAND_DISPLAY" ]; then
    WAYLAND_DISPLAY="wayland-2"  # fallback
fi

echo "✅ Compositor ready on $WAYLAND_DISPLAY"
echo ""
echo "📺 You should now see an Axiom window on your screen!"
echo ""
echo "🧪 Testing with weston-terminal..."

# Launch test client
export WAYLAND_DISPLAY
weston-terminal &
CLIENT_PID=$!

echo ""
echo "🎬 Client launched! Window should appear in the Axiom compositor."
echo "   Press Ctrl+C to stop, or wait 30 seconds..."
echo ""

# Keep running for 30 seconds
sleep 30

echo ""
echo "🛑 Stopping test..."
kill $CLIENT_PID 2>/dev/null || true
kill $PRESENTER_PID 2>/dev/null || true
wait 2>/dev/null || true

echo "✅ Test complete!"