# Building Axiom

## Build Artifacts

-   **Release**: `cargo build --release` -> `target/release/axiom`
    -   Optimized for performance. Use this for general usage.
-   **Debug**: `cargo build` -> `target/debug/axiom`
    -   Contains debug symbols and extensive logging. Use for development.

## Feature Flags

Axiom uses Cargo features to gate optional functionality. The build is always-on (`default` is empty); the live Smithay 0.7 backend requires no extra flag.

-   `default`: empty (production build).
-   `real-compositor`: marker flag for the live compositor path (currently always-on).
-   `wgpu-present`: marker flag reserved for the wgpu surface-rendering path.
-   `experimental-smithay`: marker flag kept for ABI compatibility with the legacy `src/experimental/` paths.
-   `demo`: enables the `--demo` and `--effects-demo` demo binaries.
-   `examples`: builds the `metrics_client` example.

## Running Tests

```bash
# Run all unit tests
cargo test

# Run specific test module
cargo test workspace::tests

# Run integration tests
cargo test --test integration_tests
```
