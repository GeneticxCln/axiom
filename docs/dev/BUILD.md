# Building Axiom

## Build artifacts

- **Debug**: `cargo build` → `target/debug/axiom`
- **Release**: `cargo build --release` → `target/release/axiom`

For current development, the debug build is usually the most useful because logging is valuable.

## Runtime recommendation

The recommended evaluation target is the nested compositor path:

```bash
cargo run -- --windowed --debug
```

## Cargo features

Axiom currently defines only a small feature surface:

- `default`: empty
- `examples`: builds the `metrics_client` example

## Tests

```bash
# library/unit-style tests
cargo test --lib

# integration tests
cargo test --test integration_tests

# all tests
cargo test
```

Some graphics-related tests may require a GPU or a virtual display. Integration tests run headless via the `Noop` backend.

## Benchmarks

Axiom's benchmark suite is implemented with Criterion in:

- `benches/compositor_benchmarks.rs`

```bash
# run the benchmark suite once
cargo bench
```

## Security checks

```bash
bash ./scripts/check_security.sh all
```