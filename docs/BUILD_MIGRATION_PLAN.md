# Axiom Build Recovery and Migration Plan

Author: Agent Mode (gpt-5)
Date: 2025-08-15

Summary
- Current status: The project is real and substantial, but it does not compile due to API drift in Smithay and wgpu.
- Goal: Get to a repeatable green build, runnable demos, and a path to production features.
- Strategy: Prefer a forward migration to current dependencies. Provide a fallback plan to pin older versions if blockers arise.

Table of Contents
1. Objectives
2. Current State and Key Breakages
3. Decision: Pin vs. Migrate
4. Plan A (Recommended): Migrate to current APIs
   4.1 wgpu 0.19 migration steps
   4.2 Smithay 0.3 + winit integration steps
   4.3 Renderer and effects adjustments
   4.4 IPC, input, workspace, and window glue
   4.5 Iterative compile strategy
5. Plan B (Fallback): Pin older versions for quick green
6. Testing and Verification
7. Run Instructions (dev and demos)
8. Quality: Lint, format, docs, and CI
9. Risks, Rollback, and Branching Strategy
10. Timeline and Milestones

1) Objectives
- Restore a clean build (cargo build) on Linux (CachyOS/Arch-like) using stable Rust.
- Restore unit/integration tests and demo runs.
- Keep the codebase aligned with current Smithay and wgpu where feasible.
- Minimize intrusive rewrites; keep architectural intent intact.

2) Current State and Key Breakages
Observed compile errors:
- Smithay API imports in src/smithay_backend.rs do not match smithay = "0.3.0"
  - Missing/renamed items: backend::winit::WinitEventLoop, smithay::desktop, wayland reexports, EventLoopBuilderExtUnix path, CompositorClientState/Handler/State types.
- wgpu device creation in src/renderer/mod.rs expects DeviceDescriptor { features, limits }, but wgpu 0.19 uses { required_features, required_limits }.
- A number of warnings (non-blocking).

3) Decision: Pin vs. Migrate
- Recommended: Migrate forward to Smithay 0.3 and wgpu 0.19 (what Cargo.toml already specifies). This avoids dependency fragmentation and gives access to current features and fixes.
- Fallback: Temporarily pin to older versions (e.g., wgpu 0.18, smithay 0.2.x) to get a quick green build if migration blocks progress.

4) Plan A (Recommended): Migrate to current APIs

4.1 wgpu 0.19 migration steps (src/renderer/mod.rs)
- Update DeviceDescriptor usage:
  - Replace:
    DeviceDescriptor { label, features, limits }
  - With:
    DeviceDescriptor { label, required_features, required_limits }
  - Use required_features: wgpu::Features::empty() unless you actually need features.
  - Use required_limits: wgpu::Limits::default() or adapter.limits(); pick the minimal needed set.
- Verify SurfaceConfiguration fields are correct for wgpu 0.19 (present_mode, alpha_mode, view_formats, usage, format, width, height).
- Check queue/device creation code for API signature changes.
- Ensure RenderPassDescriptor, Pipeline layouts, and Texture usages align with 0.19 (names and enums are largely stable, but confirm).

4.2 Smithay 0.3 + winit integration steps (src/smithay_backend.rs)
- Do not rely on deep reexports. Prefer direct crates:
  - wayland_server::{Display, DisplayHandle, ...}
  - winit::event_loop::{EventLoop, EventLoopBuilder}
  - winit::platform::unix::EventLoopExtUnix (name changed; create_new is still via trait)
- The smithay::desktop module moved to a separate crate (smithay-desktop) in some versions. Options:
  - If you used Space/Window purely for tracking, replace with your own structs in window manager, or add smithay-desktop = "=0.3" if compatible and refactor imports to smithay_desktop::Space, etc.
  - If adding smithay-desktop, validate compatibility with smithay 0.3.
- Compositor traits:
  - smithay::wayland::compositor no longer exports CompositorClientState/CompositorHandler/CompositorState in the same way.
  - Strategy: Start minimal. Implement just what you need: surface registration, basic event dispatch. Use smithay examples as a reference for 0.3 (names: compositor::CompositorState still exists, but the handler traits may be organized differently; consider using smithay::reexports::wayland_server directly for Display/Client).
- Event loop:
  - Use calloop for integration (already in deps). Create a calloop::EventLoop, integrate the Wayland display with a Source.
  - If you were using Winit-based backend helpers, consider either:
    - Winit front buffer for dev windowing, or
    - DRM/udev path for real compositor (longer path).
  - For Phase 5, keep windowed dev mode via Winit if that’s what demo paths rely on.
