#!/usr/bin/env bash
# Phase 5 Development Kickoff Script
# 
# This script sets up the development environment and runs initial tests
# to establish baselines for the Phase 5 development cycle.

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Project root directory
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

echo -e "${BLUE}🚀 Axiom Phase 5 Development Kickoff${NC}"
echo -e "${BLUE}=====================================${NC}"
echo

# Function to print status messages
status() {
    echo -e "${GREEN}✅ $1${NC}"
}

warning() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

error() {
    echo -e "${RED}❌ $1${NC}"
}

# Check prerequisites
echo -e "${BLUE}📋 Checking Prerequisites${NC}"

# Check Rust toolchain
if ! command -v cargo &> /dev/null; then
    error "Rust/Cargo not found. Please install Rust: https://rustup.rs/"
    exit 1
fi
status "Rust toolchain found: $(rustc --version)"

# Check for required tools
MISSING_TOOLS=()
for tool in git jq valgrind; do
    if ! command -v "$tool" &> /dev/null; then
        MISSING_TOOLS+=("$tool")
    fi
done

if [ ${#MISSING_TOOLS[@]} -gt 0 ]; then
    warning "Missing tools: ${MISSING_TOOLS[*]}"
    echo "Install them with:"
    echo "  Ubuntu/Debian: sudo apt-get install ${MISSING_TOOLS[*]}"
    echo "  Arch: sudo pacman -S ${MISSING_TOOLS[*]}"
    echo "  macOS: brew install ${MISSING_TOOLS[*]}"
fi

# Install additional cargo tools needed for Phase 5
echo
echo -e "${BLUE}🔧 Installing Development Tools${NC}"

CARGO_TOOLS=(
    "cargo-audit"
    "cargo-valgrind" 
    "cargo-tarpaulin"
    "cargo-expand"
    "cargo-outdated"
    "grcov"
)

for tool in "${CARGO_TOOLS[@]}"; do
    if ! cargo "$tool" --version &> /dev/null; then
        echo "Installing $tool..."
        cargo install "$tool" || warning "Failed to install $tool"
    else
        status "$tool already installed"
    fi
done

# Set up git hooks (if in a git repository)
if [ -d ".git" ]; then
    echo
    echo -e "${BLUE}🎣 Setting up Git Hooks${NC}"
    
    mkdir -p .git/hooks
    
    # Pre-commit hook for formatting and basic checks
    cat > .git/hooks/pre-commit << 'EOF'
#!/bin/bash
set -e

echo "🔍 Running pre-commit checks..."

# Check formatting
if ! cargo fmt -- --check; then
    echo "❌ Code formatting issues found. Run 'cargo fmt' to fix."
    exit 1
fi

# Run clippy
if ! cargo clippy --all-targets --all-features -- -D warnings; then
    echo "❌ Clippy warnings found. Please fix them."
    exit 1
fi

# Run quick tests
if ! cargo test --lib --bins; then
    echo "❌ Tests failed. Please fix them."
    exit 1
fi

echo "✅ Pre-commit checks passed!"
EOF
    
    chmod +x .git/hooks/pre-commit
    status "Git pre-commit hook installed"
fi

# Run initial project health check
echo
echo -e "${BLUE}🏥 Project Health Check${NC}"

echo "📊 Project Statistics:"
echo "  - Total Rust files: $(find src -name "*.rs" | wc -l)"
echo "  - Total lines of code: $(find src -name "*.rs" -exec wc -l {} \; | awk '{sum += $1} END {print sum}')"
echo "  - Cargo dependencies: $(grep -c "^[a-zA-Z]" Cargo.toml || echo "0")"

# Check if code compiles
echo
echo "🔧 Compilation Check:"
if cargo check --all-features; then
    status "Code compiles successfully"
else
    error "Compilation failed - fix errors before proceeding"
    exit 1
fi

# Run existing tests to establish baseline
echo
echo "🧪 Running Test Suite (establishing baseline):"
if cargo test --lib --bins --verbose; then
    status "All tests pass"
else
    warning "Some tests failed - this will be addressed in Phase 5"
fi

# Run security audit
echo
echo "🔒 Security Audit:"
if cargo audit; then
    status "No security vulnerabilities found"
else
    warning "Security vulnerabilities detected - address these in Priority 1"
fi

# Check for outdated dependencies
echo
echo "📦 Dependency Check:"
if cargo outdated --exit-code 1; then
    status "All dependencies are up to date"
else
    warning "Some dependencies are outdated - consider updating"
fi

# Run benchmarks to establish performance baseline
echo
echo "⚡ Performance Baseline:"
if [ -f "benches/compositor_benchmarks.rs" ]; then
    echo "Running benchmarks (this may take a few minutes)..."
    if cargo bench --bench compositor_benchmarks > benchmark_baseline.txt 2>&1; then
        status "Performance baseline established in benchmark_baseline.txt"
    else
        warning "Benchmark run failed - will establish baseline later"
    fi
else
    warning "Benchmark file not found - will be created in Priority 1"
fi

# Generate initial test coverage report
echo
echo "📈 Test Coverage Analysis:"
if command -v cargo-tarpaulin &> /dev/null; then
    echo "Generating coverage report..."
    if cargo tarpaulin --out Html --output-dir coverage/; then
        status "Coverage report generated in coverage/tarpaulin-report.html"
    else
        warning "Coverage analysis failed"
    fi
else
    warning "cargo-tarpaulin not installed - install it for coverage analysis"
fi

# Create Phase 5 development branch (if in git repo)
if [ -d ".git" ] && git rev-parse --git-dir > /dev/null 2>&1; then
    echo
    echo "🌱 Git Branch Setup:"
    
    current_branch=$(git branch --show-current)
    if [ "$current_branch" != "phase5-dev" ]; then
        if git show-ref --verify --quiet refs/heads/phase5-dev; then
            warning "phase5-dev branch already exists"
        else
            git checkout -b phase5-dev
            status "Created and switched to phase5-dev branch"
        fi
    else
        status "Already on phase5-dev branch"
    fi
fi

# Create development environment status file
cat > DEVELOPMENT_STATUS.md << 'EOF'
# Axiom Phase 5 Development Status

## Environment Setup
- [x] Rust toolchain installed
- [x] Development tools installed
- [x] Git hooks configured
- [x] Initial health check completed

## Priority 1: Core Stability & Testing
- [ ] Comprehensive test suite (target: 80% coverage)
- [ ] Integration tests for all major components
- [ ] Memory leak detection and fixes
- [ ] Performance regression prevention
- [ ] Error handling improvements

## Priority 2: Real Wayland Client Support
- [ ] Complete Smithay integration
- [ ] Protocol support for major applications
- [ ] Input event processing from Smithay
- [ ] Multi-output support

## Priority 3: Lazy UI Integration
- [ ] IPC robustness improvements
- [ ] Real-time performance monitoring
- [ ] AI optimization integration
- [ ] Usage pattern learning

## Priority 4: Distribution & Packaging
- [ ] Arch Linux AUR package
- [ ] Ubuntu/Debian .deb package
- [ ] CI/CD pipeline setup
- [ ] Release automation

## Current Metrics (Baseline)
- **Code Compilation**: ✅ Success
- **Test Pass Rate**: To be established
- **Security Issues**: To be audited
- **Performance Baseline**: To be established
- **Code Coverage**: To be measured

---
*Updated: $(date)*
EOF

echo
echo -e "${GREEN}🎉 Phase 5 Development Environment Ready!${NC}"
echo
echo "Next Steps:"
echo "1. Review PHASE5_ROADMAP.md for detailed development plan"
echo "2. Check DEVELOPMENT_STATUS.md for current progress"
echo "3. Start with Priority 1 tasks (testing infrastructure)"
echo "4. Run 'cargo test' to see current test status"
echo "5. Run 'cargo bench' to establish performance baselines"
echo
echo -e "${BLUE}Happy coding! 🦀${NC}"
