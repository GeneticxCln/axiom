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

-   **Flag**: `--windowed`
-   **Usage**: `cargo run -- --windowed`

### 3. Headless Test Backend
A test-only backend that skips Wayland socket bind and display creation, so the CI can construct a compositor (`AxiomCompositor::new_for_test` + `AxiomSmithayBackendReal::new_for_test`) without real system resources. Used by the 79 unit tests in `cargo test --lib`.

### 4. Legacy / Experimental
The codebase no longer ships experimental backends in `src/experimental/`. Old Smithay 0.3 scaffolding was migrated to `src/backend/` under the `experimental-smithay` feature flag (currently a no-op marker; the real Smithay 0.7 path is always-on).

## Feature Flags (`Cargo.toml`)

| Flag                | Effect                                                                              |
|---------------------|--------------------------------------------------------------------------------------|
| `default`           | Empty — production build; pulls in Smithay 0.7 + wgpu 0.19.                         |
| `real-compositor`   | Marker flag (empty feature). Reserved for the live-compositor path.                  |
| `wgpu-present`      | Marker flag; future use for `--wgpu` integration of `AxiomRenderer`.                |
| `demo`              | Enables internal demo modes (`--demo`, `--effects-demo`).                            |
| `examples`          | Builds the `metrics_client` example.                                                 |
| `experimental-smithay` | Marker flag; kept for ABI compatibility with the legacy `src/experimental/` path. |

## Picking a Backend at Runtime

There is no runtime selector; build the binary with `cargo build` (production) and run it with `--windowed` if you want the winit nested-session mode. For CI, no special flag is needed — `cargo test --lib` exercises every subsystem through the headless path.
