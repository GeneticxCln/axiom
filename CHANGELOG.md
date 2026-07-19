# Changelog

## v0.1.0-alpha.2 (2026-07-19)

### Features
- **Server-side decorations enabled** — SSD rendering now live via WGPU solid-color pipeline
- **general.debug config** — runtime log level control via `log::set_max_level`
- **general.vsync config** — WGPU present mode selector (`Fifo` vs `Immediate`) driven by config
- **input.mouse_accel config** — applied to libinput pointer devices on add
- Font atlas / glyph cache (ab_glyph) for title text rendering
- Font atlas text pipeline compiled into renderer (ensure_text_pipeline)
- WGPU-only compositing pipeline — legacy dual GL/WGPU path removed
- Direct WGPU surface presentation + software DRM composite
- Config-driven blur sigma with runtime WGSL generation
- Focus ring (border_color uniform) for window focus indication
- DRM hotplug + wl_output protocol + DPI scale factor support
- DRM connector/CRTC/encoder/mode enumeration with GBM surface creation
- libseat/seatd/logind session management for DRM device access
- Scrollable workspace columns with layout caching
- Floating window support (removed from column layout on float)
- Multi-monitor support with configurable output ordering
- XWayland clipboard + XDG shell popup support
- IPC single-dispatch for command-type messages
- Metrics client example (IPC GetPerformanceReport query)

### Performance
- Layout cache conditionally invalidated (scroll-dirty flag)
- Floating rects early-return with Vec::with_capacity
- Bounded IPC channels (256) to prevent unbounded memory growth
- CancellationToken per client for clean shutdown of zombie IPC tasks
- Shadow batching and renderer caching
- Surface format: prefer Bgra8UnormSrgb
- Decoration quads use dedicated solid pipeline (separate from window render)
- Cached staging buffer reuse for DRM readback path

### Testing
- **233 tests passing** (187 unit + 2 bin + 44 integration) — 14 IPC fuzz + 9 config property + 36 backend are subsets of the 187 unit tests
- 4 new IPC fuzz/malformed-input tests
- 6 property-based layout invariants maintained
- Real-client smoke test matrix (weston-terminal, weston-smoke, foot)
- Benchmark CI with cached Criterion baselines and regression detection
- Memory audit with Valgrind (4 bugs fixed)
- Code coverage script (cargo-tarpaulin) with CI upload to Codecov
- Shell syntax validation (bash -n) for all scripts

### Bug Fixes
- **Release build regression** — `create_solid_pipeline` incorrectly gated behind `#[cfg(debug_assertions)]`
- **anyhow::Context import gated in release** — removed `#[cfg(debug_assertions)]` from import
- **deny.toml syntax error** — extra `]` bracket causing cargo-deny parse failure
- render_to_surface_auto: surface config lookup after remove_entry crash
- Pipeline format mismatch: Bgra8UnormSrgb vs Rgba8UnormSrgb validation error
- Nested (winit) mode: surface timeout / emergency shutdown fixed
- Nested mode hangs on X11 with WINIT_UNIX_BACKEND and EGL_BAD_CONFIG
- Smoke test prefers Wayland backend when available
- Advisory cleanup: RUSTSEC-2026-0190, RUSTSEC-2026-0192, RUSTSEC-2026-0196 documented
- Clippy: items_after_test_module, for_kv_map, collapsible_match
- Formatting: cargo fmt across all source files
- Config tests: remove stale fields
- Remove unused deps: libloading, gl, mockall
- render_bridge: rename should_use_wgpu_gl_bridge → should_render

### Refactoring
- renderer/mod.rs module docs: complete WGPU-only rendering path documentation
- DRM hardware validation matrix tracked in docs/dev/DRM_HARDWARE_MATRIX.md
- Backend struct sealed: session stored in AxiomSmithayBackendReal + DrmBackend

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

### Security
- IPC socket directory 0o700, socket file 0o600
- UID-based peer credential verification on connection
- Connection semaphore (max 16 concurrent)
- Idle timeout (60s) for inactive connections
- Config file saved with 0o600 permissions
- cargo-deny and cargo-audit in CI

### Dependencies
- ab_glyph 0.2: font atlas rasterization for decoration title text
- libseat 0.2: seat management for DRM/input device access
- smithay: enable backend_session_libseat feature
