# Running Axiom

## Supported alpha target

Axiom is currently best evaluated in **nested/windowed mode**:

```bash
cargo run -- --windowed --debug
```

This keeps Axiom inside your existing desktop session and is the most complete runtime path in the current alpha state.

## Basic smoke test

Once Axiom is running in nested mode, launch a simple Wayland client against its socket.

Example using `weston-terminal`:

```bash
# Terminal 1
cargo run -- --windowed --debug

# Terminal 2 (replace the socket with the one printed by Axiom)
WAYLAND_DISPLAY=wayland-axiom-12345 weston-terminal
```

A successful alpha-path smoke test is:
- Axiom starts without crashing
- a client connects and maps a surface
- keyboard/pointer input still works in the nested window
- closing the client removes the window cleanly
- shutting down Axiom exits cleanly

## Automated smoke script

This repository now ships an automated nested smoke test that launches Axiom, probes its Wayland socket, starts a real client, waits for a mapped XDG toplevel, then verifies clean teardown:

```bash
cargo build
xvfb-run -a ./scripts/nested_smoke_test.sh ./target/debug/axiom
```

The script uses `weston-terminal` as the default real client and prefers `weston-info`/`wayland-info` for an additional registry probe when available.

## Standalone / DRM mode

A standalone DRM/KMS mode exists:

```bash
cargo run -- --backend=drm
```

But it is still a development target, not the recommended day-to-day path yet.

## Debug logging

```bash
RUST_LOG=debug cargo run -- --windowed
```

## IPC socket

Preferred socket path:
- `$XDG_RUNTIME_DIR/axiom/axiom.sock`

Fallback when `XDG_RUNTIME_DIR` is unavailable:
- `/tmp/axiom-<pid>/axiom-lazy-ui.sock`

Helper scripts in this repo can also use `AXIOM_SOCKET_PATH` to target a specific socket path.

## Notes

- Nested mode is the primary alpha target.
- Standalone DRM mode may require additional permissions and a fuller local setup.
- If you are only verifying that the compositor starts and accepts clients, prefer nested mode first.
- See [Known Limitations](LIMITATIONS.md) before treating Axiom as anything other than alpha software.
