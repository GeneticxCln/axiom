# 🗺️ Axiom Master Development Plan

**Status**: Active 🟢
**Current Phase**: Phase 6 (Real Compositor Integration)
**Last Updated**: 2026-01-30

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

## 🚀 Active Phase: Phase 6 - Real Compositor Integration
**Goal**: Replace simulation with a fully functional Wayland backend capable of running real applications (Firefox, Terminal, etc.).

### 6.1 Backend Migration (✅ Completed)
- [x] **Canonical Backend**: Migrated `run_real_backend` to use `smithay_backend_real.rs`.
- [x] **Cleanup**: Removed legacy `backend_real.rs` and unused modules.
- [x] **Compilation**: Verified clean build with `real-compositor` feature.
- [x] **Protocols**: Integrated core Wayland protocols (Compositor, XDG Shell, Seat, Output) and extensions (Layer Shell, Viewporter).

### 6.2 Real Window Lifecycle (🔄 Next Priorities)
**Goal**: Map Smithay `XdgSurface` events to `AxiomWindow` structures to make windows "exist" in the workspace logic.

- [ ] **Surface Mapping**: Create `AxiomWindow` from `XdgToplevel`.
- [ ] **Lifecycle Hooks**: Connect `request_map`, `request_unmap`, `destroy` to `WindowManager`.
- [ ] **State Sync**: Implement `configure` events to resize windows based on tiling layout.
- [ ] **Decorations**: Negotiate Server-Side Decorations (SSD) vs Client-Side Decorations (CSD).

### 6.3 Input Routing
**Goal**: Route physical events (libinput) from Smithay -> Axiom InputManager -> Wayland Clients.

- [ ] **Pointer**: Map mouse movement/clicks to the focused window surface.
- [ ] **Keyboard**: Forward key events to the active `wl_seat` focus.
- [ ] **Bindings**: Ensure global shortcuts (e.g., `Super+Enter`) intercept before client delivery.
- [ ] **Focus Chasing**: Update Wayland focus when `WindowManager` changes active window.

### 6.4 Rendering Pipeline
**Goal**: Draw the actual client pixels (buffers) onto the screen using WGPU/OpenGL.

- [ ] **Buffer Management**: Import `wl_shm` and DMA-BUF buffers into textures.
- [ ] **Compositor Loop**: Render `AxiomWindow` textures at their calculated workspace positions.
- [ ] **Effects Integration**: Apply shaders (blur, rounded corners) to these real textures.
- [ ] **Cursor Rendering**: Draw the hardware cursor overlay.

---

## 🔮 Future Phase: Phase 7 - Production Polish
**Goal**: Stability, compatibility, and distribution.

### 7.1 Stability & Performance
- [ ] **Error Recovery**: Graceful handling of GPU context loss or client crashes.
- [ ] **Optimization**: Reduce CPU/memory footprint of the composite loop.
- [ ] **Multi-Monitor**: Robust handling of hotplugging and DPI scaling.

### 7.2 Application Compatibility
- [ ] **XWayland**: Full integration for X11 apps (Steam, older tools).
- [ ] **Clipboard**: Implement `wl_data_device` for copy/paste.
- [ ] **Popups**: Correct handling of menus and tooltips (XDG Popups).

### 7.3 Distribution
- [ ] **Packaging**: AUR, RPM, DEB packages.
- [ ] **Docs**: User guide for configuration and installation.

---

## ✅ Validation Checkpoints (Phase 6 Incremental Tests)
Use these as pass/fail gates while implementing each sub-phase:

| # | Checkpoint | Phase | Status |
|---|------------|-------|--------|
| 1 | Backend brings up a Wayland socket | 6.1 | ✅ Done |
| 2 | `weston-simple-egl` or `weston-terminal` connects and creates a surface | 6.2 | ⬜ Pending |
| 3 | Window appears in Axiom layout and is tracked in `WindowManager` | 6.2 | ⬜ Pending |
| 4 | Mouse/keyboard events are routed and keybindings still work | 6.3 | ⬜ Pending |
| 5 | The client buffer is visible in the renderer | 6.4 | ⬜ Pending |

---

## 🛠️ Immediate Next Steps (Continuation Plan)

1.  **Window Mapping**: Implement `XdgShellHandler` in `smithay_backend_real.rs` to call `WindowManager::create_window`.
2.  **Buffer Access**: Implement `shm` buffer access to get pixel data for rendering.
3.  **Render Loop**: Update `AxiomRenderer` to accept `Smithay` surfaces instead of dummy data.
