# Axiom Alpha Release Notes Template

Use this template for each alpha release note draft.

> **Release posture:** Axiom is an **alpha compositor prototype with a strong nested development path**.

---

# Axiom vX.Y.Z-alpha.N

## Summary

A short 2-4 bullet summary of the most important user-visible changes.

- TODO: major compositor/runtime improvement
- TODO: major testing/packaging/release-process improvement
- TODO: notable limitation removed or documentation corrected

## Recommended evaluation path

The recommended way to evaluate Axiom remains the nested/windowed path:

```bash
cargo run -- --windowed --debug
```

Real nested smoke path:

```bash
xvfb-run -a bash ./scripts/nested_smoke_test.sh ./target/debug/axiom
```

## Highlights

### Compositor/runtime
- TODO
- TODO

### Packaging / release / CI
- TODO
- TODO

## Validation included in this alpha

Mention the checks that were run for this release.

- `cargo fmt -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --lib --all-features -- --test-threads=1`
- `cargo test --test integration_tests --all-features`
- `bash ./scripts/coverage.sh unit`
- `xvfb-run -a bash ./scripts/nested_smoke_test.sh ./target/debug/axiom`
- `bash ./scripts/benchmark.sh run`
- `bash ./scripts/memory_profile.sh valgrind-tests`
- `bash ./scripts/check_security.sh all`
- `bash ./scripts/build_arch_package.sh run`

Adjust the list honestly if some checks were skipped or not applicable.

## Known limitations

Call out the current limitations explicitly.

- The compositor is winit-only (nested mode); standalone DRM/KMS is not available.
- Visible server-side decorations are not fully integrated into the live output path; current live policy remains **CSD-first**.
- Rendering uses direct GLES through the winit window. WGPU and presentation bridges have been removed.

Reference:
- `docs/user/LIMITATIONS.md`

## Build / packaging notes

- TODO: mention any dependency changes
- TODO: mention any session-wrapper or packaging changes
- TODO: mention any config migration or install caveat

## Upgrade notes

- TODO: mention config changes if any
- TODO: mention removed/renamed behavior if any
- TODO: otherwise state “No special upgrade steps required for this alpha.”

## Full notes

Add any additional implementation details that are worth surfacing publicly.
