# Axiom Development Setup Guide

## Prerequisites for Real Wayland Compositor Development

### System Dependencies (Arch / CachyOS)

```bash
# Install Wayland development libraries
sudo pacman -S wayland wayland-protocols libxkbcommon mesa

# Install input and graphics libraries
sudo pacman -S libinput libudev0-shim libdrm

# Install development tools
sudo pacman -S pkg-config cmake ninja

# Install optional Wayland utilities for testing
sudo pacman -S weston wayland-utils
```

Plenty of these (`libdrm`, `libinput`, `wayland-protocols`) are pulled transitively by Smithay 0.7 through its feature set. The explicit install is mostly useful when you want the `weston` test client binary.

### What Cargo Provides

Smithay 0.7 already supplies:
- `calloop` event loop internals
- `libinput` re-exports
- `drm` / `gbm` abstractions for the DRM backend (the `backend_drm` feature)
- `wgpu`-friendly `GlesRenderer`

So the first-party `Cargo.toml` deps (`calloop`, `drm`, `gbm`, `input`, `xkbcommon`) are mostly safe to trim in a future cleanup pass — they are either unused in `src/` or re-exported by Smithay.

## Development Environment Setup

### 1. Wayland Testing Setup (Nested Session)

```bash
# Terminal 1: Start a parent Wayland compositor (e.g. weston)
weston --width=1920 --height=1080 &

# Terminal 2: Build and run Axiom under the nested session
cargo run -- --debug --windowed
```

### 2. Code Layout

The compositor is structured around one event loop (`AxiomCompositor::run`) that drives a Smithay 0.7 backend (`AxiomSmithayBackendReal`) plus Tokio-flavoured helpers (IPC, XWayland manager). The key files:

```
src/lib.rs                       — re-exports + BuildInfo
src/main.rs                      — CLI + subsystem wiring
src/compositor.rs                — event loop, tick, render orchestration
src/backend/mod.rs               — Smithay 0.7 backend (new + new_for_test)
src/backend/xwm.rs               — x11rb window-manager side of XWayland
src/renderer/mod.rs              — wgpu 0.19 surface + texture management
src/workspace/mod.rs             — niri-style scrollable tapes
src/effects/{mod,animations,blur,shadow,shaders}.rs — wgpu blur/shadow + spring physics
src/window/mod.rs                — window manager + tiling layout
src/input/mod.rs                 — keybindings + scroll/gesture actions
src/ipc/mod.rs                   — Tokio Unix-domain-socket IPC
src/xwayland/mod.rs              — `Xwayland` subprocess
src/decoration.rs                — server-side decorations
src/config/mod.rs                — TOML config + validation
```

### 3. Testing

```bash
# Run unit tests (CI runs with --test-threads=1 to avoid WGPU races)
cargo test --lib -- --test-threads=1

# Run integration tests
cargo test --test integration_tests -- --test-threads=1

# Property tests run as part of `cargo test --lib`
cargo test --lib workspace
cargo test --lib config
```

### 4. Smoke Test with `weston-terminal`

The real-compositor path expects a Wayland client to connect via the socket exposed in `$WAYLAND_DISPLAY`. To verify in a winit window:

```bash
# Terminal 1
cargo run -- --windowed

# Terminal 2 (with WAYLAND_DISPLAY pointing at Axiom)
WAYLAND_DISPLAY=wayland-axiom-$$ weston-terminal
```

If weston-terminal opens a window inside the winit window, the WLGPU + SHM upload path is wired correctly.

### 5. Common Issues

#### "No Wayland display found"
```bash
export WAYLAND_DISPLAY=wayland-0  # or whatever Axiom listens on (check logs)
```

#### WGPU adapter unavailable on CI
The CI workflow uses `cargo test --lib -- --test-threads=1`; if you see wgpu initialization failures locally, run with `WGPU_BACKEND=vulkan` or `WGPU_BACKEND=gl`.

## Performance Monitoring

```bash
# Watch compositor memory
watch -n 1 'ps aux | grep axiom'

# Frame timing via Perfetto / tokio-console (when enabled)
RUST_LOG=trace cargo run -- --windowed
```

## Next Steps

1. Implement `WorkspaceCommand` + `EffectsControl` IPC handlers in `src/ipc/mod.rs`.
2. Wire `effects/blur.rs` and `effects/shadow.rs` into the live Smithay render path (currently exercised only by tests).
3. Implement `wl_data_device` (clipboard) + XDG popup handling for full X11 compatibility.
