#!/bin/bash
# Test script for Axiom tiling window management

echo "🚀 Starting Axiom Compositor with tiling test..."

# Kill any existing axiom processes
pkill -9 axiom 2>/dev/null
pkill -9 run_present 2>/dev/null
sleep 1

# Start the compositor in windowed mode
./target/release/run_present_winit &
COMPOSITOR_PID=$!

echo "⏳ Waiting for compositor to initialize..."
sleep 3

# The compositor creates wayland-2 by default (we see this in logs)
export WAYLAND_DISPLAY=wayland-2

# Verify the socket exists
if [ ! -S "/run/user/$UID/$WAYLAND_DISPLAY" ]; then
    echo "❌ Error: Wayland socket not found at /run/user/$UID/$WAYLAND_DISPLAY"
    echo "Available sockets:"
    ls -la /run/user/$UID/wayland-* 2>/dev/null || echo "  None found"
    kill $COMPOSITOR_PID 2>/dev/null
    exit 1
fi

echo "✅ Compositor running on $WAYLAND_DISPLAY"

# Wait a bit more for full initialization
sleep 2

echo "📦 Launching test windows..."

# Try different terminal emulators (use what's available)
if command -v weston-terminal &> /dev/null; then
    echo "  Using weston-terminal..."
    for i in {1..3}; do
        weston-terminal &
        sleep 0.5
    done
elif command -v foot &> /dev/null; then
    echo "  Using foot..."
    for i in {1..3}; do
        foot &
        sleep 0.5
    done
elif command -v alacritty &> /dev/null; then
    echo "  Using alacritty..."
    for i in {1..3}; do
        alacritty &
        sleep 0.5
    done
elif command -v kitty &> /dev/null; then
    echo "  Using kitty..."
    for i in {1..3}; do
        kitty &
        sleep 0.5
    done
else
    echo "⚠️  No Wayland terminal emulator found!"
    echo "   Install one: sudo pacman -S foot"
fi

echo ""
echo "✨ Axiom Tiling Test Ready!"
echo ""
echo "🎹 Keyboard Shortcuts to Test:"
echo "  Super + L            → Cycle layout modes"
echo "  Super + J            → Focus next window"
echo "  Super + K            → Focus previous window"
echo "  Super + Shift + J    → Move window down"
echo "  Super + Shift + K    → Move window up"
echo "  Super + Left         → Previous workspace"
echo "  Super + Right        → Next workspace"
echo "  Super + Shift + Left → Move window to left workspace"
echo "  Super + Shift + Right→ Move window to right workspace"
echo ""
echo "Press Ctrl+C to stop the compositor"

# Keep script running
wait $COMPOSITOR_PID
