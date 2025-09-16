#!/usr/bin/env bash
set -euo pipefail

# Axiom Dev Setup Script
# Usage: ./scripts/dev_setup.sh [arch|ubuntu]

OS=${1:-}

if [[ -z "$OS" ]]; then
  if [[ -f /etc/arch-release ]] || grep -qi 'arch' /etc/os-release; then
    OS=arch
  elif grep -qi 'ubuntu\|debian' /etc/os-release; then
    OS=ubuntu
  else
    echo "Please specify your distro: ./scripts/dev_setup.sh [arch|ubuntu]" >&2
    exit 1
  fi
fi

if [[ $OS == "arch" ]]; then
  echo "Installing dependencies for Arch..."
  sudo pacman -Syu --noconfirm \
    base-devel \
    rust \
    cargo \
    wayland \
    wayland-protocols \
    libxkbcommon \
    mesa \
    libdrm \
    libinput \
    systemd \
    dbus \
    pkgconf \
    python
elif [[ $OS == "ubuntu" ]]; then
  echo "Installing dependencies for Ubuntu/Debian..."
  sudo apt-get update
  sudo apt-get install -y \
    build-essential \
    curl \
    pkg-config \
    libwayland-dev \
    wayland-protocols \
    libxkbcommon-dev \
    libegl1-mesa-dev \
    libdrm-dev \
    libgbm-dev \
    libinput-dev \
    libsystemd-dev \
    libdbus-1-dev \
    python3
else
  echo "Unsupported distro: $OS" >&2
  exit 1
fi

echo "Installing rustup and Rust toolchain (if needed)..."
if ! command -v rustup >/dev/null 2>&1; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
  source "$HOME/.cargo/env"
fi
rustup toolchain install stable
rustup default stable

# Optional: NVIDIA NVML feature dependency (libnvidia-ml)
echo "If using --features gpu-nvml, ensure NVIDIA drivers and libnvidia-ml are installed."

echo "Setup complete. You can now build with: cargo build"