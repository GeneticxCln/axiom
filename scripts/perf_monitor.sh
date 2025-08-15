#!/bin/bash

# Phase 5: Simple Performance Monitoring Script for Axiom

set -euo pipefail

echo "🔍 Axiom Performance Monitor"
echo "============================"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

# Run tests and measure time
echo -e "${YELLOW}Running test suite...${NC}"
TEST_START=$(date +%s%N)
cargo test --quiet 2>&1 | tail -5
TEST_END=$(date +%s%N)
TEST_TIME=$((($TEST_END - $TEST_START)/1000000))
echo -e "${GREEN}✅ Test suite completed in ${TEST_TIME}ms${NC}"

# Check binary size
echo -e "\n${YELLOW}Checking binary sizes...${NC}"
cargo build --release 2>/dev/null
RELEASE_SIZE=$(du -h target/release/axiom 2>/dev/null | cut -f1)
echo -e "${GREEN}Release binary size: ${RELEASE_SIZE}${NC}"

# Count lines of code
echo -e "\n${YELLOW}Code statistics...${NC}"
RUST_LINES=$(find src -name "*.rs" -exec wc -l {} + | tail -1 | awk '{print $1}')
TEST_LINES=$(find tests -name "*.rs" -exec wc -l {} + 2>/dev/null | tail -1 | awk '{print $1}')
echo -e "${GREEN}Rust code: ${RUST_LINES} lines${NC}"
echo -e "${GREEN}Test code: ${TEST_LINES} lines${NC}"

# Memory usage estimate (rough)
echo -e "\n${YELLOW}Running memory check...${NC}"
if command -v /usr/bin/time &> /dev/null; then
    /usr/bin/time -f "Peak memory: %M KB" cargo check --quiet 2>&1 | grep "Peak memory" || echo "Memory check unavailable"
fi

echo -e "\n${GREEN}✅ Performance monitoring complete${NC}"
