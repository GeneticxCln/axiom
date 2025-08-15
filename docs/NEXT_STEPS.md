# Phase 5 Next Steps Plan

This is the working plan to complete Phase 5 (Production Readiness).

Phase 1 — Metrics and IPC (quick wins, 1–2 hrs)
- Replace placeholders in IPC metrics:
  - CPU/memory: use /proc sampling (or sysinfo) to compute usage periodically
  - FPS/frame time: already present; keep broadcasting per frame
  - Active windows: replace approximation with window_manager count
- Add a simple rate limiter so we don’t broadcast every frame when unchanged

Phase 2 — Lint and hygiene (2–4 hrs)
- Fix clippy “absurd extreme comparisons” in tests (e.g., assert!(u32 >= 0))
- Remove unused imports/vars in effects, renderer, xwayland, wayland_protocols
- Replace trivial or_insert_with with or_default; remove assert!(true)
- Run cargo clippy --all-targets --all-features to get to low/no warnings

Phase 3 — Real compositor path build + smoke test (1–2 hrs)
- Build: cargo build --features real-compositor
- Dev-run enhanced backend (windowed): ./target/debug/axiom --debug --windowed --real_smithay
- Validate: WAYLAND_DISPLAY set, socket created, event loop runs, IPC connects

Phase 4 — Protocol and multi-output hardening (0.5–1 day)
- wayland_protocols.rs: align with Smithay helpers or remove dead code
- Multi-output: add config schema, validation, and expose arrangement modes
- Real input/window modules: guard with feature flags; document Smithay API gaps

Phase 5 — Testing and CI (0.5–1 day)
- Add integration tests:
  - IPC health and performance report requests
  - Metrics broadcast observable via a test client
- Wire scripts/test.sh into CI; run cargo test, clippy, fmt check
- Optional: add GH Actions workflow for PRs

Nice-to-haves (later)
- Feed compositor/effects stats into ipc_metrics.rs collector (averages/peaks/health)
- Hook renderer timing into metrics (wgpu queue timings)
- Keep Arch PKGBUILD up-to-date with feature flags

