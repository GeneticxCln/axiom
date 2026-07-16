# Security Architecture

## Threat Model

Axiom is an alpha compositor that currently operates in two modes:
- **Nested mode** — runs inside an existing Wayland or X11 session; limited privilege exposure
- **DRM mode** — runs as a standalone display server with direct hardware access

Primary trust boundary: the Wayland socket and IPC socket. Clients connecting to these sockets are considered untrusted.

## IPC Security

- **Socket permissions**: Directory created with `0o700`, socket file tightened to `0o600` after bind
- **Peer credential verification**: All IPC connections verified via `UnixStream::peer_cred()` — connections from different UIDs are rejected
- **Connection limiting**: Semaphore-based cap (default 16 concurrent connections)
- **Idle timeout**: Inactive connections disconnected after 60s
- **Message parsing**: Bounded mpsc channels (256) prevent unbounded memory growth under backpressure

## File System

- **Config file**: Saved with `0o600` permissions
- **Socket path**: `$XDG_RUNTIME_DIR/axiom/axiom.sock` (fallback: `/tmp/axiom-<pid>/axiom-lazy-ui.sock`)
- **XWayland**: Standard `/tmp/.X11-unix/X<display>` socket path

## DRM Mode

- **Device access**: `/dev/input/event*` opened directly via `std::fs::OpenOptions::open()` — currently requires root or appropriate capabilities
- **Session management**: Not yet integrated with logind/seatd (tracked in Phase 4.3)
- **VT switching**: Not yet implemented — no text console restore on exit

## XWayland

- **No sandboxing**: XWayland runs with the full privileges of the compositor process
- No Landlock, seccomp, bubblewrap, or namespace isolation applied
- XWayland lifecycle is managed (spawn, display/socket setup, XWM socket pair) but without security wrappers

## Supply Chain

- **cargo-deny**: License allowlist (9 licenses), dependency bans, advisory checking in CI
- **cargo-audit**: Vulnerability scanning in CI with `--deny warnings`
- **deny.toml**: 6 explicitly ignored advisories (reviewed), license allowlist, dependency bans

## Known Gaps

1. **logind/seatd integration** — needed for automatic DRM/input device permission management
2. **XWayland sandboxing** — should apply Landlock or seccomp to XWayland subprocess
3. **Capability dropping** — should drop Linux capabilities after acquiring DRM/input devices
4. **VT switching** — should restore text console on compositor exit
5. **Security documentation** — this document is the initial version; threat model should be expanded

## Tooling

- `bash scripts/check_security.sh` — runs `cargo audit` + `cargo deny`
- CI: `.github/workflows/ci.yml` security job runs both checks
- `Makefile`: `audit` and `audit-fix` targets
