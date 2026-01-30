# Running Axiom

## Standard Execution

To run Axiom directly (e.g., from a TTY):

```bash
# Recommended: Run the release build
sudo ./target/release/axiom
```

*Note: `sudo` may be required depending on your permission setup for accessing input devices and DRM cards directly.*

## Windowed Mode (Development)

You can run Axiom nested inside another Wayland compositor or X11 session for testing:

```bash
./target/debug/axiom --windowed
```

## Debug Mode

Enable verbose logging for debugging:

```bash
./target/debug/axiom --debug
```

## Environment Variables

-   `RUST_LOG`: Control logging levels (e.g., `RUST_LOG=debug`).
-   `WAYLAND_DISPLAY`: Set by Axiom to `wayland-axiom-0` (or similar) for clients.
