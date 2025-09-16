# Axiom To‑Do Summary

Generated: 2025-09-16 03:01 UTC

Scope
- Consolidated from explicit code comments (TODO/FIXME/HACK/TBD) and unchecked Markdown checkboxes found across the repository.
- Duplicates merged; each item lists source references so you can jump back to context.
- Does not include generic mentions of “optimize” unless explicitly marked as TODO/HACK/TBD.

Legend
- [ ] = Pending task
- [x] = Completed
- [~] = Partially completed
- Sources: path:line[, path:line, ...]

## 1) Core Wayland/Smithay integration
- [ ] Set up Smithay compositor state, display, and event loop (initialization)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:22–24; PHASE_6_IMPLEMENTATION_PLAN.md:254
- [ ] Configure development/production backends (winit for dev, DRM for production)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:24–25
- [ ] Implement core Wayland globals: wl_compositor, wl_shm, wl_seat, wl_output  
  Sources: SMITHAY_INTEGRATION_PLAN.md:28–31
- [ ] Implement xdg_wm_base protocols: xdg_surface, xdg_toplevel, xdg_popup  
  Sources: SMITHAY_INTEGRATION_PLAN.md:41–44
- [ ] Map Smithay surfaces to internal Window structs; handle commits/damage; implement positioning; connect to WindowManager  
  Sources: SMITHAY_INTEGRATION_PLAN.md:35–38
- [ ] Integrate xdg_decoration: SSD/CSD negotiation and server‑side decorations  
  Sources: SMITHAY_INTEGRATION_PLAN.md:47–49
- [ ] Wire up input devices via Smithay: keyboard, pointer, touch; focus handling and “focus follows mouse” option  
  Sources: SMITHAY_INTEGRATION_PLAN.md:53–61
- [ ] Real winit backend initialization  
  Sources: src/experimental/smithay/smithay_backend_real.rs:123; src/experimental/smithay/smithay_backend_working.rs:77
- [ ] Initialize real Wayland display/protocols  
  Sources: src/experimental/smithay/smithay_backend_real.rs:133; src/experimental/smithay/smithay_backend_working.rs:87; src/experimental/smithay/smithay_enhanced.rs:16,23,79
- [ ] Process real Wayland events  
  Sources: src/experimental/smithay/smithay_backend_real.rs:143; src/experimental/smithay/smithay_backend_working.rs:97; src/experimental/smithay/smithay_enhanced.rs:197
- [ ] Cleanup Smithay resources and handle window close/popups  
  Sources: src/experimental/smithay/smithay_backend_real.rs:182; src/experimental/smithay/smithay_backend_real_minimal.rs:407; src/experimental/smithay/smithay_backend_production.rs:510
- [ ] Deep compositor integration with Smithay (future)  
  Sources: src/compositor.rs:495
- [ ] XWayland integration working  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:194; docs/PROJECT_PLAN.md:49; PHASE5_ROADMAP.md:47
- [ ] Multiple concurrent clients supported  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:192

## 2) Rendering and effects pipeline
- [ ] Initialize renderer (OpenGL/Vulkan) and GPU context  
  Sources: SMITHAY_INTEGRATION_PLAN.md:25,65; src/effects/mod.rs:814
- [ ] Texture/buffer management for surfaces  
  Sources: SMITHAY_INTEGRATION_PLAN.md:66
- [ ] Shader pipeline for effects (blur, shadows, rounded corners, opacity/scale transforms)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:67,70–73
- [ ] Damage tracking, partial redraws, frame scheduling, adaptive sync  
  Sources: SMITHAY_INTEGRATION_PLAN.md:76–79
- [ ] Implement real surface rendering in production backend  
  Sources: src/experimental/smithay/smithay_backend_production.rs:302
- [ ] Real GPU acceleration with visual effects  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:178
- [ ] GPU memory management: efficient texture/buffer pooling  
  Sources: PHASE5_ROADMAP.md:30
- [ ] Re‑enable deferred GPU‑dependent code paths (effects init)  
  Sources: src/effects/mod.rs:814
- [ ] Replace hacky animation state tracking with robust design  
  Sources: src/effects/animations.rs:470
- [ ] All planned effects working with real windows  
  Sources: TRANSFORMATION_TO_REAL_COMPOSITOR.md:346

## 3) Input and gestures
- [ ] Add mouse button bindings  
  Sources: src/input/mod.rs:245
- [ ] Implement smooth scrolling with pan gestures  
  Sources: src/input/mod.rs:298
- [ ] Implement workspace overview with pinch gesture  
  Sources: src/input/mod.rs:302
- [ ] Real keyboard and mouse input via Smithay  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:187; SMITHAY_INTEGRATION_PLAN.md:53–61
- [ ] Hardware input device integration  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:268
- [ ] Input latency measurement  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:217
- [ ] Shortcuts trigger workspace actions with real apps  
  Sources: TRANSFORMATION_TO_REAL_COMPOSITOR.md:337

