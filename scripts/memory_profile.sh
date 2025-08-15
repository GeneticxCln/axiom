#!/bin/bash
# Memory profiling script for Axiom compositor
# Detects memory leaks and analyzes memory usage patterns

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

echo "🔍 Axiom Memory Profiling Tool"
echo "================================"

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Check for required tools
check_tool() {
    if ! command -v "$1" &> /dev/null; then
        echo -e "${RED}❌ $1 is not installed${NC}"
        echo "Please install $1 to continue"
        exit 1
    fi
}

echo -e "${BLUE}📋 Checking required tools...${NC}"
check_tool "valgrind"
check_tool "heaptrack"
check_tool "cargo"

# Parse command line arguments
PROFILE_TYPE="quick"
BINARY_PATH="./target/debug/axiom"
DEMO_MODE=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --full)
            PROFILE_TYPE="full"
            shift
            ;;
        --release)
            BINARY_PATH="./target/release/axiom"
            shift
            ;;
        --demo)
            DEMO_MODE="--demo"
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --full      Run comprehensive memory analysis (slower)"
            echo "  --release   Profile release build instead of debug"
            echo "  --demo      Run with demo mode enabled"
            echo "  --help      Show this help message"
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            exit 1
            ;;
    esac
done

# Build the project with debug symbols
echo -e "${BLUE}🔨 Building Axiom with debug symbols...${NC}"
if [[ "$BINARY_PATH" == *"release"* ]]; then
    RUSTFLAGS="-C debuginfo=2" cargo build --release
else
    cargo build
fi

# Create output directory for reports
REPORT_DIR="$PROJECT_ROOT/memory_reports"
mkdir -p "$REPORT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Function to run valgrind memory check
run_valgrind() {
    echo -e "${BLUE}🔬 Running Valgrind memory check...${NC}"
    
    VALGRIND_OUTPUT="$REPORT_DIR/valgrind_${TIMESTAMP}.txt"
    
    timeout 30s valgrind \
        --leak-check=full \
        --show-leak-kinds=all \
        --track-origins=yes \
        --verbose \
        --log-file="$VALGRIND_OUTPUT" \
        "$BINARY_PATH" --debug --windowed --no-effects $DEMO_MODE \
        2>&1 | tee "$REPORT_DIR/valgrind_run_${TIMESTAMP}.log"
    
    # Parse valgrind output for leaks
    if grep -q "definitely lost: 0 bytes" "$VALGRIND_OUTPUT" && \
       grep -q "indirectly lost: 0 bytes" "$VALGRIND_OUTPUT"; then
        echo -e "${GREEN}✅ No memory leaks detected by Valgrind${NC}"
    else
        echo -e "${YELLOW}⚠️  Potential memory leaks detected${NC}"
        echo "Details in: $VALGRIND_OUTPUT"
        
        # Extract leak summary
        echo -e "${YELLOW}Leak Summary:${NC}"
        grep -A 10 "LEAK SUMMARY" "$VALGRIND_OUTPUT" || true
    fi
}

# Function to run heaptrack profiling
run_heaptrack() {
    echo -e "${BLUE}📊 Running Heaptrack memory profiling...${NC}"
    
    HEAPTRACK_OUTPUT="$REPORT_DIR/heaptrack_${TIMESTAMP}"
    
    # Run heaptrack
    timeout 30s heaptrack \
        -o "$HEAPTRACK_OUTPUT" \
        "$BINARY_PATH" --debug --windowed --no-effects $DEMO_MODE \
        2>&1 | tee "$REPORT_DIR/heaptrack_run_${TIMESTAMP}.log" || true
    
    # Analyze heaptrack data
    if [ -f "${HEAPTRACK_OUTPUT}.gz" ]; then
        echo -e "${GREEN}✅ Heaptrack data collected${NC}"
        echo "Analyzing memory usage patterns..."
        
        # Generate text report
        heaptrack_print "${HEAPTRACK_OUTPUT}.gz" > "$REPORT_DIR/heaptrack_analysis_${TIMESTAMP}.txt"
        
        # Extract key metrics
        echo -e "${BLUE}Memory Usage Summary:${NC}"
        grep -E "peak heap memory consumption|peak RSS|total memory leaked" \
            "$REPORT_DIR/heaptrack_analysis_${TIMESTAMP}.txt" || true
        
        echo ""
        echo "Full analysis available at: $REPORT_DIR/heaptrack_analysis_${TIMESTAMP}.txt"
        echo "To view interactively, run: heaptrack_gui ${HEAPTRACK_OUTPUT}.gz"
    else
        echo -e "${YELLOW}⚠️  Heaptrack data collection incomplete${NC}"
    fi
}

