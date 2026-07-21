# Changelog

## Unreleased

### Correction — winit-only, GLES rendering

The entries below that mention a **WGPU pipeline**, **DRM/KMS**, **XWayland**,
**seccomp sandbox**, or the **clipboard/render bridges** describe a design that
has since been **removed**. The compositor is now **winit-only** (Smithay 0.7
`backend_winit`) with **GLES rendering** via `GlesRenderer` — there is no `wgpu`
dependency and no DRM backend. The following were deleted:

- `effects/`, `renderer/` (WGPU/WGSL pipeline), `xwayland/`, `xwm.rs`,
  `sandbox.rs`, `src/backend/drm.rs`, `clipboard_bridge.rs`, `render_bridge.rs`
- `BackendKind::Drm` and all DRM match arms (`initialize_drm`, `run_one_cycle_drm`,
  hotplug monitor, libseat session)
- `XWaylandConfig` and the `no_effects` / `backend` CLI flags
- deps: `libseat`, `calloop`, `drm`, `drm-fourcc`, `gbm`, `input`, `udev`

The config `effects` section is retained as **data only** (IPC `EffectsControl`
is accepted but is a no-op). The `placeholder-pipeline` Cargo feature and its
WGSL shader are also gone; windows without a committed buffer simply are not
drawn.

### Phase 2: Core feature completion — COMPLETE ✅

#### Phase 2.4: Per-connector incremental modesetting ✅

- **`DrmBackend::apply_hotplug_diff` — per-connector incremental modesetting.** *(Superseded — DRM backend removed; retained here only as historical record of the original design.)*
- **`compute_output_diff` — pure helper behind the diff math.** Free function in `src/backend/drm.rs` taking `&[String]` existing + `&[String]` new and returning `(Vec<String> added, Vec<String> removed)`. Pure: no hardware, no allocation beyond the two return vectors. Unit-tested in `mod tests` for empty / identical / single-add / single-remove / mixed / replace / idempotent / duplicates / case-sensitive / both-empty cases (10 tests, all clippy-clean).
- **`find_all_connected_connectors` — CRTC-aware connector scan.** Refactored to take an `in_use_crtcs: &HashSet<crtc::Handle>` argument and skip CRTCs that are already pinned by existing outputs. Picks the first compatible CRTC that is NOT in use, so newly-arrived connectors can never steal a CRTC from an already-displayed monitor. Called from both `KmsState::open` (with an empty set) and `KmsState::scan_new_connectors` (with `allocated_crtc_handles()`).
- **`KmsState::allocate_one_output` / `destroy_one_output` / `allocated_crtc_handles` / `scan_new_connectors` / `build_kms_output`.** New per-connector modeset primitives. Single source of truth: `build_kms_output` is the helper that knows how to modeset one connector (CPU scanout / GBM / dumb fallback branches), and both `KmsState::open` (initial enumeration) and `allocate_one_output` (incremental hotplug add) call it.

### Build / Feature gates

- **`placeholder-pipeline` feature removed.** The WGSL placeholder pipeline and
  its `placeholder.wgsl` shader no longer exist; untextured windows are simply
  not drawn. The `default = ["placeholder-pipeline"]` feature set is gone.
  *(See the Correction note at the top of this release.)*

### CI

- **Feature-off integration tests now have a CI lane.** A new `feature-off-test` GitHub Actions job runs `cargo test --no-default-features --test integration_tests --lib` so the `cfg(not(feature = "placeholder-pipeline"))`-gated tests in `tests/integration_tests.rs` (`test_compose_full_frame_skips_untextured_windows`, `test_prepare_window_resources_skips_untextured_window`) actually execute under CI.

### Hardening

- **Phase 1.A4 drop-order invariant** is now backstopped by a `static_assertions::assert_fields!` compile-time check in `compositor::tests::test_phase1_a4_drop_order_symbols_locked`. The macro enforces field *presence* (any rename of `state` / `winit_backend` / `winit_event_loop` triggers a compile error in CI); declaration *order* remains the responsibility of the SAFETY comment in `backend/mod.rs::AxiomSmithayBackendReal::initialize_winit`, documented inline.

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
