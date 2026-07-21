# Axiom Development Setup Guide

## Prerequisites for Wayland Compositor Development

### System Dependencies (Ubuntu / Debian)

```bash
sudo apt install \
  build-essential \
  cargo \
  libwayland-dev \
  libxkbcommon-dev \
  wayland-protocols \
  pkg-config
```

### Arch / CachyOS

```bash
sudo pacman -S \
  cargo \
  mesa \
  pkgconf \
  rust \
  wayland \
  wayland-protocols \
  libxkbcommon
```

### What Cargo Provides

Smithay 0.7 supplies everything needed:
- `wayland-server` + `wayland-protocols`
- `xkbcommon` bindings
- GLES renderer backend

## Development Environment Setup

### 1. Nested Session (recommended)

```bash
# Terminal 1: Start the compositor
cargo run -- --windowed --debug

# Terminal 2: Launch a client against Axiom's socket
WAYLAND_DISPLAY=wayland-axiom-$(pidof axiom) weston-terminal
```

### 2. Code Layout

```
src/lib.rs                — re-exports + BuildInfo
src/main.rs               — CLI + subsystem wiring
src/compositor.rs         — event loop, tick orchestration
src/backend/mod.rs        — Smithay 0.7 backend orchestration
src/backend/render.rs     — GLES render path (shared under split)
src/backend/input.rs      — input/event handling
src/backend/clipboard.rs  — Wayland clipboard handling
src/workspace/mod.rs      — scrollable workspaces (niri-style)
src/window/mod.rs         — window manager + tiling
src/input/mod.rs          — keybindings + actions
src/ipc/mod.rs            — Unix-socket JSON IPC
src/decoration.rs         — server-side decoration geometry
src/config/mod.rs         — TOML config + validation
```

### 3. Testing

```bash
# All tests (unit, integration, smoke)
cargo test

# Run headless integration tests only
cargo test --test integration_tests

# Real-client smoke under xvfb (requires xvfb)
xvfb-run -a cargo test --test real_client_smoke
```

### 4. Common Issues

**"No Wayland display found"**
```bash
export WAYLAND_DISPLAY=wayland-0  # or Axiom's printed socket name
```

**"could not find system fonts"**
SSD title text renders without fonts; titlebars/buttons still appear.