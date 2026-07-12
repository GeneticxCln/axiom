# Axiom 🚀

**A hybrid Wayland compositor prototype combining scrollable workspaces with visual effects.**

<div align="center">

[![Rust Version](https://img.shields.io/badge/rust-1.70%2B-orange)](#)
[![License](https://img.shields.io/badge/license-GPLv3-blue)](#)
[![Status](https://img.shields.io/badge/status-alpha-yellow)](#)

**Where productivity meets beauty.**

</div>

## Current Status

Axiom is currently an **alpha-stage compositor prototype**.

What is true today:
- The project has a real Smithay-based compositor backend.
- The **nested / windowed (`--windowed`) path is the most complete development target**.
- Scrollable workspace logic, configuration parsing, IPC, and renderer/effects infrastructure are implemented.
- The standalone DRM/KMS path now has an early compositor output path, but is **not yet release-ready**.

What is not true yet:
- Axiom is **not** in “production polish”.
- Packaging and release assets are still incomplete.
- Multi-monitor, fractional scaling, XWayland compatibility, and the render pipeline still need more work.

For the detailed project status and roadmap, see [MASTER_DEVELOPMENT_PLAN.md](MASTER_DEVELOPMENT_PLAN.md).

## Vision

Axiom explores a compositor UX that combines:
- **Scrollable workspaces** inspired by niri
- **Visual effects** inspired by Hyprland
- **Structured IPC** for external tooling / optimization clients

## What Works Today

### Core logic
- Scrollable workspace/tape model
- Window registry and basic lifecycle management
- TOML configuration loading/validation
- JSON IPC over Unix sockets
- Animation/effects state management

### Development compositor path
- Smithay-based Wayland socket
- XDG toplevel/popup handling
- Nested development mode via `--windowed`
- Input routing and compositor shortcuts
- WGPU renderer plus transitional GL presentation path
- Explicit **client-side decoration negotiation** in the live runtime path until visible SSD rendering is integrated

## What Is Still Incomplete

- Fully finished standalone DRM/KMS compositor path
- Unified render/present architecture
- Robust multi-monitor behavior
- Fractional scaling / HiDPI polish
- Visible server-side decoration rendering in the live compositor output path (current live policy remains CSD-first)
- Complete XWayland clipboard and compatibility flow
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
│   ├── effects/
│   ├── renderer/
│   ├── window/
│   ├── input/
│   ├── ipc/
│   └── xwayland/
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
cargo run -- --windowed --debug
```

The nested/windowed path is the recommended way to evaluate Axiom right now.

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

### Currently supported optimization surface

Persistent config mutations via `OptimizeConfig` / `SetConfig` are currently limited to:
- `effects.blur.radius`
- `effects.animations.duration`
- `workspace.scroll_speed`

Runtime effects tuning via `EffectsControl` currently supports:
- `enabled`
- `blur_radius`
- `animation_speed`

Helper clients in this repository now restrict themselves to those supported keys/fields.

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
- [DRM Hardware Validation](docs/dev/DRM_HARDWARE_VALIDATION.md)
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
- [Hyprland](https://github.com/hyprwm/Hyprland)
- [Smithay](https://github.com/Smithay/smithay)

## License

GPL-3.0