## 4) Window management and UX
- [ ] Basic window management: move, resize, close  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:188; REAL_COMPOSITOR_PLAN.md:119; TRANSFORMATION_TO_REAL_COMPOSITOR.md:331
- [ ] Window focus and decoration management  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:211
- [ ] Popups & dialogs: proper modal handling  
  Sources: PHASE5_ROADMAP.md:51; src/experimental/smithay/smithay_backend_production.rs:510
- [ ] Window stacking (Z‑order)  
  Sources: PHASE5_ROADMAP.md:106
- [ ] Tiling layouts  
  Sources: PHASE5_ROADMAP.md:107
- [ ] Virtual desktops alongside scrolling workspaces  
  Sources: PHASE5_ROADMAP.md:105
- [ ] Drag & Drop (inter‑application data transfer)  
  Sources: PHASE5_ROADMAP.md:52
- [ ] Clipboard protocol implementation  
  Sources: PHASE5_ROADMAP.md:53; SMITHAY_INTEGRATION_PLAN.md:91
- [ ] Window rules: per‑app configuration and placement  
  Sources: PHASE5_ROADMAP.md:50
- [ ] Scrollable workspaces with real windows  
  Sources: REAL_COMPOSITOR_PLAN.md:122; TRANSFORMATION_TO_REAL_COMPOSITOR.md:334
- [ ] Basic effects (shadows, borders) visible with real windows  
  Sources: REAL_COMPOSITOR_PLAN.md:123
- [ ] Window animations functional with real windows  
  Sources: REAL_COMPOSITOR_PLAN.md:124
- [ ] Workspace thumbnails/overview mode  
  Sources: docs/PROJECT_PLAN.md:88

## 5) IPC, telemetry, and AI
- [x] Implement real CPU monitoring  
  Sources: src/ipc_enhanced.rs:309; Implemented in IPC HealthCheck via /proc in src/ipc/mod.rs
- [x] Implement real memory monitoring  
  Sources: src/ipc_enhanced.rs:316; Implemented in IPC HealthCheck via /proc in src/ipc/mod.rs
- [~] Implement real GPU monitoring  
  Sources: src/ipc_enhanced.rs:323; Implemented basic AMD sysfs sampler; optional NVML support via feature `gpu-nvml`
- [ ] Implement pattern analysis for optimization  
  Sources: src/ipc_enhanced.rs:521
- [x] Return actual configuration values in IPC queries  
  Sources: src/ipc/mod.rs:318
- [x] Execute workspace commands via IPC  
  Sources: src/ipc/mod.rs:343; forwarded to compositor runtime and handled in src/compositor.rs
- [x] Apply effects changes via IPC  
  Sources: src/ipc/mod.rs:355; runtime wired to EffectsEngine
- [~] Provide real metrics in IPC responses; generate performance reports  
  Sources: src/ipc/mod.rs:364,377; IMPLEMENTATION_STATUS_PHASE5.md:180 (HealthCheck returns CPU/mem/GPU; report generation pending)
- [x] Fill GPU metric (currently TBD placeholder)  
  Sources: src/ipc/mod.rs:536; now provided by sysfs/NVML sampler
- [x] Live SetConfig for workspace.scroll_speed with validation  
  Sources: src/compositor.rs (runtime command), src/workspace/mod.rs (setter)
- [ ] Real‑time performance monitoring; adaptive effects; usage pattern learning  
  Sources: DEVELOPMENT_STATUS.md:39–41; PHASE5_ROADMAP.md:58–61,70–73

## 6) Protocols and application compatibility
- [ ] Full wl_surface/xdg_shell support with conformance testing  
  Sources: PHASE5_ROADMAP.md:38; IMPLEMENTATION_STATUS_PHASE5.md:198
- [ ] wlr‑layer‑shell (panels/overlays)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:83
- [ ] wp‑viewporter (scaling)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:84
- [ ] wp‑presentation‑time (frame timing)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:85
- [ ] zwp‑linux‑dmabuf (zero‑copy buffers)  
  Sources: SMITHAY_INTEGRATION_PLAN.md:86
- [ ] GTK application compatibility  
  Sources: PHASE5_ROADMAP.md:44
- [ ] Qt application compatibility  
  Sources: PHASE5_ROADMAP.md:45
- [ ] Electron apps (VSCode, Discord, browsers)  
  Sources: PHASE5_ROADMAP.md:46
- [ ] Gaming (Steam, native, XWayland gaming)  
  Sources: PHASE5_ROADMAP.md:47
- [ ] XWayland compatibility and parity goals  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:194; docs/PROJECT_PLAN.md:217

## 7) Performance, stability, and metrics targets
- [ ] Frame rate with multiple applications; <16ms frame times under effects  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:214; PHASE5_ROADMAP.md:141
- [ ] Stable 60fps scrolling with 10+ windows  
  Sources: docs/PROJECT_PLAN.md:214
