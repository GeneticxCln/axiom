# Render Architecture

## Decision

**Axiom's primary compositor architecture is WGPU-first.**

That means:
- **WGPU owns frame composition** for windows and effects.
- **GL is a transitional presentation shim**, not the long-term source of compositor behavior.
- New rendering features should be implemented against the **WGPU compositor path**, not added only to the legacy/raw GL path.

## Why this decision

The repository currently contains multiple rendering layers:
- Smithay/GLES integration
- raw GL blit helpers in `src/backend/mod.rs`
- WGPU off-screen compositor in `src/renderer/mod.rs`
- DRM/GBM plumbing in `src/backend/drm.rs`

Keeping all of them as equal long-term rendering paths would make the project harder to finish and maintain. The code already has the most structured composition/effects logic in the WGPU renderer, so that is the best place to converge.

## Current state

### What WGPU already does
- maintains renderer-owned window rectangles and textures
- composes textured windows into an off-screen target
- queues and dispatches shadow/blur passes
- supports headless composition/testing helpers

### What GL currently does
- provides the current nested presentation bridge in the winit path
- receives the composed frame after WGPU composition
- uploads/blits the final image to the active framebuffer
- reuses persistent upload-side state (for example the fullscreen blit texture) while the renderer reuses the GPU readback staging buffer on same-size frames to reduce bridge churn
- is now intentionally isolated behind backend bridge helpers rather than spread through multiple rendering paths

### Why this is transitional
The current nested path still relies on a costly flow:

```text
WGPU compose -> CPU readback -> GL texture upload -> GL fullscreen blit
```

This is acceptable as an alpha-stage bridge, but it is **not** the intended end state.

Recent bridge hardening has reduced some avoidable churn (for example, same-size
frames now reuse the GPU readback staging buffer instead of allocating a fresh
readback buffer every compose), but the architectural CPU roundtrip still exists.

## Target architecture

## 1. Composition owner: WGPU

The compositor should have one authoritative frame builder:
- window geometry
- texture composition
- effects application
- final frame assembly

That owner is WGPU.

## 2. Presentation should become thinner

Presentation should eventually be a thin backend-specific step:
- nested/windowed path: present the already-composed frame without full CPU roundtrip
- DRM/KMS path: feed compositor output into the standalone output path without building a separate second-class render architecture

## 3. Avoid feature duplication

Do **not** add the same effect logic independently to both:
- a WGPU composition path, and
- a separate GL-only feature path

If temporary GL work is needed, it should be treated as compatibility plumbing, not as the main rendering feature surface.

## Scope boundaries

### WGPU path owns
- full-frame composition
- placeholder/textured window composition semantics
- effect queue consumption
- renderer-side resource lifecycle

### Backend layer owns
- input/event processing
- Wayland/Smithay protocol handling
- output setup
- presentation scheduling
- temporary interop with the presentation target
- temporary policy decisions that avoid claiming rendering features not yet visible in the live output path (for example: current xdg-decoration negotiation remains CSD-first until visible SSD rendering lands)

### DRM layer owns
- device/output discovery
- modesetting / output lifecycle
- hotplug monitoring
- current standalone alpha presentation bridge (WGPU-composed frame -> CPU dumb-buffer scanout)
- eventual standalone present path integration

## Near-term rules for contributors

1. Prefer implementing new visual behavior in `src/renderer/` or `src/effects/`.
2. Treat raw GL code in `src/backend/mod.rs` as a compatibility bridge.
3. Do not add new long-term rendering features that exist only in the GL shim.
4. Keep backend and renderer responsibilities separate: backend orchestrates, renderer composes.

## Near-term migration goals

1. Keep nested `--windowed` mode usable while retaining the current bridge.
2. Reduce or eliminate the full-frame CPU readback in the common path.
3. Reuse the same compositor frame architecture for standalone DRM output.
4. Remove duplicated rendering responsibilities once the target present path is stable.

## Non-goals right now

This decision does **not** mean:
- the GL bridge disappears immediately
- the DRM path is already complete
- the compositor can already present directly from WGPU on every backend

It only means the project now has a documented architectural direction:

> **WGPU is the compositor; backend-specific presentation code is an implementation detail.**
