# Security Architecture

## Threat Model

Axiom is an alpha compositor running in nested mode inside an existing
Wayland or X11 session, with limited privilege exposure.

Primary trust boundary: the Wayland socket and IPC socket. Clients connecting
to these sockets are considered untrusted.

## IPC Security

- **Socket permissions**: Directory created with `0o700`, socket file tightened to `0o600` after bind
- **Peer credential verification**: All IPC connections verified via `UnixStream::peer_cred()` — connections from different UIDs are rejected
- **Connection limiting**: Semaphore-based cap (default 16 concurrent connections)
- **Idle timeout**: Inactive connections disconnected after 60s
- **Message parsing**: Bounded mpsc channels (256) prevent unbounded memory growth under backpressure

## File System

- **Config file**: Saved with `0o600` permissions
- **Socket path**: `$XDG_RUNTIME_DIR/axiom/axiom.sock` (fallback: `/tmp/axiom-<pid>/axiom-lazy-ui.sock`)

## Supply Chain

- **cargo-deny**: License allowlist (9 licenses), dependency bans, advisory checking in CI
- **cargo-audit**: Vulnerability scanning in CI with `--deny warnings`
- **deny.toml**: 6 explicitly ignored advisories (reviewed), license allowlist, dependency bans

## Known Gaps

None currently tracked.

## Tooling

- `bash scripts/check_security.sh` — runs `cargo audit` + `cargo deny`
- CI: `.github/workflows/ci.yml` security job runs both checks
- `Makefile`: `audit` and `audit-fix` targets
