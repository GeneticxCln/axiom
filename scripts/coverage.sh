#!/usr/bin/env bash
# Run code coverage via cargo-tarpaulin
# Usage: ./scripts/coverage.sh [--xml|--html|--lcov]

set -euo pipefail

FORMAT="${1:-html}"
OUT_DIR="target/coverage"

case "$FORMAT" in
  --xml)  OUT_ARG="--out Xml";   OUT_FILE="$OUT_DIR/cobertura.xml" ;;
  --html) OUT_ARG="--out Html";  OUT_FILE="$OUT_DIR/index.html" ;;
  --lcov) OUT_ARG="--out Lcov";  OUT_FILE="$OUT_DIR/lcov.info" ;;
  *)      echo "Usage: $0 [--xml|--html|--lcov]"; exit 1 ;;
esac

mkdir -p "$OUT_DIR"

echo "Running coverage (lib only — integration tests need a display server)..."
cargo tarpaulin \
  --lib \
  --exclude-files "benches/*" "tests/*" "examples/*" \
  --output-dir "$OUT_DIR" \
  $OUT_ARG \
  --skip-clean

echo "Coverage report: $OUT_FILE"
