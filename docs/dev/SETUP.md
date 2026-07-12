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

# Install optional Wayland/X11 utilities for testing
sudo pacman -S weston wayland-utils xorg-xwayland xorg-xdpyinfo xorg-xmessage
```

Plenty of these (`libdrm`, `libinput`, `wayland-protocols`) are pulled transitively by Smithay 0.7 through its feature set. The explicit install is mostly useful when you want the `weston` test client binary or the local XWayland validation wrapper.

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
src/backend/mod.rs               — Smithay 0.7 backend orchestration / state machine
src/backend/render_bridge.rs     — nested WGPU→CPU→GL presentation bridge helpers
src/backend/clipboard_bridge.rs  — Wayland/X11 clipboard bridge helpers
src/backend/xwm.rs               — x11rb window-manager side of XWayland
src/renderer/mod.rs              — wgpu 0.19 surface + texture management
src/workspace/mod.rs             — niri-style scrollable tapes
src/effects/{mod,animations,blur,shadow,shaders}.rs — wgpu blur/shadow + spring physics
src/window/mod.rs                — window manager + tiling layout
src/input/mod.rs                 — keybindings + scroll/gesture actions
src/ipc/mod.rs                   — Tokio Unix-domain-socket IPC
src/xwayland/mod.rs              — `Xwayland` subprocess
src/decoration.rs                — internal decoration state / future visible SSD path
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

The nested compositor path exposes its own Wayland socket. Start Axiom first, note the socket name from the logs, then launch a simple client against that socket.

```bash
# Terminal 1
cargo run -- --windowed --debug

# Terminal 2 (replace the socket name with the one printed by Axiom)
WAYLAND_DISPLAY=wayland-axiom-12345 weston-terminal
```

If `weston-terminal` opens inside the Axiom window, the nested Wayland client path is working.

For an automated version of the same flow, use the repository smoke script:

```bash
cargo build
xvfb-run -a ./scripts/nested_smoke_test.sh ./target/debug/axiom
```

That script is the same path used by CI for the current real-client nested smoke check.

### 5. XWayland end-to-end smoke test

To validate a real X11 client mapping through Axiom's XWayland/XWM path:

```bash
bash ./scripts/check_xwayland.sh all ./target/debug/axiom
```

That covers:
- XWayland lifecycle
- real X11 client connectivity
- X11 metadata checks
- compositor-side XWM map-event wiring
- end-to-end X11-in-Axiom smoke

If you want to run only the full compositor/X11 smoke path directly:

```bash
bash ./scripts/xwayland_end_to_end_smoke.sh ./target/debug/axiom
```

### 6. Common Issues

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

## Current Development Priorities

1. Stabilize the nested/windowed compositor path as the primary alpha target.
2. Reduce render-path complexity around the documented WGPU-first presentation architecture.
3. Finish the standalone DRM/KMS compositor output path.
4. Harden XWayland clipboard/metadata behavior.
5. Expand real-client smoke coverage and release assets.
