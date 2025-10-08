#!/bin/bash
# Simple build progress monitor

echo "🔨 Axiom Build Monitor"
echo "======================"
echo ""

while true; do
    # Clear screen
    clear
    echo "🔨 Axiom Build Monitor - $(date +%H:%M:%S)"
    echo "=========================================="
    echo ""
    
    # Check if cargo is running
    if ps aux | grep -q "[c]argo build.*axiom"; then
        echo "✅ Build is RUNNING"
        
        # Count compiled crates
        COMPILED=$(grep -c "Compiling" /tmp/axiom_build.log 2>/dev/null || echo "0")
        echo "📦 Crates compiled: $COMPILED"
        
        # Show last 5 lines
        echo ""
        echo "📋 Recent activity:"
        tail -5 /tmp/axiom_build.log 2>/dev/null | grep "Compiling" | tail -3 || echo "   (waiting for output...)"
        
        # Show active rustc processes
        RUSTC_COUNT=$(ps aux | grep -c "[r]ustc")
        echo ""
        echo "⚙️  Active compiler processes: $RUSTC_COUNT"
        
    else
        echo "❌ Build NOT running"
        
        # Check if finished
        if grep -q "Finished" /tmp/axiom_build.log 2>/dev/null; then
            echo ""
            echo "🎉 BUILD COMPLETE!"
            echo ""
            grep "Finished" /tmp/axiom_build.log
            echo ""
            echo "Binary location: target/debug/run_present_winit"
            break
        else
            echo ""
            echo "⚠️  Build may have stopped or failed"
            echo "Check /tmp/axiom_build.log for details"
            break
        fi
    fi
    
    echo ""
    echo "Press Ctrl+C to stop monitoring"
    sleep 3
done
