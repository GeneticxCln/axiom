#!/usr/bin/env bash
set -euo pipefail

# Dev helper: build, run in windowed mode, ensure socket dir, copy default config, and tail logs

# Build
cargo build

# Config
CONFIG_DIR="${HOME}/.config/axiom"
mkdir -p "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/axiom.toml" ]; then
  cp config/axiom.toml "$CONFIG_DIR/axiom.toml"
  echo "Created default config at $CONFIG_DIR/axiom.toml"
fi

# Ensure runtime socket dir
RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp}"
SOCK_DIR="$RUNTIME_DIR/axiom"
mkdir -p "$SOCK_DIR"
chmod 700 "$SOCK_DIR" || true

# Run in background with debug
./target/debug/axiom --debug --windowed &
PID=$!
echo "Axiom started (pid=$PID). Socket: $SOCK_DIR/axiom.sock"

echo "Run: python3 test_ipc.py"
wait $PID