- Action items in file:
  - Remove or replace imports that no longer resolve.
  - Update creation paths for Display, event loop integration, and socket export.

4.3 Renderer and effects adjustments
- Verify the creation of pipelines and bind groups for blur/effects is compatible with wgpu 0.19.
- Confirm shaders module paths and bytemuck derive usages are correct.
- Resolve warnings by pruning unused imports and variables to keep code clean.

4.4 IPC, input, workspace, and window glue
- These modules compiled previously; after Smithay and wgpu changes they should largely remain unaffected.
- Ensure input::InputEvent simulation still runs in windowed/demo modes until real input is wired.

4.5 Iterative compile strategy
- Work in small loops:
  1) Fix wgpu DeviceDescriptor compile errors first (fastest win).
  2) Tackle smithay_backend.rs imports and minimal run loop wiring.
  3) cargo build after each logical set of changes.
  4) Once it compiles, run unit tests and demos.

5) Plan B (Fallback): Pin older versions for quick green
- In Cargo.toml:
  - Set wgpu = "=0.18.0"
  - Try smithay = "=0.2.0" or the latest 0.2.x that exists. If smithay-desktop was used, add a matching version.
- Run cargo update -p wgpu --precise 0.18.0, etc., if needed.
- Rebuild. If transitive conflicts occur, prefer Plan A.

6) Testing and Verification
- Commands (from WARP.md and additions):
  - cargo build
  - cargo test --all-targets
  - cargo test -- --nocapture
  - cargo clippy --all-targets --all-features
  - cargo fmt
  - Benchmarks: cargo bench
  - Docs: cargo doc --open
- IPC test: python3 test_ipc.py (after compositor runs in a separate shell).
- Demos:
  - ./target/debug/axiom --debug --windowed --demo
  - ./target/debug/axiom --debug --windowed --effects-demo
  - Both: ./target/debug/axiom --debug --windowed --demo --effects-demo

7) Run Instructions (dev and demos)
- Dev build: cargo build
- Run with debug logging and windowed mode:
  ./target/debug/axiom --debug --windowed
- Production (needs root/permissions; after real backend is fully wired):
  sudo ./target/release/axiom
- Config override (optional):
  ./target/debug/axiom --config ~/.config/axiom/axiom.toml --debug --windowed

8) Quality: Lint, format, docs, and CI
- Formatting: cargo fmt
- Linting: cargo clippy --all-targets --all-features
- Security audit: cargo audit
- Dependency status: cargo outdated
- Consider adding a minimal CI (GitHub Actions) with steps: build, test, clippy, fmt-check.

9) Risks, Rollback, and Branching Strategy
- Risks: Smithay API moves may require non-trivial rewrites; winit integration may be touchy.
- Rollback: Keep a branch for migration (feature/migrate-smithay-wgpu). If blocked, branch feature/pin-legacy-deps to land a green build via Plan B.
- Commit in small increments; keep cargo build green as you proceed.

10) Timeline and Milestones
- Day 0.5: Fix wgpu device descriptor and rebuild renderer (Plan A 4.1). Outcome: cargo build gets past renderer errors.
- Day 1–2: Smithay/winit import and event loop integration fixes (Plan A 4.2). Outcome: compiles end-to-end.
- Day 3: Run unit/integration tests, fix failures, run demos in windowed mode (Plan A 4.5 & 6).
- Day 4: Clippy cleanups, docs update, optional CI.

Appendix A: Concrete code change examples
- wgpu 0.19 DeviceDescriptor example:
  let device_descriptor = wgpu::DeviceDescriptor {
      label: Some("Axiom Device"),
      required_features: wgpu::Features::empty(),
      required_limits: wgpu::Limits::default(),
  };

- winit Unix builder trait path:
  use winit::platform::unix::EventLoopExtUnix; // instead of platform::unix::EventLoopBuilderExtUnix

- Prefer direct wayland_server imports:
  use wayland_server::{Display, DisplayHandle};

Appendix B: Commands quick reference
- Build (debug): cargo build
- Build (release): cargo build --release
- Run (dev): ./target/debug/axiom --debug --windowed
- Demos: ./target/debug/axiom --debug --windowed --demo --effects-demo
- Tests: cargo test --all-targets
- IPC test: python3 test_ipc.py
- Format: cargo fmt
- Lint: cargo clippy --all-targets --all-features
- Audit: cargo audit
- Outdated: cargo outdated

