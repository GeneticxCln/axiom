# Building Axiom

## Build artifacts

- **Debug**: `cargo build` → `target/debug/axiom`
- **Release**: `cargo build --release` → `target/release/axiom`

For current development, the debug build is usually the most useful because the project is still in alpha and logging is valuable.

## Runtime recommendation

The recommended evaluation target right now is the nested compositor path:

```bash
cargo run -- --windowed --debug
```

## Rendering architecture

Axiom's rendering direction is now documented explicitly in:

- [Render Architecture](RENDER_ARCHITECTURE.md)

Short version: **WGPU is the primary compositor architecture; GL is currently a transitional presentation shim.**

## Cargo features

Axiom currently defines only a small feature surface:

- `default`: empty
- `examples`: builds the `metrics_client` example
- `demo`: enables internal demo entry points

There are no active Cargo features named `real-compositor`, `wgpu-present`, or `experimental-smithay` in the current manifest.

## Tests

```bash
# library/unit-style tests
cargo test --lib

# integration tests
cargo test --test integration_tests

# all tests
cargo test
```

Some graphics-related tests may require a GPU, a virtual display, or a more complete local environment than a minimal CI shell.

## Benchmarks

Axiom's benchmark suite is implemented with Criterion in:

- `benches/compositor_benchmarks.rs`

Recommended benchmark entry points:

```bash
# run the benchmark suite once
bash ./scripts/benchmark.sh run

# save a local baseline
bash ./scripts/benchmark.sh save-baseline local-main

# compare against a saved baseline
bash ./scripts/benchmark.sh compare local-main
```

Criterion artifacts are written under `target/criterion/`.

## Memory safety checks

For the current repository, the supported Valgrind path is:

```bash
bash ./scripts/memory_profile.sh valgrind-tests
```

That runs selected non-graphics library test filters (`workspace`, `config`, `effects`) under `cargo-valgrind`.

## XWayland validation

The current repository-level XWayland check is:

```bash
bash ./scripts/check_xwayland.sh all
```

That runs:
- the lifecycle-focused XWayland test
- a real X11 client smoke test (`xdpyinfo`) against the spawned XWayland display
- a real X11 metadata smoke test (`xmessage`) that verifies title/class properties are readable over X11
- a compositor-side XWM wiring smoke test that verifies `AxiomXwm` receives a real `WindowMapped` event
- an end-to-end X11-in-Axiom smoke test that launches Axiom, starts a real X11 client, and waits for compositor-side map/unmap logs

If no parent Wayland compositor is available, the wrapper can start a temporary headless Weston instance for the Rust-side XWayland tests and use `xvfb-run` for the end-to-end nested Axiom smoke when available. If `Xwayland` is not installed locally, it exits cleanly with a skip message.

## Security checks

The repository security wrapper keeps local and CI behavior aligned:

```bash
bash ./scripts/check_security.sh all
```

That runs both `cargo audit --deny warnings` and `cargo deny check`.

## Package staging validation

The current repository-level package build check executes the real Arch PKGBUILD
functions against the current checkout, validates the staged install tree, and
smoke-tests the installed artifacts:

```bash
bash ./scripts/build_arch_package.sh run
```

This is stronger than a metadata-only check because it runs `prepare`, `build`,
and `package` from `packaging/arch/PKGBUILD` with a real temporary `$pkgdir`,
then verifies the staged `axiom` binary and `axiom-session` wrapper behavior.

## Release preparation

The canonical alpha release-prep helper is:

```bash
bash ./scripts/release_prep.sh all v0.1.0-alpha.1
```

See also:
- `docs/dev/RELEASE_PROCESS.md`
- `docs/dev/RELEASE_CHECKLIST.md`

## DRM hardware validation

For real-hardware standalone DRM/KMS validation:

```bash
bash ./scripts/drm_validation_report.sh probe
bash ./scripts/drm_validation_report.sh report
```

See:
- `docs/dev/DRM_HARDWARE_VALIDATION.md`
