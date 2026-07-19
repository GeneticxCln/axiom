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
- **Session management**: libseat/seatd integration for DRM master and input device access (see `DrmBackend::session`)
- **Capability dropping (Phase 4)**: After acquiring DRM master and opening input devices, all Linux capabilities except `CAP_SYS_NICE` are dropped via `prctl(PR_CAPBSET_DROP)`. The compositor cannot reacquire capabilities after this point.
- **VT switching**: Not yet implemented — no text console restore on exit

## XWayland

- **PR_SET_NO_NEW_PRIVS + seccomp filter (Phase 4)**: Before spawning XWayland, `sandbox::apply_sandbox()` is called which:
  - Sets `PR_SET_NO_NEW_PRIVS` — prevents the compositor and all children from gaining new privileges via setuid, file capabilities, or seccomp transitions
  - Installs a seccomp BPF filter that denies: `ptrace`, `process_vm_readv`, `process_vm_writev`, `perf_event_open`, `bpf`, `kexec_load`, `kexec_file_load`, `init_module`, `finit_module`, `delete_module`, `iopl`, `ioperm`
  - All other syscalls are allowed (default-allow, explicit-deny)
- **No Landlock filesystem sandbox**: XWayland can access the full filesystem. Future work: restrict to `/tmp/.X11-unix/` and shared memory paths only.
- XWayland lifecycle is managed (spawn, display/socket setup, XWM socket pair) with the sandbox applied before exec

## Supply Chain

- **cargo-deny**: License allowlist (9 licenses), dependency bans, advisory checking in CI
- **cargo-audit**: Vulnerability scanning in CI with `--deny warnings`
- **deny.toml**: 6 explicitly ignored advisories (reviewed), license allowlist, dependency bans

## Known Gaps

1. ✅ ~~logind/seatd integration~~ — libseat session management wired in `DrmBackend`
2. ✅ ~~XWayland sandboxing~~ — seccomp filter + NO_NEW_PRIVS applied before XWayland spawn (Phase 4)
3. ✅ ~~Capability dropping~~ — `sandbox::drop_capabilities()` called after DRM/input device acquisition (Phase 4)
4. **Landlock filesystem sandbox** — should restrict XWayland to `/tmp/.X11-unix/` and shared memory paths
5. **VT switching** — should restore text console on compositor exit
6. **Userspace sandboxing** — consider bubblewrap or namespace isolation for XWayland as additional defense-in-depth

## Tooling

- `bash scripts/check_security.sh` — runs `cargo audit` + `cargo deny`
- CI: `.github/workflows/ci.yml` security job runs both checks
- `Makefile`: `audit` and `audit-fix` targets
