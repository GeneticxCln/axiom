#!/bin/bash
# Test script to verify that the wgpu present error is fixed
# The compositor should run without "No work has been submitted" errors

echo "🧪 Testing Axiom compositor for wgpu errors..."
echo ""
echo "Starting run_present_winit in the background..."
echo "You should see a window open without any 'No work has been submitted' errors."
echo ""
echo "The window will be empty (black) until you connect Wayland clients."
echo ""
echo "Press Ctrl+C to stop the compositor."
echo ""

# Run for 5 seconds and capture errors
timeout 5s cargo run --release --bin run_present_winit --features "smithay,wgpu-present" 2>&1 | \
    grep -E "(ERROR|WARN|No work has been submitted)" || echo "✅ No errors detected!"

echo ""
echo "Test complete!"
