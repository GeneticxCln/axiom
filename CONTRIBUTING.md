# Contributing to Axiom

Axiom is a **winit-only Wayland compositor** with niri-inspired scrollable
workspaces and GLES rendering. It is an alpha-stage prototype.

## Build

```sh
cargo build                    # debug build
cargo build --release          # optimized binary
```

## Test

```sh
cargo test                     # unit + integration tests
cargo test --all-targets       # includes benches and integration tests
xvfb-run -a cargo test         # run all tests (including xvfb-required)
```

A few tests require an X server (`xvfb-run`). They are marked `#[ignore]`
and are skipped by default.

## Code quality

Ensure your changes are clean before submitting:

```sh
cargo fmt --check              # formatting
cargo clippy --all-targets -- -D warnings  # lints
cargo test --workspace         # all tests pass
cargo doc --no-deps            # documentation compiles without warnings
```

## Pull request workflow

1. Create a focused branch off `main`.
2. Make your changes — prefer small, reviewable commits.
3. Verify:
   - Build is clean (`cargo build` — no warnings).
   - Clippy is clean (`cargo clippy --all-targets -- -D warnings`).
   - All tests pass (`cargo test --workspace`).
4. Open a PR against `main` with:
   - A short description of what changed and why.
   - Any known limitations or follow-up items.

## Code style

- Standard Rust formatting (`cargo fmt`).
- Minimal `unsafe` — only for backend/graphics interop, with an inline
  safety comment.
- Document public APIs with `///` doc comments.
- Keep modules focused and ownership boundaries clear.

## Project structure

| Path | Purpose |
|------|---------|
| `src/compositor.rs` | Top-level orchestration and tick loop |
| `src/backend/` | Smithay backend (winit + GLES), input, render, clipboard |
| `src/workspace/` | Scrollable workspace management |
| `src/window/` | Window registry and state |
| `src/ipc/` | Unix-socket IPC protocol and server |
| `src/config/` | TOML configuration model and validation |

## More details

See [docs/dev/CONTRIBUTING.md](docs/dev/CONTRIBUTING.md) for a more detailed
contributing guide, and [docs/dev/BUILD.md](docs/dev/BUILD.md) for build
and test specifics.
