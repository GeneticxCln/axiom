# Building Axiom

## Build dependencies

See [docs/user/INSTALL.md](../user/INSTALL.md) for system-specific dependency
installation (Ubuntu, Arch, Fedora).

## Build commands

```bash
cargo build                    # debug build → target/debug/axiom
cargo build --release          # optimized build → target/release/axiom
```

For development, the debug build is usually preferred because logging is
valuable and compile times are shorter.

## Cargo features

Axiom defines a minimal feature surface:

- `default`: empty
- `examples`: builds the `metrics_client` example
- `multi-output-experimental`: enables multi-output render infrastructure (experimental)

## Running the compositor

```bash
cargo run -- --debug
```

The `--windowed` flag is accepted but redundant — the compositor always uses
the winit backend. See [Running](../user/RUNNING.md) for details.

## Tests

```bash
# Library/unit tests
cargo test --lib

# Integration tests
cargo test --test integration_tests

# All tests (unit + integration)
cargo test

# All targets (includes benches)
cargo test --all-targets

# Run tests requiring an X server (e.g., screencopy)
xvfb-run -a cargo test

# Run with specific features
cargo test --features multi-output-experimental
```

Some graphics-related tests require a GPU or virtual display (`xvfb-run`).
Tests that need an X server are marked `#[ignore]` and skipped by default.

## Automated smoke test

```bash
cargo build
xvfb-run -a ./scripts/nested_smoke_test.sh ./target/debug/axiom
```

This launches Axiom under `xvfb`, probes its Wayland socket, starts
`weston-terminal`, waits for a mapped XDG toplevel, then verifies clean
teardown.

## Benchmarks

```bash
cargo bench
```

Benchmarks use Criterion and live in `benches/compositor_benchmarks.rs`.

## Documentation

```bash
cargo doc --no-deps --open
```

## Security checks

```bash
bash ./scripts/check_security.sh all
```

Runs `cargo audit` and `cargo deny` against the dependency tree.