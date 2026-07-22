# Axiom 🚀

**A winit-only Wayland compositor with scrollable workspaces, built on Smithay 0.7.**

<div align="center">

[![Rust Version](https://img.shields.io/badge/rust-1.70%2B-orange)](#)
[![License](https://img.shields.io/badge/license-GPLv3-blue)](#)
[![Status](https://img.shields.io/badge/status-alpha-yellow)](#)

</div>

## Quick Start

```bash
git clone https://github.com/GeneticxCln/axiom.git
cd axiom
cargo build
cargo run -- --debug
```

This starts Axiom nested inside your existing desktop session. See [Running](docs/user/RUNNING.md) for smoke-test instructions.

## Current Status

Axiom is an **alpha-stage Wayland compositor** with a single, fully working
backend: **winit** (nested/windowed). It presents real client window content
plus server-side decoration titlebars through a Smithay 0.7 GLES renderer.

- `cargo build` and `cargo test` are clean (0 warnings).
- Scrollable workspace engine, TOML configuration, and JSON IPC are implemented.
- Multi-monitor and standalone session support are not yet available.

## Features

- **Scrollable workspaces** — niri-inspired horizontal tape model with per-column tiling, gaps, and eased scroll/momentum
- **Server-side decorations** — titlebars with close/maximize buttons rendered via GLES
- **JSON IPC** — Unix-socket control plane for external tooling (workspace commands, clipboard push)
- **Client-initiated drag-and-drop** — DnD sessions tracked, icon surface rendered
- **Touch input** — down/motion/up/cancel with touch-based window move/resize
- **Configurable keybindings** — TOML-defined modifier+key bindings for all common actions

## What Is Still Incomplete

- Single-output only (winit, no multi-monitor support)
- Fractional scaling / HiDPI polish
- Release-ready packaging — structure exists, needs final validation
- Server-initiated drag-and-drop is a stub
- Touch gesture support (tap-to-click, multi-finger gestures)

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
└── scripts/
```

## Build & Test

```bash
cargo build                    # debug build
cargo build --release          # optimized binary
cargo test                     # unit + integration tests
cargo test --all-targets       # includes benches
xvfb-run -a cargo test         # run all tests (including xvfb-required)
```

See [Build Notes](docs/dev/BUILD.md) for details.

## Configuration

Axiom uses a single TOML config file at `~/.config/axiom/axiom.toml`.

A default example is shipped in `config/axiom.toml`.

See [Configuration](docs/user/CONFIGURATION.md) for details.

## IPC / Lazy UI Integration

Axiom exposes a Unix socket for external clients.

**Socket paths:**
- Preferred: `$XDG_RUNTIME_DIR/axiom/axiom.sock`
- Fallback: `/tmp/axiom-<pid>/axiom-lazy-ui.sock`

All 10 `WorkspaceCommand` actions and `SetClipboard` are wired end-to-end.

```bash
echo '{"type":"HealthCheck"}' | nc -U "$XDG_RUNTIME_DIR/axiom/axiom.sock"
```

## Documentation

### User docs
- [Installation](docs/user/INSTALL.md)
- [Running](docs/user/RUNNING.md)
- [Configuration](docs/user/CONFIGURATION.md)
- [Known Limitations](docs/user/LIMITATIONS.md)

### Developer docs
- [Architecture Overview](src/lib.rs) (module docs)
- [Build Notes](docs/dev/BUILD.md)
- [Render Architecture](docs/dev/RENDER_ARCHITECTURE.md)
- [Backend Selection](docs/dev/BACKEND_SELECTION.md)
- [Config Support Matrix](docs/dev/CONFIG_SUPPORT.md)
- [Contributing](docs/dev/CONTRIBUTING.md)
- [Setup](docs/dev/SETUP.md)
- [Security](docs/dev/SECURITY.md)
- [Release Process](docs/dev/RELEASE_PROCESS.md)

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

## Inspiration

- [niri](https://github.com/YaLTeR/niri)
- [Smithay](https://github.com/Smithay/smithay)

## License

GPL-3.0
