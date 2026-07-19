# Changelog

## v0.1.0-alpha.2 (2026-07-19)

### Features
- Server-side decoration quads rendered via WGPU solid-color pipeline
- Font atlas / glyph cache (ab_glyph) for future title text rendering
- Font atlas text pipeline compiled into renderer (ensure_text_pipeline)
- libseat/seatd/logind session management for DRM device access
- Session-aware LibinputDevice (opens devices through seatd when available)
- DRM connector/CRTC/encoder/mode enumeration with GBM surface creation
- Seatd daemon integration: graceful fallback when no session manager
- Metrics client example (IPC GetPerformanceReport query)
- IPC fuzz tests: malformed JSON, truncated UTF-8, extreme values

### Performance
- Surface format: prefer Bgra8UnormSrgb (matches headless pipeline default)
- render_to_surface_auto now receives config by parameter (avoids redundant lookup)
- Decoration quads use dedicated solid pipeline (separate from window render)
- Cached staging buffer reuse for DRM readback path

### Testing
- 233 tests passing (187 unit + 2 bin + 44 integration)
- 4 new IPC fuzz/malformed-input tests
- Code coverage script (cargo-tarpaulin)
- Coverage CI job uploading to Codecov
- 6 property-based layout invariants maintained
- Shell syntax validation (bash -n) for all scripts

### Bug Fixes
- render_to_surface_auto: surface config lookup after remove_entry crash
- Pipeline format mismatch: Bgra8UnormSrgb vs Rgba8UnormSrgb validation error
- Nested (winit) mode: no more surface timeout / emergency shutdown
- Nested mode hangs on X11 with WINIT_UNIX_BACKEND and EGL_BAD_CONFIG
- Smoke test prefers Wayland backend when available (avoids X11 EGL issues)
- Cargo-deny config: remove deprecated deny = [] key
- Advisory cleanup: remove stale RUSTSEC-2026-0190, document all ignores
- Clippy: items_after_test_module, for_kv_map, collapsible_match
- Formatting: cargo fmt across all source files
- Config tests: remove stale fields (lazy_loading, scale_factor, etc.)
- Remove unused deps: libloading, gl, mockall
- render_bridge: rename should_use_wgpu_gl_bridge → should_render
- SSD backend_prefers_server_side_decorations: doc matches reality

### Refactoring
- renderer/mod.rs module docs: complete WGPU-only rendering path documentation
- DRM hardware validation matrix tracked in docs/dev/DRM_HARDWARE_MATRIX.md
- Backend struct sealed: session stored in AxiomSmithayBackendReal + DrmBackend

### Dependencies
- ab_glyph 0.2: font atlas rasterization for decoration title text
- libseat 0.2: seat management for DRM/input device access
- smithay: enable backend_session_libseat feature

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