- [ ] Memory usage under load; <200MB memory footprint target  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:215; docs/PROJECT_PLAN.md:216
- [ ] Animation smoothness with real rendering  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:216
- [ ] Input latency measurement and improvement  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:217
- [ ] Stability: 99.5% uptime in 24‑hour stress tests; stable under load  
  Sources: PHASE5_ROADMAP.md:139; IMPLEMENTATION_STATUS_PHASE5.md:199
- [ ] Compatibility: 95% of common applications work correctly  
  Sources: PHASE5_ROADMAP.md:140
- [ ] Performance regression prevention  
  Sources: DEVELOPMENT_STATUS.md:23; scripts/phase5_kickoff.sh:230
- [ ] CPU profiling; battery optimization  
  Sources: PHASE5_ROADMAP.md:31–33
- [ ] Memory management and crash recovery  
  Sources: PHASE5_ROADMAP.md:26–27

## 8) Testing and QA
- [ ] Unit tests to ≥80% coverage for core modules  
  Sources: PHASE5_ROADMAP.md:9–13
- [ ] Integration tests: compositor end‑to‑end, multi‑client window management, IPC robustness, performance under load  
  Sources: PHASE5_ROADMAP.md:14–17
- [ ] Property‑based tests: configuration edge cases, animation boundaries  
  Sources: PHASE5_ROADMAP.md:18–20
- [ ] Memory leak detection  
  Sources: PHASE5_ROADMAP.md:21; scripts/phase5_kickoff.sh:229
- [ ] Protocol conformance test suite  
  Sources: IMPLEMENTATION_STATUS_PHASE5.md:198

## 9) Packaging, CI/CD, and distribution
- [ ] Arch Linux AUR package  
  Sources: DEVELOPMENT_STATUS.md:44; PHASE5_ROADMAP.md:78; scripts/phase5_kickoff.sh:246
- [ ] Ubuntu/Debian .deb package  
  Sources: DEVELOPMENT_STATUS.md:45; PHASE5_ROADMAP.md:79; scripts/phase5_kickoff.sh:247
- [ ] Fedora RPM  
  Sources: PHASE5_ROADMAP.md:80
- [ ] NixOS package  
  Sources: PHASE5_ROADMAP.md:81
- [ ] Flatpak  
  Sources: PHASE5_ROADMAP.md:82
- [ ] Install script for dependencies  
  Sources: PHASE5_ROADMAP.md:85
- [ ] Configuration wizard  
  Sources: PHASE5_ROADMAP.md:86
- [ ] Desktop integration: .desktop files, session manager  
  Sources: PHASE5_ROADMAP.md:87
- [ ] Documentation: comprehensive user/admin guides  
  Sources: PHASE5_ROADMAP.md:88
- [ ] Versioning strategy (semver, stability tiers)  
  Sources: PHASE5_ROADMAP.md:91
- [ ] CI/CD pipeline setup and releases  
  Sources: DEVELOPMENT_STATUS.md:46; PHASE5_ROADMAP.md:92; scripts/phase5_kickoff.sh:248
- [ ] Security updates process; backports; release automation  
  Sources: PHASE5_ROADMAP.md:93–94; DEVELOPMENT_STATUS.md:47; scripts/phase5_kickoff.sh:249

## 10) Milestones and releases
- [ ] Week 1: Smithay backend compiles and initializes  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:254
- [ ] Week 2: weston‑terminal launches successfully  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:255; PRODUCTION_ROADMAP.md:194; REAL_COMPOSITOR_PLAN.md:116; TRANSFORMATION_TO_REAL_COMPOSITOR.md:328
- [ ] Week 3: Firefox runs with scrollable workspaces  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:256; PRODUCTION_ROADMAP.md:195; REAL_COMPOSITOR_PLAN.md:128; TRANSFORMATION_TO_REAL_COMPOSITOR.md:340
- [ ] Week 4: All visual effects work with real applications  
  Sources: PHASE_6_IMPLEMENTATION_PLAN.md:257; PRODUCTION_ROADMAP.md:196; TRANSFORMATION_TO_REAL_COMPOSITOR.md:346
- [ ] Pass application compatibility test suite; ready for daily use; public beta  
  Sources: PRODUCTION_ROADMAP.md:197–199; TRANSFORMATION_TO_REAL_COMPOSITOR.md:349
- [ ] Community and adoption goals (stars, contributors, distro availability, outreach)  
  Sources: docs/PROJECT_PLAN.md:220–223,234–237

## 11) Miscellaneous and housekeeping
- [ ] Add Cargo features when compositor dependencies are ready  
  Sources: Cargo.toml:106
- [ ] Handle custom commands in compositor  
  Sources: src/compositor.rs:244
- [x] Fill GPU metric placeholder in IPC  
  Sources: src/ipc/mod.rs:536; now provided by sysfs/NVML sampler
- [ ] Pull workspace_scroll_speed metric from workspace manager  
  Sources: src/ipc_metrics.rs:183

---

Regeneration
- This file was generated by scanning the repository. As items are completed upstream, check them off here and/or update the source documents. Re‑run the scan to refresh sources and add new items.
