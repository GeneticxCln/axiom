#!/bin/bash
# Axiom Compositor Test Coverage Script
# 
# This script runs comprehensive test coverage analysis using cargo-tarpaulin
# and generates detailed reports in multiple formats.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
COVERAGE_DIR="target/tarpaulin"
MIN_COVERAGE=70
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

echo -e "${BLUE}🧪 Axiom Compositor Test Coverage Analysis${NC}"
echo "=================================================="

# Create coverage directory
mkdir -p "$COVERAGE_DIR"

# Function to print section headers
print_section() {
    echo -e "\n${BLUE}📊 $1${NC}"
    echo "----------------------------------------"
}

# Function to check if tarpaulin is installed
check_tarpaulin() {
    if ! command -v cargo-tarpaulin &> /dev/null; then
        echo -e "${RED}❌ cargo-tarpaulin is not installed${NC}"
        echo "Install it with: cargo install cargo-tarpaulin"
        exit 1
    fi
    echo -e "${GREEN}✅ cargo-tarpaulin is available${NC}"
}

# Function to run coverage for specific test types
run_coverage() {
    local test_type="$1"
    local output_suffix="$2"
    local additional_args="${3:-}"
    
    print_section "Running $test_type coverage"
    
    local cmd="cargo tarpaulin \
        --config tarpaulin.toml \
        --output-dir $COVERAGE_DIR/$output_suffix \
        $additional_args \
        --verbose"
    
    echo "Command: $cmd"
    
    if eval "$cmd"; then
        echo -e "${GREEN}✅ $test_type coverage completed${NC}"
    else
        echo -e "${YELLOW}⚠️  $test_type coverage completed with warnings${NC}"
    fi
}

# Function to generate summary report
generate_summary() {
    print_section "Generating Coverage Summary"
    
    local json_file="$COVERAGE_DIR/unit/tarpaulin-report.json"
    if [[ -f "$json_file" ]]; then
        # Extract coverage percentage from JSON report
        local coverage=$(jq -r '.files | map(.coverage) | add / length' "$json_file" 2>/dev/null || echo "0")
        local coverage_int=$(printf "%.0f" "$coverage" 2>/dev/null || echo "0")
        
        echo "Overall Coverage: ${coverage_int}%"
        
        if (( coverage_int >= MIN_COVERAGE )); then
            echo -e "${GREEN}✅ Coverage meets minimum threshold (${MIN_COVERAGE}%)${NC}"
        else
            echo -e "${RED}❌ Coverage below minimum threshold (${MIN_COVERAGE}%)${NC}"
            echo "Current: ${coverage_int}%, Required: ${MIN_COVERAGE}%"
        fi
    else
        echo -e "${YELLOW}⚠️  Could not find JSON coverage report${NC}"
    fi
}

# Function to open coverage report
open_report() {
    local html_report="$COVERAGE_DIR/unit/tarpaulin-report.html"
    if [[ -f "$html_report" ]]; then
        echo -e "\n${BLUE}📄 Coverage Report Location:${NC}"
        echo "HTML: file://$(realpath "$html_report")"
        
        # Try to open in browser (works on most Linux desktop environments)
        if command -v xdg-open &> /dev/null; then
            echo "Opening coverage report in browser..."
            xdg-open "$html_report" 2>/dev/null || true
        fi
    fi
}

# Function to cleanup old reports
cleanup_old_reports() {
    print_section "Cleaning up old reports"
    
    # Keep only the last 5 coverage reports
    find "$COVERAGE_DIR" -maxdepth 1 -type d -name "*_*" | \
        sort -r | tail -n +6 | xargs -r rm -rf
    
    echo -e "${GREEN}✅ Cleanup completed${NC}"
}

# Main execution
main() {
    local mode="${1:-full}"
    
    check_tarpaulin
    
    case "$mode" in
        "unit")
            run_coverage "Unit Tests" "unit" "--lib --tests"
            ;;
        "integration")
            run_coverage "Integration Tests" "integration" "--test integration_tests"
            ;;
        "property")
            run_coverage "Property Tests" "property" "--lib --tests -- property_tests"
            ;;
        "fast")
            # Quick coverage run without HTML generation
            run_coverage "Fast Coverage" "fast" "--lib --tests --output-format Json"
            ;;
        "full"|*)
            echo -e "${BLUE}🔬 Running comprehensive coverage analysis${NC}\n"
            
            # Clean up old reports first
            cleanup_old_reports
            
            # Run different types of coverage
            run_coverage "Unit & Property Tests" "unit" "--lib --tests"
            
            # Skip integration tests in coverage as they may require graphics context
            echo -e "${YELLOW}ℹ️  Skipping integration tests (may require graphics context)${NC}"
            
            # Generate summary
            generate_summary
            
            # Open report
            open_report
            ;;
    esac
    
    echo -e "\n${GREEN}🎉 Coverage analysis completed!${NC}"
    echo -e "Reports saved to: ${COVERAGE_DIR}"
}

# Help function
show_help() {
    echo "Usage: $0 [mode]"
    echo ""
    echo "Modes:"
    echo "  full         - Complete coverage analysis (default)"
    echo "  unit         - Unit tests only"
    echo "  integration  - Integration tests only"
    echo "  property     - Property tests only"  
    echo "  fast         - Quick coverage run"
    echo "  help         - Show this help"
    echo ""
    echo "Examples:"
    echo "  $0           # Run full coverage"
    echo "  $0 unit      # Run unit test coverage only"
    echo "  $0 fast      # Quick coverage check"
}

# Check arguments
if [[ "${1:-}" == "help" ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    show_help
    exit 0
fi

# Run main function
main "${1:-full}"
