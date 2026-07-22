# Contributing to Axiom

We welcome contributions.

## Current project expectations

Axiom is an **alpha-stage compositor prototype**. Please prefer small, reviewable changes that improve correctness, documentation, test coverage, or a clearly scoped subsystem.

## Development flow

1. Fork and clone the repository.
2. Create a focused branch for your change.
3. Implement the change.
4. Run formatting and tests locally where possible.
5. Submit a pull request with:
   - what changed,
   - why it changed,
   - any limitations or follow-up work.

## Code style

- Use standard Rust formatting (`cargo fmt`).
- Keep warnings low (`cargo check`, `cargo clippy` where available).
- Prefer small modules and clear ownership boundaries.
- Avoid `unsafe` unless it is required for backend/graphics interop and clearly justified.
- Document public APIs with `///` comments where practical.

## Project structure

- `src/compositor.rs` — top-level orchestration and tick loop
- `src/backend/` — Smithay backend orchestration (winit + GLES), input/event handling, clipboard, and render path.
- `src/workspace/` — scrollable workspace logic
- `src/window/` — window registry/state
- `src/ipc/` — Unix-socket IPC protocol/server
- `src/config/` — TOML config model and validation

## Good contribution targets

- bug fixes in lifecycle/state synchronization
- renderer/backend cleanup
- tests for compositor behavior
- documentation accuracy
- packaging/session assets

## Communication

Please open an issue or PR discussion for larger architectural changes before starting them.
