# 🗺️ Axiom Master Development Plan

**Status**: Active 🟢
**Current Phase**: Phase 7 (Production Polish)
**Last Updated**: 2026-06-24

---

## 📋 Executive Summary
Axiom is transitioning from a high-fidelity simulation to a functional Wayland compositor. The architectural foundation (Phases 1-5) is complete. We are currently in **Phase 6**, integrating the canonical Smithay backend to handle real Wayland clients, hardware input, and GPU rendering.

This document consolidates all previous roadmaps (`PRODUCTION_ROADMAP.md`, `PHASE5_ROADMAP.md`, `PHASE_6_IMPLEMENTATION_PLAN.md`) into a single source of truth.

---

## ✅ Completed Foundation (Phases 1-5)
All core subsystems are architected, implemented, and verified in simulation mode.

| Phase | Component | Key Achievements |
|-------|-----------|------------------|
| **1** | **Architecture** | Async Tokio event loop, modular design, logging, error handling. |
| **2** | **Core Systems** | TOML Config, IPC (Unix Domain Sockets), Lazy UI (AI) integration. |
| **3** | **Workspaces** | Niri-style infinite horizontal scrolling, dynamic columns, spring physics. |
| **4** | **Window Mgmt** | Tiling algorithms, focus logic, window states (floating/fullscreen). |
| **5** | **Effects Engine** | Hyprland-style animations, shader framework, blur/shadow references. |

---

## 🚀 Phase 6: Real Compositor Integration (✅ COMPLETE)
**Goal**: Replace simulation with a fully functional Wayland backend capable of running real applications (Firefox, Terminal, etc.).

### 6.1 Backend Migration (✅ Completed)
- [x] **Canonical Backend**: Migrated to `smithay_backend_real.rs`.
- [x] **Protocols**: Integrated core Wayland protocols (Compositor, XDG Shell, Seat, Output).

### 6.2 Real Window Lifecycle (✅ Completed)
- [x] **Surface Mapping**: `XdgShellHandler::new_toplevel` → `create_window_from_surface` → `WindowManager` + `ScrollableWorkspaces`.
- [x] **Lifecycle Hooks**: `destroy_window` cleans up all three subsystems (WM, workspace, renderer).
- [x] **State Sync**: Initial `configure` with 1024×720 default; dynamic tiling-based resizes via `configured_sizes` tracking in `render()`; serialized with `pending_configure` to avoid flooding clients.
- [x] **Dead Surface Pruning**: `prune_dead_surfaces` cleans up disconnected client state.

### 6.3 Input Routing (✅ Completed)
- [x] **Pointer**: `PointerMotionAbsolute` → `element_under()` hit test → surface focus via `pointer.motion()`.
- [x] **Pointer Buttons**: Left/right/middle clicks forwarded via `pointer.button()`.
- [x] **Keyboard**: `InputManager` intercepts global shortcuts (Super+key) → `FilterResult::Intercept`; non-shortcuts → `FilterResult::Forward` to clients.
- [x] **Bindings**: `process_actions` handles ScrollLeft/Right, Quit, CloseWindow, ToggleFullscreen, MoveWindowLeft/Right.
- [x] **Focus Chasing**: `SeatHandler::focus_changed` syncs Wayland focus → `WindowManager::focus_window`.
- [x] **Scroll**: Axis events forwarded to clients AND trigger workspace navigation.

### 6.4 Rendering Pipeline (✅ Completed)
- [x] **Buffer Management**: SHM buffer data cached on commit; uploaded to GL textures in `render()`.
- [x] **Compositor Loop**: `calculate_workspace_layouts()` positions windows via tiling; positions synced to `WindowManager`.
- [x] **GL Rendering**: `GlesRenderer` with lazy-compiled GLES 2.0 shader for textured quads; scissor-based placeholders for untextured windows.
- [x] **WGPU Renderer**: Parallel WGPU renderer with `upsert_window_rect`, shadow/blur queuing, and `render_to_surface_auto()`.
- [x] **Dual Render Path**: Backend GL pass runs first (layout + texture); compositor `render_frame()` follows (WGPU effects post-processing).
- [x] **Cursor**: Pointer position tracked; cursor image handler present (no-op).

---

## 🔮 Active Phase: Phase 7 - Production Polish
**Goal**: Stability, compatibility, and distribution.

### 7.1 Stability & Performance
- [x] **Error Recovery**: Consecutive-error threshold (5) triggers emergency shutdown; `force_next_tick_error` test mechanism validates recovery.
- [ ] **Optimization**: Reduce CPU/memory footprint of the composite loop.
- [ ] **Multi-Monitor**: Robust handling of hotplugging and DPI scaling.
- [x] **Integration Tests**: 22 integration tests covering IPC, window lifecycle, effects, compositor event loop, frame pacing, viewport resize, and error recovery.

### 7.2 Application Compatibility
- [x] **XWayland**: XWM event polling integrated into main loop; X11 window lifecycle (MapRequest, ConfigureRequest, UnmapNotify) tracked; clipboard atoms + SelectionRequest handling. (Remaining: Wayland→X11 data extraction from SelectionSource, XWM→Surface wrapping for layout integration.)
- [x] **Clipboard**: `SelectionHandler::new_selection` stores Wayland source + claims X11 ownership; `AxiomXwm::handle_selection_request` serves cached data to X11 apps; `set_clipboard_data()` lets IPC/compositor layer populate cache. (Remaining: async pipe-based data extraction from `SelectionSource`, Wayland←X11 direction via `send_selection`.)
- [x] **Popups**: Popup buffer uploads wired (SHM → GL textures); render pass draws popups above windows at correct absolute positions; grab-based dismissal on outside click; `send_popup_done()` lifecycle.
- [ ] **DPI Scaling**: Fractional scaling support for HiDPI displays.

### 7.3 Effects Integration
- [x] **GPU Effects Pipeline**: WGPU `render()` now dispatches shadow/blur passes via headless target; `render_frame()` populates per-frame effect queues consumed by GPU passes. (Next: wire headless target into GL compositing for on-screen output.)
- [ ] **GL Effects**: Alternatively, implement GL-based blur/shadow shaders for the GL rendering path.
- [ ] **Animations**: Wire `AnimationController` spring physics and easing curves into window transitions.

### 7.4 Distribution
- [ ] **Packaging**: AUR, RPM, DEB packages.
- [ ] **Docs**: User guide for configuration and installation.

---

## ✅ Validation Checkpoints (Phase 6 Incremental Tests)
Use these as pass/fail gates while implementing each sub-phase:

| # | Checkpoint | Phase | Status |
|---|------------|-------|--------|
| 1 | Backend brings up a Wayland socket | 6.1 | ✅ Done |
| 2 | `weston-simple-egl` or `weston-terminal` connects and creates a surface | 6.2 | ✅ Done |
| 3 | Window appears in Axiom layout and is tracked in `WindowManager` | 6.2 | ✅ Done |
| 4 | Mouse/keyboard events are routed and keybindings still work | 6.3 | ✅ Done |
| 5 | The client buffer is visible in the renderer | 6.4 | ✅ Done |

---

## 🛠️ Immediate Next Steps (Phase 7 Kick-off)

1. **GPU Effects Integration**: ✅ WGPU blur/shadows wired; popups render in GL pass. Next: wire headless WGPU output into GL compositing.
2. **XWayland Clipboard**: ✅ XWM polling + SelectionHandler + clipboard bridge. Next: async SelectionSource→cache data extraction.
3. **Animations**: Wire `AnimationController` spring physics into window transitions.
4. **Multi-Monitor**: Test and fix hotplug DPI scaling.
