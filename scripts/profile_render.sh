#!/bin/bash
# Axiom Compositor — Render path profiler
#
# Starts the compositor in nested (winit) mode and collects perf data
# for the render path using Linux perf(1) and (optionally) FlameGraph scripts.
#
# Usage:
#   ./scripts/profile_render.sh [--help] [--duration N] [--flamegraph] [--output DIR]
#
# Prerequisites:
#   - perf  (linux-tools, usually linux-tools-$(uname -r))
#   - cargo (rustup)
#   - [optional] FlameGraph scripts (stackcollapse-perf.pl, flamegraph.pl)
#     in PATH or at $FLAMEGRAPH_DIR

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_DIR="/tmp/axiom-profile-$$"
DURATION=30
DO_FLAMEGRAPH=false
FLAMEGRAPH_DIR="${FLAMEGRAPH_DIR:-}"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Profile the Axiom compositor render path using perf.

Options:
  --help           Show this help message and exit.
  --duration N     Profile for N seconds (default: 30).
  --flamegraph     Generate a FlameGraph SVG in addition to the raw perf data.
  --output DIR     Write output to DIR (default: /tmp/axiom-profile-*).

Environment:
  FLAMEGRAPH_DIR   Directory containing stackcollapse-perf.pl and flamegraph.pl
                   (used only with --flamegraph).

Output:
  Raw perf data:    \$output_dir/perf.data
  Perf report:      \$output_dir/perf.report
  FlameGraph SVG:   \$output_dir/flamegraph.svg  (with --flamegraph)

Examples:
  # Profile for 15 seconds
  ./scripts/profile_render.sh --duration 15

  # Profile with flamegraph
  ./scripts/profile_render.sh --flamegraph

  # Custom output directory
  ./scripts/profile_render.sh --output /tmp/my-profile
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --help) usage ;;
        --duration) DURATION="$2"; shift 2 ;;
        --flamegraph) DO_FLAMEGRAPH=true; shift ;;
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

# Check prerequisites
if ! command -v perf &>/dev/null; then
    echo "Error: perf(1) not found. Install linux-tools (e.g. apt install linux-tools-common)."
    exit 1
fi

if ! command -v cargo &>/dev/null; then
    echo "Error: cargo not found."
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
cd "$PROJECT_DIR"

echo "=== Axiom Render Profile ==="
echo "Duration:   ${DURATION}s"
echo "Output dir: $OUTPUT_DIR"
echo ""

# Build release for more representative numbers
echo "→ Building release binary..."
cargo build --release -q

# Start the composinator in the background, capture its PID
echo "→ Starting compositor..."
cargo run --release &
COMPOSITOR_PID=$!
sleep 2

# Verify it's still alive
if ! kill -0 "$COMPOSITOR_PID" 2>/dev/null; then
    echo "Error: compositor failed to start."
    exit 1
fi

# Profile the compositor with perf
echo "→ Profiling render path for ${DURATION}s (perf record)..."
perf record -g -p "$COMPOSITOR_PID" -o "$OUTPUT_DIR/perf.data" \
    -- sleep "$DURATION" || true

# Stop the compositor
echo "→ Stopping compositor..."
kill "$COMPOSITOR_PID" 2>/dev/null || true
wait "$COMPOSITOR_PID" 2>/dev/null || true

# Generate perf report
echo "→ Generating perf report..."
perf report -i "$OUTPUT_DIR/perf.data" --stdio \
    > "$OUTPUT_DIR/perf.report" 2>/dev/null

echo "Raw report: $OUTPUT_DIR/perf.report"

# Optionally generate flamegraph
if $DO_FLAMEGRAPH; then
    STACKCOLLAPSE=""
    FLAMEGRAPH_PL=""

    # Search PATH and FLAMEGRAPH_DIR
    if command -v stackcollapse-perf.pl &>/dev/null; then
        STACKCOLLAPSE="$(command -v stackcollapse-perf.pl)"
    elif [[ -n "$FLAMEGRAPH_DIR" && -x "$FLAMEGRAPH_DIR/stackcollapse-perf.pl" ]]; then
        STACKCOLLAPSE="$FLAMEGRAPH_DIR/stackcollapse-perf.pl"
    fi

    if command -v flamegraph.pl &>/dev/null; then
        FLAMEGRAPH_PL="$(command -v flamegraph.pl)"
    elif [[ -n "$FLAMEGRAPH_DIR" && -x "$FLAMEGRAPH_DIR/flamegraph.pl" ]]; then
        FLAMEGRAPH_PL="$FLAMEGRAPH_DIR/flamegraph.pl"
    fi

    if [[ -z "$STACKCOLLAPSE" || -z "$FLAMEGRAPH_PL" ]]; then
        echo "Warning: FlameGraph scripts not found. Install from https://github.com/brendangregg/FlameGraph"
        echo "  or set FLAMEGRAPH_DIR to the cloned directory."
    else
        echo "→ Generating flamegraph..."
        "$STACKCOLLAPSE" < "$OUTPUT_DIR/perf.data" > "$OUTPUT_DIR/out.folded" 2>/dev/null
        "$FLAMEGRAPH_PL" "$OUTPUT_DIR/out.folded" > "$OUTPUT_DIR/flamegraph.svg"
        echo "FlameGraph: $OUTPUT_DIR/flamegraph.svg"
    fi
fi

echo ""
echo "=== Done ==="
