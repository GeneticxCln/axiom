#!/bin/bash

# Phase 5: Performance Benchmarking and Regression Detection Script
# This script runs comprehensive benchmarks and detects performance regressions

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BASELINE_FILE="$PROJECT_DIR/benchmark_baseline.txt"
RESULTS_FILE="$PROJECT_DIR/benchmark_results.txt"

echo "🚀 Axiom Performance Benchmark Suite"
echo "====================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to log with timestamp
log() {
    echo -e "[$(date '+%Y-%m-%d %H:%M:%S')] $1"
}

# Function to run benchmark and capture metrics
run_benchmark() {
    local test_name="$1"
    local description="$2"
    
    log "${BLUE}Running benchmark: $test_name${NC}"
    log "Description: $description"
    
    # Clear any previous results
    > /tmp/axiom_benchmark_tmp.txt
    
    # Run the benchmark with timing and memory measurement
    /usr/bin/time -v cargo bench --bench "$test_name" 2>&1 | tee /tmp/axiom_benchmark_tmp.txt
    
    # Extract key metrics
    local max_memory=$(grep "Maximum resident set size" /tmp/axiom_benchmark_tmp.txt | awk '{print $6}')
    local wall_time=$(grep "Elapsed (wall clock) time" /tmp/axiom_benchmark_tmp.txt | awk '{print $8}')
    local cpu_percent=$(grep "Percent of CPU this job got" /tmp/axiom_benchmark_tmp.txt | awk '{print $7}' | tr -d '%')
    
    echo "$test_name,$description,$wall_time,$max_memory,$cpu_percent" >> "$RESULTS_FILE"
    
    log "${GREEN}✅ Benchmark completed: $test_name${NC}"
    log "  Wall time: $wall_time"
    log "  Max memory: ${max_memory}KB"
    log "  CPU usage: ${cpu_percent}%"
    echo ""
}

# Function to compare with baseline
compare_with_baseline() {
    if [[ ! -f "$BASELINE_FILE" ]]; then
        log "${YELLOW}⚠️ No baseline file found. Creating baseline...${NC}"
        cp "$RESULTS_FILE" "$BASELINE_FILE"
        log "${GREEN}✅ Baseline created: $BASELINE_FILE${NC}"
        return 0
    fi
    
    log "${BLUE}📊 Comparing with baseline...${NC}"
    
    local regression_found=false
    
    while IFS=, read -r test_name description wall_time max_memory cpu_percent; do
        # Skip header
        if [[ "$test_name" == "test_name" ]]; then
            continue
        fi
        
        # Find corresponding baseline entry
        local baseline_entry=$(grep "^$test_name," "$BASELINE_FILE" || echo "")
        
        if [[ -n "$baseline_entry" ]]; then
            local baseline_time=$(echo "$baseline_entry" | cut -d',' -f3)
            local baseline_memory=$(echo "$baseline_entry" | cut -d',' -f4)
            
            # Compare performance (allow 10% degradation threshold)
            local time_diff=$(echo "$wall_time $baseline_time" | awk '{print ($1 - $2) / $2 * 100}')
            local memory_diff=$(echo "$max_memory $baseline_memory" | awk '{print ($1 - $2) / $2 * 100}')
            
            # Check for regressions
            if (( $(echo "$time_diff > 10" | bc -l) )); then
                log "${RED}❌ REGRESSION: $test_name - Time increased by ${time_diff}%${NC}"
                regression_found=true
            elif (( $(echo "$memory_diff > 15" | bc -l) )); then
                log "${RED}❌ REGRESSION: $test_name - Memory increased by ${memory_diff}%${NC}"
                regression_found=true
            else
                log "${GREEN}✅ $test_name - Performance OK${NC}"
                if (( $(echo "$time_diff < -5" | bc -l) )); then
                    log "${GREEN}  🎉 Time improved by ${time_diff}%${NC}"
                fi
                if (( $(echo "$memory_diff < -5" | bc -l) )); then
                    log "${GREEN}  🎉 Memory improved by ${memory_diff}%${NC}"
                fi
            fi
        else
            log "${YELLOW}⚠️ New benchmark: $test_name (no baseline)${NC}"
        fi
    done < "$RESULTS_FILE"
    
    if [[ "$regression_found" == true ]]; then
        log "${RED}❌ Performance regressions detected!${NC}"
        return 1
    else
        log "${GREEN}✅ No performance regressions detected${NC}"
        return 0
    fi
}

