# Axiom Master Development Plan

**Status:** Active
**Current Phase:** Phase 3 — Testing & Optimization (Phase 2 complete)
**Last Updated:** 2026-07-19

---

## Executive Summary

Axiom is an **alpha-stage hybrid Wayland compositor** (v0.1.0) using Smithay 0.7 + WGPU, inspired by niri's scrollable workspaces and Hyprland's effects.

### Current reality
- **Alpha prototype**, not a production desktop session replacement
- Nested (`--windowed`) mode is the primary development target, using direct WGPU surface presentation
- DRM/KMS path exists (GBM + dumb-buffer fallback) but is **not release-ready**
- **v0.1.0-alpha.2** pending tag; packaging assets exist (PKGBUILD, desktop entries, `axiom-session`) but remain alpha scaffolding
- CI, benchmarks, property-based tests, and security tooling are present
- **Test counts:** 233 tests (187 unit + 2 bin + 44 integration)
- **0 TODOs/FIXMEs** in source code
- **0 clippy warnings** (`-D warnings` clean)

---

## Phase 1: Immediate Fixes ✓ (Complete)

| Item | Status |
|------|--------|
| Fix 4 compiler warnings | ✅ Done |
| Fix 6 additional clippy lints | ✅ Done |
| Add `default_terminal`/`default_launcher` to config | ✅ Done |
| Replace hardcoded `xterm`/`dmenu_run` | ✅ Done |
| Add `#[must_use]` to critical success-returning fns | ✅ Done |
| Update default config TOML | ✅ Done |

**Exit criteria:** cargo build clean, cargo clippy zero warnings, all 233 tests pass. ✅

---

## Phase 2: Core Feature Completion (Weeks 1-6)

### 1. Visible SSD rendering ✅
- ✅ Solid-color shader (`solid.wgsl`) created with projection uniform + per-vertex color
- ✅ `SolidVertex`, `DecorationQuad`, pipeline factory, bind group layout added to renderer
- ✅ `prepare_decoration_resources` pre-builds GPU bind group + vertex buffer from quad list
- ✅ Both `render_to_surface` (nested) and `compose_full_frame` (headless) wired to draw decoration quads
- ✅ Compositor `prepare_frame_data()` generates quads from `DecorationManager` state
- ✅ Builds clean, all 233 tests pass
- ✅ Title text rendering wired and functional (depends on system font availability; font discovery failure logged gracefully)
- ✅ `backend_prefers_server_side_decorations()` flipped to `true` — SSD rendering is live
- ✅ `negotiated_xdg_decoration_mode()` flipped to `Mode::ServerSide`
- ✅ `XdgDecorationHandler` updated to set `ServerSide` mode when negotiated
- ✅ `general.debug` config wired to runtime log level control
- ✅ `general.vsync` config wired to WGPU present mode selector (`select_present_mode_for_vsync`)
- ✅ `input.mouse_accel` config wired to libinput `config_accel_set_speed` on device add
- ✅ `prune_dead_surfaces` already wired in `run_one_cycle_common` (line 2247)

### 2. DRM standalone GBM path ✅
- ✅ `render_drm_frame` now calls `stage_wgpu_scene_from_state` + `compose_full_frame` for GPU compositing
- ✅ New `KmsState::present_frame()` unified method: GBM page-flip when available, dumb-buffer fallback otherwise
- ✅ `DrmBackend::present_frame()` wrapper
- ✅ RGBA → BGRA conversion for GBM/dumb scanout
- ✅ GBM path: lock front buffer → `gbm_bo_write` pixel data → create FB → async page-flip
- ✅ Old `render_frame`/`present_composited_frame` fallback chain removed in favor of `present_frame`

### 3. Smithay 0.8 migration ⏳ Deferred
- Smithay 0.8 has **not been released** (latest is v0.7.0, Jun 2026)
- Smithay 0.7.0 already depends on wayland-server 0.31.13 and wayland-protocols 0.32.13 (latest)
- Gains `foreign_toplevel_list`, improved XDG protocols, better XWayland support when 0.8 ships
- **Revisit when Smithay cuts a 0.8 release**

### 4. Multi-monitor polish ✅
- ✅ Added `OutputConfig` to config with configurable `order` field for output positioning
- ✅ `sync_tapes_with_outputs` now accepts `config_order` — outputs listed in config appear first in that order, remaining live outputs appended after
- ✅ Empty config order preserves natural DRM enumeration order (backward compatible)
- ✅ Config validation: rejects empty names, invalid characters, and duplicates in output order
- ✅ 3 new tests: config order respected, absent outputs filtered, empty config falls back to natural order
- ✅ Updated `axiom.toml` with `[output]` section
- ✅ **Phase 2.4: Per-connector incremental modesetting.** `DrmBackend::apply_hotplug_diff` performs in-place add/remove of `KmsOutput`s without disturbing already-displayed monitors:
  - `compute_output_diff` — pure helper computing `(added, removed)` name diffs (10 unit tests)
  - `find_all_connected_connectors` refactored to accept `in_use_crtcs: &HashSet<crtc::Handle>` — new connectors never steal CRTCs from live outputs
  - `KmsState::build_kms_output` — single source of truth for modesetting one connector (used by both `open` and `allocate_one_output`)
  - `KmsState::scan_new_connectors` / `allocated_crtc_handles` — CRTC-aware scan for hotplug diff
  - `KmsState::allocate_one_output` — modeset a single newly-connected display
  - `KmsState::destroy_one_output` — tear down one disconnected output, restore saved CRTC, free resources
  - `reenumerate_outputs` deprecated in favor of `apply_hotplug_diff`
  - `backend/mod.rs` hotplug path now calls `apply_hotplug_diff` instead of `reenumerate_outputs`