# Function to run Rust-specific memory analysis
run_rust_analysis() {
    echo -e "${BLUE}🦀 Running Rust-specific memory analysis...${NC}"
    
    # Build with memory profiling features
    echo "Building with allocation tracking..."
    RUSTFLAGS="-C debuginfo=2" cargo build --features memory-profiling 2>/dev/null || {
        echo -e "${YELLOW}Note: memory-profiling feature not available${NC}"
    }
    
    # Run with Rust's built-in allocator stats (if available)
    RUST_BACKTRACE=1 RUST_LOG=debug \
        timeout 20s "$BINARY_PATH" --debug --windowed --no-effects $DEMO_MODE \
        2>&1 | tee "$REPORT_DIR/rust_run_${TIMESTAMP}.log" || true
    
    # Check for common Rust memory issues
    echo -e "${BLUE}Checking for common patterns...${NC}"
    
    # Look for Arc/Rc cycles
    if grep -q "Arc\|Rc" src/**/*.rs; then
        echo -e "${YELLOW}⚠️  Found Arc/Rc usage - check for reference cycles${NC}"
    fi
    
    # Check for large allocations
    if grep -q "Vec::with_capacity([0-9]\{6,\})" src/**/*.rs; then
        echo -e "${YELLOW}⚠️  Found large pre-allocations - verify necessity${NC}"
    fi
}

# Function to monitor runtime memory usage
monitor_runtime() {
    echo -e "${BLUE}📈 Monitoring runtime memory usage...${NC}"
    
    # Start the compositor in background
    "$BINARY_PATH" --debug --windowed --no-effects $DEMO_MODE &
    AXIOM_PID=$!
    
    # Monitor for 20 seconds
    MONITOR_OUTPUT="$REPORT_DIR/runtime_memory_${TIMESTAMP}.csv"
    echo "timestamp,rss_kb,vsz_kb" > "$MONITOR_OUTPUT"
    
    for i in {1..20}; do
        if kill -0 $AXIOM_PID 2>/dev/null; then
            MEMORY_INFO=$(ps -o rss=,vsz= -p $AXIOM_PID 2>/dev/null || echo "0 0")
            echo "$i,$MEMORY_INFO" >> "$MONITOR_OUTPUT"
            
            RSS=$(echo $MEMORY_INFO | awk '{print $1}')
            if [ -n "$RSS" ] && [ "$RSS" -gt 0 ]; then
                echo -ne "\rMemory usage at ${i}s: RSS=${RSS}KB                    "
            fi
            
            sleep 1
        else
            break
        fi
    done
    
    echo ""
    
    # Kill the process if still running
    kill $AXIOM_PID 2>/dev/null || true
    wait $AXIOM_PID 2>/dev/null || true
    
    echo -e "${GREEN}✅ Runtime monitoring complete${NC}"
    echo "Data saved to: $MONITOR_OUTPUT"
}

# Main execution
echo ""
echo -e "${BLUE}🚀 Starting memory profiling (${PROFILE_TYPE} mode)...${NC}"
echo ""

case $PROFILE_TYPE in
    quick)
        run_rust_analysis
        monitor_runtime
        ;;
    full)
        run_valgrind
        run_heaptrack
        run_rust_analysis
        monitor_runtime
        ;;
esac

# Generate summary report
SUMMARY_REPORT="$REPORT_DIR/summary_${TIMESTAMP}.md"
{
    echo "# Axiom Memory Profile Report"
    echo "Generated: $(date)"
    echo ""
    echo "## Configuration"
    echo "- Profile Type: $PROFILE_TYPE"
    echo "- Binary: $BINARY_PATH"
    echo "- Demo Mode: ${DEMO_MODE:-disabled}"
    echo ""
    echo "## Results"
    echo ""
    
    # Add valgrind results if available
    if [ -f "$REPORT_DIR/valgrind_${TIMESTAMP}.txt" ]; then
        echo "### Valgrind Analysis"
        grep "LEAK SUMMARY" -A 5 "$REPORT_DIR/valgrind_${TIMESTAMP}.txt" || echo "No leaks detected"
        echo ""
    fi
    
    # Add heaptrack results if available
    if [ -f "$REPORT_DIR/heaptrack_analysis_${TIMESTAMP}.txt" ]; then
        echo "### Heaptrack Analysis"
        grep -E "peak|total" "$REPORT_DIR/heaptrack_analysis_${TIMESTAMP}.txt" | head -5
        echo ""
    fi
    
    # Add runtime monitoring results
    if [ -f "$MONITOR_OUTPUT" ]; then
        echo "### Runtime Memory Usage"
        echo "Peak RSS: $(awk -F',' 'NR>1 {if($2>max) max=$2} END {print max}' $MONITOR_OUTPUT) KB"
        echo ""
    fi
    
    echo "## Recommendations"
    echo ""
    echo "1. Review any detected leaks in valgrind output"
    echo "2. Check heaptrack data for unexpected growth patterns"
    echo "3. Monitor RSS growth over extended runtime"
    echo "4. Consider using jemalloc for better memory statistics"
    
} > "$SUMMARY_REPORT"

echo ""
echo -e "${GREEN}📊 Memory profiling complete!${NC}"
echo ""
echo "Reports generated in: $REPORT_DIR"
echo "Summary report: $SUMMARY_REPORT"
echo ""

# Display summary
cat "$SUMMARY_REPORT"

echo ""
echo -e "${BLUE}💡 Tips for memory optimization:${NC}"
echo "1. Use 'cargo flamegraph' to identify hot allocation paths"
echo "2. Enable jemalloc with 'jemallocator' crate for better stats"
echo "3. Use 'dhat' crate for detailed heap profiling"
echo "4. Consider 'tokio-console' for async runtime analysis"