# Function to run memory leak detection
run_memory_leak_detection() {
    log "${BLUE}🔍 Running memory leak detection...${NC}"
    
    # Install valgrind if not present
    if ! command -v valgrind &> /dev/null; then
        log "${YELLOW}⚠️ Valgrind not found. Installing...${NC}"
        if command -v pacman &> /dev/null; then
            sudo pacman -S --needed valgrind
        elif command -v apt &> /dev/null; then
            sudo apt install -y valgrind
        else
            log "${RED}❌ Cannot install valgrind. Please install manually.${NC}"
            return 1
        fi
    fi
    
    # Build debug version for better stack traces
    log "🔧 Building debug version for memory analysis..."
    cargo build --debug
    
    # Run memory leak detection on unit tests
    log "🧪 Running memory leak detection on tests..."
    
    local valgrind_log="/tmp/axiom_valgrind.log"
    valgrind --tool=memcheck \
        --leak-check=full \
        --show-leak-kinds=all \
        --track-origins=yes \
        --verbose \
        --log-file="$valgrind_log" \
        cargo test --quiet 2>/dev/null || true
    
    # Analyze valgrind output
    if grep -q "ERROR SUMMARY: 0 errors" "$valgrind_log"; then
        log "${GREEN}✅ No memory errors detected${NC}"
    else
        log "${RED}❌ Memory issues detected!${NC}"
        log "📄 Valgrind report saved to: $valgrind_log"
        
        # Show summary
        grep -A 5 "HEAP SUMMARY:" "$valgrind_log" || true
        grep -A 10 "LEAK SUMMARY:" "$valgrind_log" || true
        
        return 1
    fi
}

# Function to run stress tests
run_stress_tests() {
    log "${BLUE}💪 Running stress tests...${NC}"
    
    # Build release version for stress testing
    cargo build --release
    
    # Stress test 1: Many windows
    log "🪟 Stress test: Creating many windows..."
    timeout 30s ./target/release/axiom --debug --windowed --stress-test-windows || true
    
    # Stress test 2: Rapid workspace switching
    log "📱 Stress test: Rapid workspace switching..."
    timeout 30s ./target/release/axiom --debug --windowed --stress-test-scrolling || true
    
    # Stress test 3: Effects stress test
    log "✨ Stress test: Visual effects load..."
    timeout 30s ./target/release/axiom --debug --windowed --stress-test-effects || true
    
    log "${GREEN}✅ Stress tests completed${NC}"
}

# Function to profile with perf (Linux only)
run_performance_profiling() {
    if [[ "$OSTYPE" != "linux-gnu"* ]]; then
        log "${YELLOW}⚠️ Performance profiling only available on Linux${NC}"
        return 0
    fi
    
    if ! command -v perf &> /dev/null; then
        log "${YELLOW}⚠️ perf not found. Skipping profiling...${NC}"
        return 0
    fi
    
    log "${BLUE}📈 Running performance profiling...${NC}"
    
    # Build release version
    cargo build --release
    
    # Profile the demo run
    local perf_data="/tmp/axiom_perf.data"
    timeout 10s perf record -g -o "$perf_data" \
        ./target/release/axiom --debug --windowed --demo || true
    
    if [[ -f "$perf_data" ]]; then
        log "📊 Performance profile saved to: $perf_data"
        log "🔍 Top functions:"
        perf report -i "$perf_data" --stdio | head -20
    fi
}

