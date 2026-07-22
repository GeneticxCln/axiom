# Changelog

## v0.1.0-alpha (2026-07-22)

### Cleanup — over-engineering stripped

- **Deleted:** `effects/`, `renderer/` (WGPU/WGSL pipeline), `xwayland/`, `xwm.rs`,
  `sandbox.rs`, `src/backend/drm.rs`, `clipboard_bridge.rs`, `render_bridge.rs`
- **Deleted:** `BackendKind::Drm` and all DRM match arms, `initialize_drm`,
  `run_one_cycle_drm`, hotplug monitor, libseat session
- **Deleted:** `XWaylandConfig`, `EffectsConfig`, `no_effects` / `backend` CLI flags
- **Removed deps:** `libseat`, `drm`, `drm-fourcc`, `gbm`, `input`, `udev`, `calloop`
- **Removed:** `placeholder-pipeline` feature and WGSL shader
- **Fixed:** `decoration_consumed_press` flag inversion in titlebar click handling
- **Fixed:** IPC-triggered workspace commands not setting `needs_redraw`
- **Fixed:** `WinitEvent::Redraw` handler now sets `needs_redraw = true`

### Rendering

- **GLES rendering through winit backend** — `render()` binds the winit GLES backend,
  imports each client `wl_buffer` into a `GlesTexture`, draws it via
  `SolidColorRenderElement` / `TextureRenderElement`, then submits. Real client
  pixels are shown.
- **Server-side decorations** rendered via GLES solid-color pipeline with title text
  (ab_glyph font atlas) when system fonts are available.
- **Occlusion culling** — front-to-back pre-pass skips surface trees of fully
  covered windows.
- **Surface commit counters** — per-surface increment on commit for damage tracking.

### IPC

- **Unix socket JSON IPC** with `LazyUIMessage` protocol (workspace control,
  clipboard, DnD, health check, performance report).
- **Non-blocking dispatch** — command-type messages processed in event loop tick,
  not in the socket handler.
- **Secure socket permissions** — XDG_RUNTIME_DIR/axiom created with 0o700,
  socket file 0o600, UID ownership verified on bind.
- **Peer credential validation** — `validate_peer_credentials_for_test` hook
  with unit tests.
- **Oversized-line disconnect** — clients sending lines > 4096 bytes are
  disconnected.
- **IPC socket discovery example** (`examples/ipc_discover.rs`) — searches
  `$XDG_RUNTIME_DIR/axiom/axiom.sock`, `/tmp/axiom-*/axiom-lazy-ui.sock`,
  supports `AXIOM_SOCKET_PATH` env var.

### Drag-and-Drop

- **Server-initiated DnD** — `StartDnd { text, mime_type }` IPC message triggers
  `start_server_dnd` on the backend, which populates clipboard cache and initiates
  a real DnD grab via Smithay's pointer `start_dnd`.

### Backend Refactoring

- **`backend/mod.rs` split** into focused submodules:
  - `state.rs` — `State` struct, all Smithay handler trait impls, delegate macros
  - `winit.rs` — `AxiomSmithayBackendReal` lifecycle, winit event loop
  - `render.rs` — GLES render loop, texture cache, occlusion
  - `input.rs` — input routing
  - `clipboard.rs` — clipboard helpers
  - `screencopy.rs` — screencopy capture dispatch
- **Fractional-scaling helpers** — `scale_to_physical` / `scale_to_logical`
  centralized in `workspace/mod.rs`, replacing duplicated math in `render.rs`.

### Multi-Output (Experimental)

- **`multi-output-experimental` feature flag** — gates all multi-output code.
- **Per-output render loop** — `render()` iterates outputs, prepares elements
  per-output, submits per-output frame. Shared texture cache across outputs.
- **Virtual multiple outputs** in winit test mode (2 outputs: 1920×1080 + 1280×720).
- **Design doc** at `docs/dev/MULTI_OUTPUT.md`.
- **4 integration tests** under `--features multi-output-experimental`.

### CI & Quality

- **CI workflow** (`.github/workflows/ci.yml`): `build` job with fmt, clippy
  (`-D warnings`), `xvfb-run -a cargo test`. Cargo caching.
- **Coverage job** — `cargo tarpaulin` with artifact upload.
- **Audit job** — `cargo audit --deny warnings` + `cargo-deny`.
- **Bench compile job** — `cargo bench --no-run` with artifact upload.
- **Release workflow** (`.github/workflows/release.yml`) — `workflow_dispatch`,
  full test suite, release build, audit, artifact upload.
- **`compiletime-invariants` feature** — gates `static_assertions` dependency.

### Testing

- **149 lib tests** + 20 integration tests + 4 e2e tests + 11 viewport/scale tests
  + 5 LRU cache tests + 4 multi-output tests — all passing.
- **IPC hardening tests** — 7 new tests for socket permissions, peer credential
  validation, full accept path.
- **LRU cache eviction tests** — 5 tests for insert/retrieve/eviction/overwrite/LRU
  promotion at 256-entry capacity.
- **Viewport resize & fractional scaling tests** — 11 tests covering 1080p/1440p/4K,
  scales 1.0/1.25/1.5/2.0, round-trip, layout cache invalidation.
- **Wayland-client e2e tests** — 4 tests for toplevel, clipboard, fullscreen, minimize.
- **Multi-output integration tests** — 4 tests under feature flag.

### Packaging

- **Systemd user unit** (`packaging/systemd/axiom.service`) — `Type=notify`,
  `graphical-session.target`, restart on failure.
- **Debian control file** skeleton (`packaging/debian/control`).
- **Flatpak manifest** skeleton (`packaging/flatpak/manifest.json`).
- **Desktop entries** — `axiom.desktop`, `axiom-wayland.desktop`, session wrapper.
- **Arch Linux PKGBUILD** directory.
- **Packaging README** with install instructions.

### Documentation

- **`ARCHITECTURE.md`** — system architecture overview.
- **`RELEASE.md`** — release process and checklist.
- **`CONTRIBUTING.md`** — build/test instructions, PR workflow, code style.
- **`docs/dev/MULTI_OUTPUT.md`** — multi-output design doc.
- **`docs/dev/PROFILE.md`** — render profiling with perf/flamegraph.
- **`docs/dev/RENDER_ARCHITECTURE.md`** — updated with multi-output notes.
- **`scripts/profile_render.sh`** — perf/flamegraph capture script.

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
