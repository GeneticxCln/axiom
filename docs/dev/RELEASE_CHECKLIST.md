# Axiom Alpha Release Checklist

Use this checklist before cutting an alpha release.

## 1. Scope and honesty
- [ ] README status still describes Axiom as an **alpha compositor prototype**
- [ ] Known limitations in `docs/user/LIMITATIONS.md` are current
- [ ] Release notes clearly distinguish:
  - recommended nested/windowed path
  - early/in-progress standalone DRM path
  - current live decoration policy (CSD-first until visible SSD rendering exists)
  - any major regressions or unsupported features

## 2. Build and test gates
- [ ] `cargo fmt -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --lib --all-features -- --test-threads=1`
- [ ] `cargo test --test integration_tests --all-features`
- [ ] `bash ./scripts/coverage.sh unit`
- [ ] nested smoke test passes with a real client:
  - `xvfb-run -a bash ./scripts/nested_smoke_test.sh ./target/debug/axiom`
- [ ] benchmark suite runs cleanly:
  - `bash ./scripts/benchmark.sh run`
- [ ] valgrind-based memory safety checks run cleanly:
  - `bash ./scripts/memory_profile.sh valgrind-tests`
- [ ] XWayland validation runs cleanly:
  - `bash ./scripts/check_xwayland.sh all ./target/debug/axiom`
  - includes lifecycle, real-client, metadata, compositor-side XWM wiring, and end-to-end X11-in-Axiom smoke checks
- [ ] security checks run cleanly:
  - `bash ./scripts/check_security.sh all`
- [ ] CI workflow passes on the release commit

## 3. Runtime spot checks
- [ ] `cargo run -- --windowed --debug` starts successfully
- [ ] a real Wayland client maps in nested mode (`weston-terminal` or equivalent)
- [ ] client close removes the mapped window cleanly
- [ ] IPC socket path matches the documented behavior:
  - preferred: `$XDG_RUNTIME_DIR/axiom/axiom.sock`
  - fallback: `/tmp/axiom-<pid>/axiom-lazy-ui.sock`
- [ ] helper clients still work against the current IPC schema
- [ ] if the release claims DRM/KMS improvements, update `docs/dev/DRM_HARDWARE_VALIDATION.md`
  and attach at least one fresh real-hardware report from `scripts/drm_validation_report.sh report`

## 4. Packaging assets
- [ ] `bash ./scripts/check_packaging_assets.sh`
- [ ] `bash ./scripts/build_arch_package.sh run`
- [ ] `packaging/arch/PKGBUILD` stages a valid install tree against the current source
- [ ] staged installed artifacts pass smoke checks:
  - staged `axiom --help` runs
  - staged `axiom-session` rejects missing `XDG_RUNTIME_DIR`
  - staged `axiom-session` prefers user config when present
  - staged `axiom-session` falls back cleanly when no user config exists
- [ ] nested desktop entry is installed and references a valid icon
- [ ] Wayland session entry launches `axiom-session`
- [ ] `packaging/axiom-session` starts the DRM path with config discovery intact
- [ ] `assets/logo.svg` is installed as the package icon asset

## 5. Documentation sync
- [ ] `README.md` quick-start commands still work
- [ ] `docs/user/INSTALL.md` and `docs/user/RUNNING.md` match current behavior
- [ ] `docs/dev/SETUP.md` matches current smoke-test flow
- [ ] `MASTER_DEVELOPMENT_PLAN.md` still reflects the current phase and priorities

## 6. Release publication
- [ ] `bash ./scripts/release_prep.sh all vX.Y.Z-alpha.N`
- [ ] `docs/dev/RELEASE_NOTES_TEMPLATE.md` still matches the current alpha release posture
- [ ] version number and changelog/release notes are updated
- [ ] git tag is created for the alpha release
- [ ] GitHub release notes include:
  - summary of user-visible changes
  - recommended evaluation path
  - known limitations
  - upgrade/build notes if relevant
- [ ] release process in `docs/dev/RELEASE_PROCESS.md` still matches reality

## Current release posture
Until the standalone DRM path, XWayland compatibility, and packaging are more mature, releases should be framed as:

> **alpha compositor prototype with a strong nested development path**