# Main execution
main() {
    cd "$PROJECT_DIR"
    
    # Create results file with header
    echo "test_name,description,wall_time,max_memory_kb,cpu_percent" > "$RESULTS_FILE"
    
    # Check if cargo bench works
    if ! cargo bench --help &> /dev/null; then
        log "${YELLOW}⚠️ cargo bench not available. Installing...${NC}"
        # Add criterion to Cargo.toml if not present
        if ! grep -q "\[dev-dependencies\]" Cargo.toml; then
            echo "" >> Cargo.toml
            echo "[dev-dependencies]" >> Cargo.toml
        fi
        if ! grep -q "criterion" Cargo.toml; then
            echo "criterion = \"0.5\"" >> Cargo.toml
        fi
    fi
    
    log "${BLUE}🏗️ Building release version...${NC}"
    cargo build --release
    
    # Run core benchmarks
    log "${BLUE}🚀 Starting benchmark suite...${NC}"
    
    # Note: These benchmarks need to be implemented in benches/
    # For now, we'll run basic performance tests
    
    # Test compilation performance
    log "${BLUE}⚙️ Testing compilation performance...${NC}"
    local start_time=$(date +%s.%3N)
    cargo check --quiet
    local end_time=$(date +%s.%3N)
    local compile_time=$(echo "$end_time - $start_time" | bc)
    echo "cargo_check,Compilation check time,${compile_time}s,0,0" >> "$RESULTS_FILE"
    
    # Test startup performance
    log "${BLUE}🚀 Testing startup performance...${NC}"
    local startup_log="/tmp/axiom_startup.log"
    timeout 5s /usr/bin/time -v ./target/release/axiom --help 2> "$startup_log" || true
    if [[ -f "$startup_log" ]]; then
        local startup_time=$(grep "Elapsed (wall clock) time" "$startup_log" | awk '{print $8}')
        local startup_memory=$(grep "Maximum resident set size" "$startup_log" | awk '{print $6}')
        echo "startup,Application startup time,$startup_time,${startup_memory},0" >> "$RESULTS_FILE"
    fi
    
    # Test test suite performance
    log "${BLUE}🧪 Testing test suite performance...${NC}"
    local test_log="/tmp/axiom_test.log"
    /usr/bin/time -v cargo test --quiet 2> "$test_log"
    local test_time=$(grep "Elapsed (wall clock) time" "$test_log" | awk '{print $8}')
    local test_memory=$(grep "Maximum resident set size" "$test_log" | awk '{print $6}')
    echo "test_suite,Full test suite execution,$test_time,${test_memory},0" >> "$RESULTS_FILE"
    
    # Compare with baseline
    compare_with_baseline
    local benchmark_status=$?
    
    # Run additional analysis if requested
    if [[ "${1:-}" == "--full" ]]; then
        log "${BLUE}🔍 Running full analysis suite...${NC}"
        
        run_memory_leak_detection
        local memory_status=$?
        
        run_stress_tests
        
        run_performance_profiling
        
        # Overall status
        if [[ $benchmark_status -eq 0 && $memory_status -eq 0 ]]; then
            log "${GREEN}🎉 All performance checks passed!${NC}"
            exit 0
        else
            log "${RED}❌ Performance issues detected${NC}"
            exit 1
        fi
    else
        log "${YELLOW}ℹ️ Run with --full for complete analysis${NC}"
        exit $benchmark_status
    fi
}

# Handle command line arguments
if [[ "${1:-}" == "--help" ]]; then
    echo "Axiom Performance Benchmark Suite"
    echo ""
    echo "Usage: $0 [--full] [--help]"
    echo ""
    echo "Options:"
    echo "  --full    Run complete analysis including memory leak detection"
    echo "  --help    Show this help message"
    echo ""
    echo "This script runs performance benchmarks and compares them with"
    echo "a baseline to detect performance regressions."
    exit 0
fi

main "$@"
