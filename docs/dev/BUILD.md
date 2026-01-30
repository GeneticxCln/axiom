# Building Axiom

## Build Artifacts

-   **Release**: `cargo build --release` -> `target/release/axiom`
    -   Optimized for performance. Use this for general usage.
-   **Debug**: `cargo build` -> `target/debug/axiom`
    -   Contains debug symbols and extensive logging. Use for development.

## Feature Flags

Axiom uses Cargo features to gate experimental or optional functionality.

-   `real-compositor`: Enables the full Smithay-based backend (Default).
-   `experimental-smithay`: Enables experimental Smithay integration modules.
-   `demo`: Enables internal demo modes.
-   `examples`: Builds example clients (e.g., metrics client).

## Running Tests

```bash
# Run all unit tests
cargo test

# Run specific test module
cargo test workspace::tests

# Run integration tests
cargo test --test integration_tests
```
