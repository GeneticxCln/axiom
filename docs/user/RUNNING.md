# Running Axiom

## Supported runtime

Axiom runs in **nested/windowed mode** using winit + GLES:

```bash
cargo run -- --windowed --debug
```

This keeps Axiom inside your existing desktop session and is the complete, recommended alpha path.

## Basic smoke test

Once Axiom is running in nested mode, launch a simple Wayland client against its socket.

Example using `weston-terminal`:

```bash
# Terminal 1
cargo run -- --windowed --debug

# Terminal 2 (replace the socket with the one printed by Axiom)
WAYLAND_DISPLAY=wayland-axiom-12345 weston-terminal
```

A successful smoke test:
- Axiom starts without crashing
- A client connects and maps a surface
- Server-side decoration titlebar appears
- Closing the client removes the window cleanly
- Shutting down Axiom exits cleanly

## Automated smoke script

An automated nested smoke test lives in the repository:

```bash
cargo build
xvfb-run -a ./scripts/nested_smoke_test.sh ./target/debug/axiom
```

The script launches Axiom, probes its Wayland socket, starts `weston-terminal`, waits for a mapped XDG toplevel, then verifies clean teardown.

## Debug logging

```bash
RUST_LOG=debug cargo run -- --windowed
```

## IPC socket

Preferred socket path:
- `$XDG_RUNTIME_DIR/wayland-axiom-<pid>`

## Notes

- Nested mode is the primary alpha target.
- See [Known Limitations](../user/LIMITATIONS.md) before treating Axiom as anything other than alpha software.