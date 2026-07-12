# Axiom Master Development Plan

**Status:** Active
**Current Phase:** Repository stabilization and alpha hardening
**Last Updated:** 2026-07-11

---

## Executive Summary

Axiom has a strong prototype codebase with real compositor infrastructure, but it is **not yet a production-ready compositor**.

### Current reality
- The **nested / windowed path** is the most complete execution mode.
- Core logic for configuration, workspaces, IPC, rendering infrastructure, and input exists.
- The standalone DRM/KMS path now has an early compositor output path using WGPU-composed frames copied into CPU dumb-buffer scanout, but it still needs validation, optimization, and multi-output hardening.
- Documentation, packaging, and status claims previously overstated project maturity; this document is now the source of truth.

### Immediate objective
Stabilize the repository around an **honest alpha target**:
1. fix correctness/documentation issues,
2. make nested mode the first supported experience,
3. finish the standalone DRM path afterward.

---

## Completed Foundations

### Architecture and core systems
- Modular Rust codebase with compositor, backend, renderer, config, workspace, effects, IPC, and XWayland modules
- Tokio-driven orchestration layer
- TOML configuration loading, validation, and atomic save
- Unix-socket IPC with structured JSON messages and basic peer verification

### Workspace and window logic
- Scrollable workspace/tape model
- Dynamic column management
- Minimize/floating/fullscreen state tracking
- Layout calculation and pointer hit-testing

### Rendering/effects infrastructure
- WGPU renderer for off-screen composition and effect passes
- GL presentation path for current nested rendering flow
- Animation/effects state engine with spring-based transitions

### Backend groundwork
- Smithay-based Wayland compositor backend
- Winit nested backend
- DRM/libinput/udev scaffolding
- XWayland/XWM integration groundwork

---

## Current Project Assessment

### Working well
- Config system
- Workspace logic
- Basic window lifecycle tracking
- IPC server foundation
- Nested development mode
- A substantial logic/integration test suite
- Real nested smoke coverage using an actual Wayland client in CI

### Partially complete
- Real Wayland client lifecycle and metadata propagation
- Effects integration into the live render path
- Decorations
- XWayland compatibility
- Multi-monitor handling
- Standalone DRM/KMS execution path

### Missing or incomplete
- Release-ready packaging
- Session assets and polished installation flow
- Fractional scaling
- Unified render architecture

### Main technical blockers
1. Transitional render architecture still includes a WGPU compose + GL presentation bridge
2. Incomplete DRM/KMS compositor path and limited committed real-hardware validation coverage
3. Multi-monitor/layout correctness gaps
4. Broader XWayland compatibility still needs more end-to-end validation beyond the current lifecycle/metadata/clipboard paths
5. Documentation and packaging drift

### Render direction decision
Axiom is now explicitly **WGPU-first**:
- WGPU owns compositor frame composition and effects
- GL is treated as a temporary presentation bridge in the nested path
- standalone DRM work should converge on the same compositor frame architecture rather than grow a second independent render feature path

See `docs/dev/RENDER_ARCHITECTURE.md` for the design note.

---

## Development Phases

## Phase 1 — Immediate fixes
Focus on correctness, honesty, and obvious lifecycle bugs.

### Goals
- Correct repo status/documentation
- Fix packaging metadata and missing assets
- Reconcile IPC socket behavior across code/docs/clients
- Fix focused-window action bugs
- Ensure window destroy cleans all subsystems

### Exit criteria
- No major docs/status contradictions remain
- Packaging references valid files
- Helper IPC clients can find the actual socket path
- Window destroy no longer leaks renderer/effects state

---

## Phase 2 — Stable nested compositor
Treat `--windowed` as the first supported alpha target.

### Goals
- Choose and document the intended render architecture
- Reduce or isolate the WGPU → CPU → GL roundtrip in the main path
- Integrate real decoration rendering or explicitly reduce SSD claims
- Align runtime config and Lazy UI protocol behavior with what is actually supported

### Exit criteria
- Nested mode is the recommended and documented alpha target
- Basic nested compositor smoke flow is documented, automated, and testable
- Known limitations are documented for alpha users
- Major protocol/runtime mismatches are resolved

---

## Phase 3 — Standalone DRM alpha
Finish the real session-compositor path.

### Goals
- Complete the compositor output/render path in DRM mode
- Validate output hotplug and multi-monitor state synchronization
- Make layout and coordinate handling correct across outputs
- Improve HiDPI behavior, including early fractional scale support

### Exit criteria
- DRM mode can render mapped client content reliably enough for alpha testing
- Output add/remove flows are documented and validated

---

## Phase 4 — Testing, optimization, and release prep
Turn the alpha into something users can evaluate repeatably.

### Goals
- Expand real client smoke coverage beyond the current nested `weston-terminal` path
- Expand XWayland validation
- Refactor oversized modules
- Add release/session/package assets
- Prepare first tagged alpha release

### Exit criteria
- Release checklist exists (`docs/dev/RELEASE_CHECKLIST.md`)
- Known limitations are documented
- Packaging/session assets exist
- First alpha release can be cut honestly

---

## Task Ordering

The current execution order is:
1. docs/status correction
2. packaging cleanup
3. IPC socket consistency
4. input/lifecycle bug fixes
5. nested-mode stabilization
6. DRM completion
7. test/release hardening

---

## Decision Note

Axiom should **not** present itself as a production-ready compositor until:
- one compositor mode is clearly supportable end-to-end,
- docs match reality,
- packaging/session assets exist,
- and real-client testing is in place.

Until then, the right positioning is:

> **alpha compositor prototype with a strong nested development path**
