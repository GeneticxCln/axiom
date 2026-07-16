# Changelog

## v0.1.0-alpha.1 (2026-07-15)

### Features
- WGPU-only compositing pipeline — replaces legacy dual GL/WGPU render path
- Direct WGPU surface presentation + software DRM composite — eliminates CPU readback and GL bridge
- Config-driven blur sigma with runtime WGSL generation
- Focus ring (border_color uniform) for window focus indication
- DRM hotplug + wl_output protocol + DPI scale factor support
- IPC single-dispatch for command-type messages
- Scrollable workspace columns with layout caching
- Floating window support (removed from column layout on float)
- SSD decoration pipeline with solid-color shader
- Multi-monitor support with configurable output ordering
- XWayland clipboard support
- XDG shell popup support

### Performance
- Layout cache conditionally invalidated (scroll-dirty flag)
- Floating rects early-return with Vec::with_capacity
- Bounded IPC channels (256) to prevent unbounded memory growth
- CancellationToken per client for clean shutdown of zombie IPC tasks
- Shadow batching and renderer caching

### Testing
- 227 tests passing (183 unit + 44 integration)
- Property-based layout invariant tests (count preservation, no overlap, positive dimensions, monotonic y-order)
- Real-client smoke test matrix (weston-terminal, weston-smoke, foot)
- Benchmark CI with cached Criterion baselines and regression detection
- Memory audit with Valgrind (4 bugs fixed)
- Code coverage reporting

### Packaging
- Arch Linux PKGBUILD (axiom-compositor-git) with desktop entries and session wrapper
- Session wrapper (axiom-session) with config discovery
- Nested and Wayland-session desktop entries
- SVG icon and default config shipped with package

### Documentation
- Architecture overview in src/lib.rs
- User guide for nested + DRM modes (docs/user/)
- Build, setup, and contributing guides (docs/dev/)
- Release process, checklist, and release notes template
- Known limitations documented

### Bug Fixes
- WGSL shader validation + pipeline visibility
- XWayland restart on startup (remove premature auto-start)
- Runtime crash on immediate shutdown and GL bridge panic
- SurfaceError::Outdated handling in render_output (reconfigure + retry)
- IPC P0-P1 issues (UID peer credential validation, socket permissions 0o600)
- Effects-time label and docs
- Compile errors, config path inconsistency, test stability

### Security
- IPC socket directory 0o700, socket file 0o600
- UID-based peer credential verification on connection
- Connection semaphore (max 16 concurrent)
- Idle timeout (60s) for inactive connections
- Config file saved with 0o600 permissions
- cargo-deny and cargo-audit in CI