**Exit criteria:** ✅ Nested mode fully functional with visible decorations. ✅ DRM mode renders on at least one real GPU family. ✅ All 243 tests pass. ⏳ Smithay 0.8 upgrade deferred — blocked on upstream release.

**Phase 2 complete.**

---

## Phase 3: Testing & Optimization

### 1. Expand property-based tests ✅
- 6 new property tests for workspace layout invariants:
  - `test_layout_count_matches_visible_windows` — |layouts| == visible non-minimized non-floating windows
  - `test_layout_no_overlap` — window rectangles in same column never intersect
  - `test_layout_positive_dimensions` — every rect has width ≥ 1, height ≥ 1
  - `test_layout_monotonic_y_order` — windows in same column top-to-bottom match column order
  - `test_layout_excludes_minimized_and_floating` — these windows never appear in layouts
  - `test_layout_cache_consistency` — repeated calls with identical state return identical results
- Added `Rectangle::intersects()` method for overlap detection

### 2. Real-client test matrix ✅
- `nested_smoke_test.sh` refactored to support `AXIOM_SMOKE_MATRIX=true` mode
- Tests: `weston-terminal`, `weston-smoke`, `foot` (when available)
- New CI job `nested-smoke` installs weston + foot, runs matrix under xvfb

### 3. Benchmark automation ✅
- CI performance job now uses `actions/cache` to persist Criterion baselines
- On `main`: `--save-baseline ci-main` stored in cache keyed by Cargo.lock hash
- On PRs: baseline is restored and compared; regressions emit a CI warning

### 4. Memory audit ✅
- Code review of window lifecycle (`WindowManager`, `ScrollableWorkspaces`, `AxiomCompositor`) — no confirmed leaks
- Code review of renderer GPU resources — all wgpu resources properly dropped
- Code review of IPC state — found and fixed 4 issues:
  - **Unbounded mpsc channel → bounded (256)** — prevents unbounded growth under backpressure
  - **Zombie client tasks on shutdown** — added `CancellationToken.child_token()` to abort blocked reads
  - **Compositor shutdown skips renderer** — added explicit `AxiomRenderer::shutdown()` + call from compositor
  - **Floating windows remain in column Vecs** — removed from column on float; cache invalidation on both float/unfloat

### 5. Performance optimization ✅
- **Layout cache invalidated every frame** — `WorkspaceTape::update_animations()` now returns `bool`; cache only invalidated when scroll position actually changed
- **`floating_rects()` allocates Vec on every motion** — early-return `Vec::new()` when no floating windows; `Vec::with_capacity` to avoid reallocation

**Exit criteria:** 233 tests passing (187 unit + 2 bin + 44 integration). Benchmark compilation verified. Zero clippy warnings. All Phase 2 features wired. Release builds compile clean. ✅

---

## Phase 4: Release Preparation (v0.1.0-alpha.2 complete)

Phase 4 produced the **v0.1.0-alpha.1** tag (packaging scaffolding) and the **v0.1.0-alpha.2** release (SSD rendering, config wiring, release build fixes, audit fixes).

### 1. Documentation ✅
- Release notes for v0.1.0-alpha.1 and v0.1.0-alpha.2
- CHANGELOG.md consolidated and up to date
- All module docs, user guides, architecture docs current

### 2. Packaging ✅
- Arch PKGBUILD, desktop entries, session wrapper, icon, config — present
- ✅ **Flatpak manifest** (`packaging/flatpak/org.axiom.Compositor.yml`) — freedesktop 24.08 runtime + Rust SDK extension, Wayland/DRI/DBus/input permissions, XWayland socket access, desktop entries, SVG icon, AppStream metainfo

### 3. Release automation ✅
- Version 0.1.0 in Cargo.toml, tags v0.1.0-alpha.1 / v0.1.0-alpha.2
- CHANGELOG.md, release process/checklist, release notes for both alphas

### 4. Security audit ✅
- cargo-deny + cargo-audit clean (RUSTSEC ignores documented)
- IPC security, UID verification, config permissions all in place
- ✅ **Process sandboxing** (`src/sandbox.rs`) — capability dropping via `prctl(PR_CAPBSET_DROP)` after DRM/input acquisition, `PR_SET_NO_NEW_PRIVS` + seccomp BPF filter (denies ptrace, bpf, kexec, kernel modules, ioports) applied before XWayland spawn

### 5. Reliability fixes (this release) ✅
- Release builds compile (create_solid_pipeline + anyhow::Context cfg fix)
- deny.toml syntax fixed (extra bracket)
- 0 TODOs/FIXMEs in source
- All 290 tests pass with 0 clippy warnings

**Exit criteria (alpha.2):** CHANGELOG updated, release notes written, all gates green ✅

**Phase 4 complete.** Flatpak manifest and process sandboxing shipped.

---

## What's Next (v0.2.0 / future alphas)

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| WGPU surface presentation fails on some GPUs | Medium | High | Fallback to headless render path |
| Smithay 0.8 migration breaks existing handlers | High | Medium | Phased upgrade, keep 0.7 test baseline |
| DRM path blocked by missing hardware access | High | High | Focus nested mode as primary alpha target |
| XWayland clipboard edge cases | Medium | Low | Document limitations, add test cases |

---

## Task Ordering

1. ✅ Phase 1 — Immediate fixes (complete)
2. ✅ Visible SSD rendering (Phase 2.1)
3. ✅ DRM GBM path (Phase 2.2)
4. ⏳ Smithay 0.8 migration (Phase 2.3) — deferred, no 0.8 release yet
5. ✅ Multi-monitor polish (Phase 2.4)
6. ✅ Testing & optimization (Phase 3)
7. ⚠️ Release preparation (Phase 4) — alpha.1 cut; CI/doc reliability hardening in progress
