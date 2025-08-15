#!/bin/bash
# Axiom Compositor Test Runner
# 
# Comprehensive test script for local development and CI environments

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TEST_TIMEOUT=300  # 5 minutes
VERBOSE=${VERBOSE:-false}
HEADLESS=${HEADLESS:-true}

echo -e "${BLUE}🧪 Axiom Compositor Test Suite${NC}"
echo "=================================="

# Function to print section headers
print_section() {
    echo -e "\n${BLUE}🔍 $1${NC}"
    echo "$(printf '%.0s-' {1..40})"
}

# Function to run tests with proper error handling
run_test() {
    local test_name="$1"
    local test_command="$2"
    local allow_failure="${3:-false}"
    
    echo -e "\n${YELLOW}Running: $test_name${NC}"
    echo "Command: $test_command"
    
    if [[ "$VERBOSE" == "true" ]]; then
        local cmd_verbose="$test_command --verbose"
    else
        local cmd_verbose="$test_command"
    fi
    
    local start_time=$(date +%s)
    
    if timeout $TEST_TIMEOUT bash -c "$cmd_verbose"; then
        local end_time=$(date +%s)
        local duration=$((end_time - start_time))
        echo -e "${GREEN}✅ $test_name completed in ${duration}s${NC}"
        return 0
    else
        local exit_code=$?
        local end_time=$(date +%s)
        local duration=$((end_time - start_time))
        
        if [[ "$allow_failure" == "true" ]]; then
            echo -e "${YELLOW}⚠️  $test_name failed (allowed) after ${duration}s${NC}"
            return 0
        else
            echo -e "${RED}❌ $test_name failed after ${duration}s (exit code: $exit_code)${NC}"
            return $exit_code
        fi
    fi
}

