# Render Architecture

## Current state

Axiom renders through the **Smithay 0.7 GLES backend bound to the winit
window**. There is no WGPU renderer, no effects pipeline, and no DRM/KMS
scanout path — those were removed. The compositor presents **real client
window content** plus server-side decoration titlebars.

### Winit GLES path

```text
client commits wl_buffer
  → bind winit GLES backend
  → import each client wl_buffer into a GlesTexture
  → build render elements:
      SolidColorRenderElement  (backdrop + SSD titlebars/buttons)
      TextureRenderElement     (client window content)
  → submit frame to the winit window
```

Each frame, `AxiomSmithayBackendReal::render()`:

1. Binds the winit GLES backend for the current output.
2. Imports every visible client's committed `wl_buffer` into a `GlesTexture`.
3. Composes a solid backdrop and the server-side decoration titlebars/buttons
   as `SolidColorRenderElement`s, and the client content as
   `TextureRenderElement`s.
4. Submits the frame, presenting real client pixels to the winit window.

`WinitEvent::Resized` updates the workspace viewport and the output mode, so
live resize works.

### Multi-output render loop (experimental)

When built with `--features multi-output-experimental`, the render loop
iterates over all outputs in `state.outputs`, calling
`prepare_render_elements_for_output()` per output to collect scene elements.
Currently each output renders into the shared winit framebuffer; the
infrastructure is in place for per-output viewport sizing when the workspace
manager supports distinct per-output tapes.

Key differences from the single-output path:

- `render()` binds the winit backend once, then loops: for each output →
  `prepare_render_elements_for_output()` → `render_scene_into()`.
- `prepare_render_elements_for_output()` delegates to
  `prepare_render_scene()` (single viewport for now; per-output viewport
  sizing is the upgrade path).
- The shared `texture_cache` on `State` is imported once and reused across
  outputs — no duplicate GPU texture uploads.
- Screencopy and damage submit are unchanged (single winit window).

## Running with the multi-output feature

```sh
# Build with the feature
cargo build --features multi-output-experimental

# Run tests
cargo test --features multi-output-experimental

# Run only multi-output integration tests
cargo test --features multi-output-experimental --test multi_output_render
```

## Scope boundaries

- **Backend layer** (`src/backend/`): input/events, Wayland protocol, output
  setup, winit GLES surface binding, and the render submission step.
- **Workspace engine** (`src/workspace/`): window geometry, scroll/momentum,
  per-column tiling, and gaps that feed the render elements.
- **Config / IPC**: drive state; they do not own rendering.

## Non-goals (current)

- No GPU post-processing (blur, shadows, rounded corners) — the effects module
  was removed. `LazyUIMessage::EffectsControl` is accepted by IPC but is a
  no-op.
- No standalone DRM/KMS scanout.
- No CPU readback / software composite path.

## Notes for contributors

1. New visual behavior belongs in the winit GLES render path
   (`src/backend/mod.rs` `render()`), using Smithay render elements
   (`SolidColorRenderElement` / `TextureRenderElement`).
2. Keep backend orchestration and workspace geometry separate: the workspace
   engine produces rectangles; the backend turns them into render elements.
3. Do not reintroduce a second rendering architecture (WGPU, DRM) without a
   documented decision — the project converged on the single winit GLES path
   to stay maintainable.
