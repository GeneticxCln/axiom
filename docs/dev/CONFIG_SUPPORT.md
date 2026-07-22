# Configuration Support Matrix

This document tracks whether each parsed configuration field is:
- **Applied** — actively affects runtime behavior today
- **Partially applied** — affects some runtime paths but not all intended behavior
- **Accepted but not applied** — validated and stored, but not yet wired into live runtime behavior

Axiom is still in alpha, so some fields exist ahead of full implementation.

## Workspace

| Field | Status | Notes |
|---|---|---|
| `workspace.scroll_speed` | Applied | Used in workspace navigation / momentum behavior and IPC config mutation path |
| `workspace.infinite_scroll` | Partially applied | Parsed and exposed; behavior is scaffolded but not a fully distinct bounded-workspace mode |
| `workspace.auto_scroll` | Accepted but not applied | Stored/validated only |
| `workspace.workspace_width` | Applied | Used by workspace layout calculation |
| `workspace.gaps` | Applied | Used by workspace tiling/layout |
| `workspace.smooth_scrolling` | Accepted but not applied | Scroll animation system exists, but this flag is not currently used as a hard runtime switch |
| `workspace.momentum_friction` | Applied | Used by momentum scrolling physics |
| `workspace.momentum_min_velocity` | Applied | Used by momentum scrolling stop threshold |
| `workspace.snap_threshold_px` | Applied | Used by momentum snapping |

## Effects

All effects config fields are accepted and stored by the parser, but the
`effects/` module has been removed. None of these fields have any runtime
effect on compositor behavior.

## Window

| Field | Status | Notes |
|---|---|---|
| `window.placement` | Accepted but not applied | Stored/validated only |
| `window.focus_follows_mouse` | Applied | Pointer motion can now move keyboard focus to the hovered window |
| `window.border_width` | Applied | Propagated into renderer border-width state |
| `window.active_border_color` | Partially applied | Used by decoration theme state; visible live decoration rendering still incomplete |
| `window.inactive_border_color` | Partially applied | Used by decoration theme state; visible live decoration rendering still incomplete |
| `window.gap` | Accepted but not applied | Deprecated in code comments; layout uses `workspace.gaps` |
| `window.default_layout` | Accepted but not applied | Stored/validated only |

## Input

| Field | Status | Notes |
|---|---|---|
| `input.keyboard_repeat_delay` | Applied | Wired into Smithay seat keyboard repeat info |
| `input.keyboard_repeat_rate` | Applied | Wired into Smithay seat keyboard repeat info |
| `input.mouse_accel` | Accepted but not applied | Stored/validated only |
| `input.touchpad_tap` | Accepted but not applied | Stored/validated only |
| `input.natural_scrolling` | Accepted but not applied | Stored/validated only |

## Bindings

| Field | Status | Notes |
|---|---|---|
| `bindings.scroll_left` | Applied | InputManager |
| `bindings.scroll_right` | Applied | InputManager |
| `bindings.move_window_left` | Applied | InputManager/backend action dispatch |
| `bindings.move_window_right` | Applied | InputManager/backend action dispatch |
| `bindings.close_window` | Applied | InputManager/backend action dispatch |
| `bindings.toggle_fullscreen` | Applied | InputManager/backend action dispatch |
| `bindings.toggle_floating` | Applied | InputManager/backend action dispatch |
| `bindings.toggle_minimize` | Applied | InputManager/backend action dispatch; semantics still alpha |
| `bindings.launch_terminal` | Applied | Spawns configured default command path in backend logic |
| `bindings.launch_launcher` | Applied | Spawns configured default command path in backend logic |
| `bindings.quit` | Applied | Runtime quit action |
| `bindings.mouse_back` | Applied | InputManager mouse binding parser |
| `bindings.mouse_forward` | Applied | InputManager mouse binding parser |
| `bindings.mouse_middle` | Applied | InputManager mouse binding parser |

## Backend

| Field | Status | Notes |
|---|---|---|
| `backend.kind` | Applied | Selects `winit` / `noop` |

## Feature gates

| Field | Status | Notes |
|---|---|---|
| `features.enable_minimize` | Applied | Controls minimize button behavior and feature exposure |
| `features.enable_xdg_decoration_protocol` | Partially applied | Can register protocol global, but live compositor output still does not claim visible SSD rendering |

## General

| Field | Status | Notes |
|---|---|---|
| `general.debug` | Accepted but not applied | CLI `--debug` currently controls logging; config value is not yet used to initialize logger |
| `general.max_fps` | Applied | Used by compositor tick pacing |
| `general.vsync` | Accepted but not applied | Stored/validated only |
