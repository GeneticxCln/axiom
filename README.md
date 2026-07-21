# Axiom 🚀

**A Wayland compositor with scrollable workspaces, built on Smithay 0.7.**

<div align="center">

[![Rust Version](https://img.shields.io/badge/rust-1.70%2B-orange)](#)
[![License](https://img.shields.io/badge/license-GPLv3-blue)](#)
[![Status](https://img.shields.io/badge/status-alpha-yellow)](#)

**Where productivity meets a calm, scrollable desktop.**

</div>

## Current Status

Axiom is an **alpha-stage Wayland compositor** with a single, fully working
backend: **winit** (nested/windowed). It presents real client window content
plus server-side decoration titlebars through a Smithay 0.7 GLES renderer.

What is true today:
- Real Smithay 0.7 compositor backend (winit-only; no DRM/KMS, no xwayland).
- GLES rendering through the winit window — real client pixels are shown.
- Scrollable workspace engine, configuration parsing, and JSON IPC are implemented.
- `cargo build` and `cargo test` are green (0 warnings).

What is not true yet:
- Axiom is **not** in "production polish".
- Packaging and release assets are still incomplete.
- Multi-monitor, fractional scaling, and touch input still need work.
- Full drag-and-drop data transfer is not yet implemented; clipboard works in
  both directions (compositor→client and Wayland-client→compositor capture).

## Vision

Axiom explores a compositor UX that combines:
- **Scrollable workspaces** inspired by niri
- **Structured IPC** for external tooling / optimization clients

## What Works Today

### Core logic
- Scrollable workspace/tape model (`ScrollableWorkspaces` / `WorkspaceTape`)
- Window registry and basic lifecycle management
- TOML configuration loading/validation
- JSON IPC over Unix sockets
- Eased scroll/momentum with per-column tiling and gaps

### Compositor path (winit)
- Smithay-based Wayland socket
- XDG toplevel/popup handling
- Input routing and compositor shortcuts
- GLES rendering via the winit backend: each client's committed `wl_buffer`
  is imported into a `GlesTexture` and drawn (plus a solid backdrop and
  server-side decoration titlebars/buttons) via `SolidColorRenderElement` /
  `TextureRenderElement`, then submitted
- Live resize via `WinitEvent::Resized` updates the workspace viewport + output mode

## What Is Still Incomplete

- Standalone DRM/KMS compositor path (removed — winit is the only backend)
- Robust multi-monitor behavior
- Fractional scaling / HiDPI polish
- Touch input
- Full drag-and-drop data transfer
- All 8 resize edges are wired (Left/Right/Top/Bottom + 4 corners)
- Release-ready packaging and session assets

## Repository Layout

```text
axiom/
├── src/
│   ├── main.rs
│   ├── compositor.rs
│   ├── backend/
│   ├── config/
│   ├── workspace/
│   ├── window/
│   ├── input/
│   └── ipc/
├── docs/
├── config/axiom.toml
├── test_ipc.py
└── MASTER_DEVELOPMENT_PLAN.md
```

## Quick Start

### Build
```bash
cargo build
```

### Run the recommended alpha target
```bash
cargo run -- --debug
```

The winit/nested path is the recommended way to evaluate Axiom right now.

Automated nested smoke test (uses a real Wayland client such as `weston-terminal`):

```bash
cargo build
xvfb-run -a ./scripts/nested_smoke_test.sh ./target/debug/axiom
```

## Configuration

Axiom uses a single TOML config file at:

```text
~/.config/axiom/axiom.toml
```

A default example is shipped in:

```text
config/axiom.toml
```

See [docs/user/CONFIGURATION.md](docs/user/CONFIGURATION.md) for details.

## IPC / Lazy UI Integration

Axiom exposes a Unix socket for external clients.

### Socket paths
- Preferred: `$XDG_RUNTIME_DIR/axiom/axiom.sock`
- Fallback (when `XDG_RUNTIME_DIR` is unavailable): `/tmp/axiom-<pid>/axiom-lazy-ui.sock`

Because the fallback path is process-specific, helper clients in this repo support manual override via `AXIOM_SOCKET_PATH` and also scan the fallback pattern automatically.

### Workspace commands

All 10 `WorkspaceCommand` actions are wired end-to-end and enforced against a
whitelist (`KNOWN_WORKSPACE_ACTIONS`) in production. Unknown actions are
rejected with an `unknown_action` ACK.

### Effects control

`LazyUIMessage::EffectsControl` is accepted by the IPC layer, but effects are
no-ops — the effects module was removed. The config `effects` section is
retained as data only.

### Try the test client
```bash
python3 test_ipc.py
```

## Documentation

### User docs
- [Installation](docs/user/INSTALL.md)
- [Running](docs/user/RUNNING.md)
- [Configuration](docs/user/CONFIGURATION.md)
- [Known Limitations](docs/user/LIMITATIONS.md)

### Developer docs
- [Backend Selection](docs/dev/BACKEND_SELECTION.md)
- [Render Architecture](docs/dev/RENDER_ARCHITECTURE.md)
- [Config Support Matrix](docs/dev/CONFIG_SUPPORT.md)
- [Build Notes](docs/dev/BUILD.md)
- [Release Checklist](docs/dev/RELEASE_CHECKLIST.md)
- [Release Process](docs/dev/RELEASE_PROCESS.md)
- [Release Notes Template](docs/dev/RELEASE_NOTES_TEMPLATE.md)
- [Contributing](docs/dev/CONTRIBUTING.md)
- [Setup](docs/dev/SETUP.md)

## Contributing

Contributions are welcome, but please treat the project as an active alpha.

See [docs/dev/CONTRIBUTING.md](docs/dev/CONTRIBUTING.md).

## Inspiration

- [niri](https://github.com/YaLTeR/niri)
- [Smithay](https://github.com/Smithay/smithay)

## License

GPL-3.0
