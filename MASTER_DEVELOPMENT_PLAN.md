# Axiom Master Development Plan

**Status:** Active
**Current Phase:** Phase 4 — Release Preparation ✅
**Last Updated:** 2026-07-16

---

## Executive Summary

Axiom is an **alpha-stage hybrid Wayland compositor** (v0.1.0) using Smithay 0.7 + WGPU, inspired by niri's scrollable workspaces and Hyprland's effects.

### Current reality
- **227 tests passing** (183 unit + 44 integration)
- **Clean build, zero clippy warnings**
- **v0.1.0-alpha.1** tagged release
- Nested (`--windowed`) mode is the primary development target, using direct WGPU surface presentation
- DRM/KMS path exists via CPU dumb-buffer software rendering
- Documentation, packaging (Arch PKGBUILD + CI tarball), CI, benchmarks, property-based tests, and security audit all in place

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

**Exit criteria:** cargo build clean, cargo clippy zero warnings, all 220 tests pass. ✅

---

## Phase 2: Core Feature Completion (Weeks 1-6)

### 1. Visible SSD rendering ✅
- ✅ Solid-color shader (`solid.wgsl`) created with projection uniform + per-vertex color
- ✅ `SolidVertex`, `DecorationQuad`, pipeline factory, bind group layout added to renderer
- ✅ `prepare_decoration_resources` pre-builds GPU bind group + vertex buffer from quad list
- ✅ Both `render_to_surface` (nested) and `compose_full_frame` (headless) wired to draw decoration quads
- ✅ Compositor `prepare_frame_data()` generates quads from `DecorationManager` state
- ✅ Builds clean, all 220 tests pass
- ⏳ Title text rendering deferred (requires font atlas pipeline)
- ⏳ `backend_prefers_server_side_decorations()` remains `false` until text is renderable

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
- ⏳ Individual output hotplug (add/remove without full re-enumeration) — deferred, requires KmsState diffing improvements

**Exit criteria:** Nested mode fully functional with visible decorations. DRM mode renders on at least one real GPU family. All tests pass after Smithay upgrade.

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

**Exit criteria:** 183 unit + 44 integration = 227 tests passing. Benchmark baseline comparison in CI. Zero clippy warnings. All Phase 2 features wired.

---

## Phase 4: Release Preparation ✅

### 1. Documentation ✅
- `cargo doc` published — all modules have `//!` doc comments, architecture diagrams in `src/lib.rs`
- Architecture overview — `docs/dev/RENDER_ARCHITECTURE.md` covers the rendering pipeline
- User guide — `docs/user/RUNNING.md`, `docs/user/INSTALL.md`, `docs/user/CONFIGURATION.md` complete
- Known limitations — `docs/user/LIMITATIONS.md` documents all known gaps
- Security architecture — `docs/dev/SECURITY.md` documents threat model, IPC security, supply chain, known gaps
- Release notes for v0.1.0-alpha.1 — `release-notes/v0.1.0-alpha.1.md` populated with real content

### 2. Packaging ✅
- Arch PKGBUILD finalized — builds, installs session wrapper, desktop entries, icon, config, README, LICENSE
- CI package job — builds release binary, validates desktop entries and session wrapper, creates tarball artifact
- Binary release artifacts — CI uploads `axiom-v*-linux-amd64.tar.gz` as build artifact
- `check_packaging_assets.sh` validates all packaging files + desktop entries in CI
- `build_arch_package.sh` runs full PKGBUILD stages + smoke tests offline
- ⏳ Flatpak manifest — deferred, non-blocking for alpha

### 3. Session integration ✅
- `axiom.desktop` — nested launcher entry (`axiom --windowed`)
- `axiom-wayland.desktop` — Wayland session entry for display managers; includes `X-Wayland-Compositor=true`
- `axiom-session` — POSIX sh wrapper with config discovery (user → system → defaults)
- ⏳ systemd-logind/seatd integration — deferred (DRM opens `/dev/input/event*` directly, noted in known limitations)

### 4. Release automation ✅
- Version `0.1.0` in `Cargo.toml`, tag `v0.1.0-alpha.1` exists
- `CHANGELOG.md` created with full history from git log
- `docs/dev/RELEASE_PROCESS.md` documents the 6-step release flow
- `docs/dev/RELEASE_CHECKLIST.md` covers build/test gates, runtime checks, packaging, documentation, publication
- `scripts/release_prep.sh` automates release preparation (check, draft-notes, print-publish, all)
- `Makefile` targets: `release-check`, `release-prep`, `doc`, `doc-open`

### 5. Security audit ✅
- IPC socket directory `0o700`, socket file `0o600` — verified in code review
- UID-based peer credential verification on all IPC connections
- Connection semaphore (max 16), idle timeout (60s)
- Config file saved with `0o600`
- `cargo-deny` + `cargo-audit` in CI (`check_security.sh` script)
- Security architecture documented in `docs/dev/SECURITY.md` with known gaps identified
- ⏳ logind/seatd for DRM device access — deferred
- ⏳ XWayland sandboxing (Landlock/seccomp) — deferred

**Exit criteria:** `v0.1.0-alpha` tag cut. README accurate. Packaging builds from source. ✅

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
7. ✅ Release preparation (Phase 4)
