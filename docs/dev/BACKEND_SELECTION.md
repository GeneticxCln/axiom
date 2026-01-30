# Backend Selection

Axiom supports multiple backend implementations for development and testing.

## Available Backends

### 1. Real Compositor (Default / Recommended)
The canonical backend based on Smithay 0.3.x. It supports real Wayland clients, input handling, and protocol extensions.

-   **Source**: `src/experimental/smithay/smithay_backend_real.rs`
-   **Features**: `real-compositor` + `experimental-smithay`
-   **Build**: `cargo build --features "real-compositor experimental-smithay"`
-   **Capabilities**:
    -   Client connection handling
    -   Window management
    -   Protocol extensions (Layer Shell, XDG Decoration, etc.)
    -   Input processing

### 2. Winit Backend (Windowed Mode)
Runs the compositor nested inside another Wayland/X11 session using `winit`. Useful for rapid iteration without switching TTYs.

-   **Flag**: `--windowed`
-   **Usage**: `cargo run -- --windowed`

### 3. Headless / Mock Backends (Legacy/Testing)
Various experimental backends exist in `src/experimental/` for testing specific subsystems in isolation.