# Function to check prerequisites
check_prerequisites() {
    print_section "Checking Prerequisites"
    
    # Check Rust toolchain
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}❌ Cargo not found. Please install Rust.${NC}"
        exit 1
    fi
    echo -e "${GREEN}✅ Cargo found: $(cargo --version)${NC}"
    
    # Check for required system libraries (on Linux)
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        local missing_libs=()
        
        if ! pkg-config --exists wayland-client; then
            missing_libs+=("libwayland-dev")
        fi
        
        if ! pkg-config --exists xkbcommon; then
            missing_libs+=("libxkbcommon-dev")
        fi
        
        if [[ ${#missing_libs[@]} -gt 0 ]]; then
            echo -e "${YELLOW}⚠️  Missing system libraries: ${missing_libs[*]}${NC}"
            echo "Install with: sudo apt-get install ${missing_libs[*]}"
        else
            echo -e "${GREEN}✅ Required system libraries found${NC}"
        fi
    fi
    
    # Check for virtual display in headless mode
    if [[ "$HEADLESS" == "true" ]] && [[ "$OSTYPE" == "linux-gnu"* ]]; then
        if ! command -v Xvfb &> /dev/null; then
            echo -e "${YELLOW}⚠️  Xvfb not found. Some graphics tests may fail.${NC}"
            echo "Install with: sudo apt-get install xvfb"
        else
            echo -e "${GREEN}✅ Xvfb found for headless testing${NC}"
        fi
    fi
}

# Function to run different test categories
run_unit_tests() {
    print_section "Unit Tests"
    run_test "Unit Tests" "cargo test --lib"
}

run_property_tests() {
    print_section "Property-Based Tests"
    run_test "Property Tests - Config" "cargo test --lib config::property_tests"
    run_test "Property Tests - Workspace" "cargo test --lib workspace::property_tests"
}

run_integration_tests() {
    print_section "Integration Tests"
    
    if [[ "$HEADLESS" == "true" ]] && command -v Xvfb &> /dev/null; then
        echo "Running integration tests with virtual display..."
        export DISPLAY=:99
        Xvfb :99 -screen 0 1024x768x24 &
        local xvfb_pid=$!
        sleep 2
        
        run_test "Integration Tests" "cargo test --test integration_tests" true
        
        kill $xvfb_pid 2>/dev/null || true
    else
        echo "Running integration tests without virtual display..."
        run_test "Integration Tests" "cargo test --test integration_tests" true
    fi
}

run_doc_tests() {
    print_section "Documentation Tests"
    run_test "Doc Tests" "cargo test --doc"
}

run_benchmark_tests() {
    print_section "Benchmark Tests"
    
    if [[ -d "benches" ]]; then
        run_test "Benchmarks" "cargo bench --no-run" true
    else
        echo -e "${YELLOW}ℹ️  No benchmark directory found, skipping${NC}"
    fi
}

run_format_check() {
    print_section "Code Formatting"
    run_test "Format Check" "cargo fmt -- --check"
}

run_clippy_check() {
    print_section "Clippy Linting"
    run_test "Clippy" "cargo clippy --all-targets --all-features -- -D warnings"
}

run_security_audit() {
    print_section "Security Audit"
    
    if command -v cargo-audit &> /dev/null; then
        run_test "Security Audit" "cargo audit" true
    else
        echo -e "${YELLOW}ℹ️  cargo-audit not installed, skipping${NC}"
        echo "Install with: cargo install cargo-audit"
    fi
}

# Function to generate test report
generate_report() {
    print_section "Test Summary"
    
    local total_tests=0
    local passed_tests=0
    
    # Count test results (simplified - in real implementation would parse test output)
    echo -e "${GREEN}Test execution completed${NC}"
    echo "Check individual test outputs above for detailed results"
    
    # Check if any critical files exist
    if [[ -f "Cargo.toml" ]]; then
        echo -e "${GREEN}✅ Project structure validated${NC}"
    fi
    
    if [[ -d "src" ]]; then
        echo -e "${GREEN}✅ Source directory found${NC}"
    fi
    
    if [[ -d "tests" ]]; then
        echo -e "${GREEN}✅ Test directory found${NC}"
    fi
}

# Main execution function
main() {
    local mode="${1:-all}"
    local start_time=$(date +%s)
    
    check_prerequisites
    
    case "$mode" in
        "unit")
            run_unit_tests
            ;;
        "property")
            run_property_tests
            ;;
        "integration")
            run_integration_tests
            ;;
        "lint")
            run_format_check
            run_clippy_check
            ;;
        "security")
            run_security_audit
            ;;
        "docs")
            run_doc_tests
            ;;
        "bench")
            run_benchmark_tests
            ;;
        "quick")
            echo -e "${BLUE}🚀 Running quick test suite${NC}"
            run_format_check
            run_clippy_check
            run_unit_tests
            ;;
        "ci")
            echo -e "${BLUE}🤖 Running CI test suite${NC}"
            run_format_check
            run_clippy_check
            run_unit_tests
            run_property_tests
            run_integration_tests
            run_doc_tests
            run_security_audit
            ;;
        "all"|*)
            echo -e "${BLUE}🔬 Running comprehensive test suite${NC}"
            run_format_check
            run_clippy_check
            run_unit_tests
            run_property_tests
            run_integration_tests
            run_doc_tests
            run_benchmark_tests
            run_security_audit
            ;;
    esac
    
    generate_report
    
    local end_time=$(date +%s)
    local total_duration=$((end_time - start_time))
    
    echo -e "\n${GREEN}🎉 Test suite completed in ${total_duration}s${NC}"
}

# Help function
show_help() {
    echo "Usage: $0 [mode]"
    echo ""
    echo "Test Modes:"
    echo "  all          - Run all tests (default)"
    echo "  unit         - Unit tests only"
    echo "  property     - Property-based tests only"
    echo "  integration  - Integration tests only"
    echo "  lint         - Code formatting and linting"
    echo "  security     - Security audit"
    echo "  docs         - Documentation tests"
    echo "  bench        - Benchmark tests"
    echo "  quick        - Fast test suite (lint + unit)"
    echo "  ci           - CI test suite"
    echo "  help         - Show this help"
    echo ""
    echo "Environment Variables:"
    echo "  VERBOSE=true    - Enable verbose output"
    echo "  HEADLESS=false  - Disable headless mode"
    echo ""
    echo "Examples:"
    echo "  $0              # Run all tests"
    echo "  $0 quick        # Quick test run"
    echo "  VERBOSE=true $0 unit  # Verbose unit tests"
}

# Parse arguments
if [[ "${1:-}" == "help" ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    show_help
    exit 0
fi

# Run main function
main "${1:-all}"
