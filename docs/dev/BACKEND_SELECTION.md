# Backend Selection

Axiom supports multiple backend implementations for development and testing.

## Available Backends

### 1. Real Compositor (Default / Recommended)
The canonical backend based on Smithay 0.7. It supports real Wayland clients, input handling, and protocol extensions.

-   **Source**: `src/backend/mod.rs` (with `src/backend/xwm.rs` for XWayland plumbing)
-   **Build**: `cargo build` (no special features required; Smithay 0.7 is the default backend)
-   **Capabilities**:
    -   Client connection handling
    -   Window management (XDG shell)
    -   SHM buffer upload + GLES 2.0 textured-quad rendering
    -   Input processing (via `InputManager` + xkbcommon keysyms)
    -   Data device / selection plumbing
    -   Winit-based windowed mode (`--windowed`)

### 2. Winit Backend (Windowed Mode)
Runs the compositor nested inside another Wayland/X11 session using `winit`. Useful for rapid iteration without switching TTYs.

-   **Flag**: `--windowed` (or `--backend=winit`)
-   **Usage**: `cargo run -- --windowed`

### 3. DRM / KMS Session-Compositor (Probe + Scaffolding)
The production path that drives the GPU directly via DRM/KMS, libinput, and udev hotplug. The `BackendKind` enum and `from_config_str` parser are implemented in [`src/backend/drm.rs`](../../src/backend/drm.rs). The DRM device probe checks `/dev/dri/card*` for availability. The full `LibSeatSession` + calloop integration is deferred to a follow-up PR.

-   **Flag**: `--backend=drm`
-   **Status**: `BackendKind::from_config_str` maps `"drm"`, `"kms"`, `"session"`, `"tty"` to `BackendKind::Drm`. `DrmBackend::new()` probes DRM device nodes. `initialize_drm()` logs intent and registers a keyboard seat. The event loop tick (`run_one_cycle_drm`) is a no-op awaiting calloop wiring. The compositor binds the Wayland socket but no KMS modesetting or page-flip occurs until calloop lands.

### 4. Headless Test Backend
A test-only backend that skips Wayland socket bind and display creation, so the CI can construct a compositor (`AxiomCompositor::new_for_test` + `AxiomSmithayBackendReal::new_for_test`) without real system resources. Used by the 79 unit tests in `cargo test --lib`.

### 5. Legacy / Experimental
The codebase no longer ships experimental backends in `src/experimental/`. Old Smithay 0.3 scaffolding was migrated to `src/backend/` under the `experimental-smithay` feature flag (currently a no-op marker; the real Smithay 0.7 path is always-on).

## CLI Flag: `--backend`

| Value     | Effect                                                     |
|-----------|------------------------------------------------------------|
| `winit`   | Winit nested-session windowed mode (default, dev-friendly).|
| `drm`     | Stub DRM/KMS session-compositor (architecture wired; calloop follow-up required).|
| `noop`    | Headless no-op backend (used by `cargo test --lib`).       |

Aliases accepted via TOML (`backend.kind`) and CLI: `windowed`/`dev` → winit, `kms`/`session`/`tty` → drm, `test`/`headless` → noop. Unknown values fall back to `winit` with a warning so a typo never bricks the compositor.

## Feature Flags (`Cargo.toml`)

| Flag                | Effect                                                                              |
|---------------------|--------------------------------------------------------------------------------------|
| `default`           | Empty — production build; pulls in Smithay 0.7 + wgpu 0.19.                         |
| `real-compositor`   | Marker flag (empty feature). Reserved for the live-compositor path.                  |
| `wgpu-present`      | Marker flag; future use for `--wgpu` integration of `AxiomRenderer`.                |
| `demo`              | Enables internal demo modes (`--demo`, `--effects-demo`).                            |
| `examples`          | Builds the `metrics_client` example.                                                 |
| `backend_session`   | Stub feature (mirrors smithay's `backend_session`) — see source comment.             |
| `backend_udev`      | Stub feature (mirrors smithay's `backend_udev`).                                     |
| `backend_drm`       | Stub feature (mirrors smithay's `backend_drm`).                                      |
| `backend_libinput`  | Stub feature (mirrors smithay's `backend_libinput`).                                 |
| `experimental-smithay` | Marker flag; kept for ABI compatibility with the legacy `src/experimental/` path. |

## Picking a Backend at Runtime

The CLI flag `--backend=<winit|drm|noop>` overrides any TOML value at the `config.backend.kind` slot. The resulting kind is then derived in `AxiomSmithayBackendReal::new` via [`BackendKind::from_config_str`](../../src/backend/drm.rs), and `initialize()` dispatches to either `initialize_winit()` or `initialize_drm()` (both implemented; DRM is a stub awaiting calloop). For CI, no special flag is needed — `cargo test --lib` exercises every subsystem through the `Noop` path.

## Minimized Feature Surface (Scope Decision)

Two features are deliberately *minimized* to keep the implementation surface focused and avoid digging deeper into Wayland protocol integration this milestone. Both are gated behind kill-switches in `[features]` (see [`FeaturesConfig`](../../src/config/mod.rs)) that **default to `false`**:

1. **Minimize / iconify** — Wayland has no standard minimize protocol. Supporting it well would require a compositor-internal iconified-window list and synthetic-surface round-tripping. With `[features].enable_minimize = false` (the default), `DecorationManager` will not draw a minimize button on the titlebar and `handle_button_press` will never return `DecorationAction::Minimize`.
2. **`xdg-decoration-v1` (SSD/CSD negotiation)** — `wayland-protocols["server","staging","unstable"]` is enabled in `Cargo.toml`, but `XdgDecorationHandler` is intentionally not registered in [`src/backend/mod.rs`](../../src/backend/mod.rs) (see the 🚧 comment next to where `CompositorState::new` is set up). We unilaterally render internal SSDs via `DecorationManager`. The forward-looking kill-switch is `[features].enable_xdg_decoration_protocol = false`; flipping it to `true` is a no-op until a follow-up PR wires the handler.

Both flags default to `false` so out-of-the-box Axiom ships with the simplified behavior; users who want either feature can opt in by setting the matching flag to `true` and supplying the corresponding implementation.
