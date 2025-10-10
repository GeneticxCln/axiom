#!/bin/bash
# Simple test to verify client connectivity with Axiom compositor

# Find the compositor's Wayland socket
WAYLAND_DISPLAY=$(ls -1 /run/user/$(id -u)/wayland-* 2>/dev/null | tail -1 | xargs basename 2>/dev/null)

if [ -z "$WAYLAND_DISPLAY" ]; then
    echo "❌ No Wayland socket found. Is Axiom running?"
    exit 1
fi

echo "✅ Found Wayland socket: $WAYLAND_DISPLAY"
echo ""

# Test 1: Check if weston-info can connect
echo "=== Test 1: Wayland Protocol Introspection ==="
if command -v weston-info &> /dev/null; then
    export WAYLAND_DISPLAY
    timeout 5 weston-info 2>&1 | head -40
    echo ""
else
    echo "⚠️ weston-info not installed. Install with: sudo pacman -S weston"
    echo ""
fi

# Test 2: Try to run weston-simple-shm
echo "=== Test 2: Simple SHM Client Test ==="
if command -v weston-simple-shm &> /dev/null; then
    export WAYLAND_DISPLAY
    echo "Starting weston-simple-shm (will run for 5 seconds)..."
    timeout 5 weston-simple-shm &
    WS_PID=$!
    sleep 2
    
    if ps -p $WS_PID > /dev/null 2>&1; then
        echo "✅ weston-simple-shm is running!"
        echo "   Check if you see a colorful square window in Axiom"
    else
        echo "❌ weston-simple-shm crashed or failed to start"
    fi
    wait $WS_PID 2>/dev/null
    echo ""
else
    echo "⚠️ weston-simple-shm not installed. Install with: sudo pacman -S weston"
    echo ""
fi

# Test 3: Try a real terminal
echo "=== Test 3: Terminal Test ==="
if command -v foot &> /dev/null; then
    echo "You can try running: WAYLAND_DISPLAY=$WAYLAND_DISPLAY foot"
elif command -v alacritty &> /dev/null; then
    echo "You can try running: WAYLAND_DISPLAY=$WAYLAND_DISPLAY alacritty"
elif command -v kitty &> /dev/null; then
    echo "You can try running: WAYLAND_DISPLAY=$WAYLAND_DISPLAY kitty"
else
    echo "⚠️ No Wayland terminal found. Install foot, alacritty, or kitty."
fi

echo ""
echo "=== Axiom Compositor Logs ==="
echo "Check the Axiom logs for:"
echo "  - 'mapped window id=Some(...)' - Window was successfully created"
echo "  - 'renderer now has X windows' - Windows are being tracked"
echo "  - Buffer-related messages showing texture uploads"
