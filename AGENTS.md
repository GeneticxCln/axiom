# Axiom — Project State

## Build & Test
```sh
cargo build              # expect clean (0 warnings)
cargo test               # expect all passing (143 as of last check)
cargo test --all-targets # includes benches and integration tests
```
CI runs under `xvfb-run`. `render()` presents real client pixels via the
Smithay 0.7 GLES winit backend (not a no-op).

## Active Cleanup

Goal: Strip over-engineering — delete dead code, unused deps, unrequested abstractions. Winit-only backend, no effects, no renderer, no xwayland. **All phases complete.**

### Phase 1 ✅ (completed)
- Deleted: `effects/`, `renderer/`, `xwayland/`, `xwm/`, `clipboard_bridge/`, `render_bridge/`, `xwayland_dispatch.rs`
- Deleted: `demo_workspace.rs`, `demo_phase4_effects.rs`, `sandbox.rs`, `drm.rs`
- Stripped Cargo.toml from 11 unused deps + reduced smithay features to 4
- Updated: `compositor.rs`, `backend/mod.rs`, `main.rs`, `lib.rs`, `decoration.rs`, integration tests

### Phase 2 ✅ (completed)
- Stripped `BackendKind::Drm` + all DRM match arms, `initialize_drm()`, `run_one_cycle_drm()`, `render_drm_frame()`
- Removed `EffectsConfig`, `XWaylandConfig` + all sub-structs (validation, merge, tests)
- Removed CLI flags: `no_effects`, `demo`, `effects_demo`, `backend`
- Removed IPC endpoints for effects/xwayland control
- Removed unused deps: `libseat`, `drm`, `drm-fourcc`, `gbm`, `input`, `udev`, `calloop`
- Removed dead functions/fields: `popup_render_id`, `preferred_text_mime_type`, `clipboard_update_tx`
- Cleaned 8 pre-existing dead_code warnings

### Bug Fixes ✅ (completed)
- Fixed `decoration_consumed_press` flag inversion (`backend/mod.rs:1595`): caller no longer overwrites flag that `handle_decoration_button()` already set
- Fixed IPC-triggered workspace commands not setting `needs_redraw` (`compositor.rs`): all 9 methods now request redraw after state change
- Fixed `WinitEvent::Redraw` handler (`backend/mod.rs:1346`): now sets `needs_redraw = true` so OS redraw requests are honored

### Rendering ✅ (completed)
- Rebuilt GLES rendering through the winit backend: `render()` binds the winit
  GLES backend, imports each client `wl_buffer` into a `GlesTexture`, draws it
  (plus a solid backdrop and SSD titlebars/buttons) via
  `SolidColorRenderElement` / `TextureRenderElement`, then submits. Real
  client pixels are shown.

## Known Gaps (not blocking)
- Full drag-and-drop data transfer is not yet implemented.
- Touch input is not yet implemented.
- `LazyUIMessage::EffectsControl` is accepted by IPC but effects are no-ops
  (effects module removed); the config `effects` section is retained as data only.

## Next / Contemplated
- Ship- and usability-focused improvements (DnD, touch)
