# Contributing to Axiom

We welcome contributions!

## Development Flow

1.  **Fork & Clone**: Fork the repo and clone it locally.
2.  **Branch**: Create a feature branch (`git checkout -b feature/my-feature`).
3.  **Code**: Implement your changes.
    -   Follow Rust formatting: `cargo fmt`
    -   Ensure no warnings: `cargo check`
4.  **Test**: Add unit tests for logic and ensure `cargo test` passes.
5.  **Pull Request**: Submit PR with a clear description.

## Code Style

-   Use **standard Rust formatting** (`rustfmt`).
-   Prioritize **readability** and **safety** (avoid `unsafe` unless strictly necessary).
-   Document public APIs with doc comments (`///`).

## Project Structure

-   `src/compositor.rs`: Main entry loop.
-   `src/workspace/`: Scrollable workspace logic (niri-style).
-   `src/renderer/`: wgpu rendering pipeline.
-   `src/window/`: Window state management.
-   `src/experimental/smithay/`: Smithay backend integration.

## Communication

Feel free to open Issues for bugs or feature requests.
