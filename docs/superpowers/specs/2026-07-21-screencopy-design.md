# Screencopy Protocol — Phase 4.2

## Protocol

`zwlr_screencopy_manager_v1` version 1 (SHM-only capture). The wlr-screencopy
protocol allows Wayland clients (grim, wf-recorder, etc.) to capture composited
output as pixel data.

Smithay 0.7 has no built-in screencopy handler — the protocol types are
available through `wayland_protocols_wlr` (re-exported by smithay), but all
dispatch logic and capture implementation is custom.

## Architecture

**Files changed:**
- `src/backend/mod.rs` — `ScreencopyManagerState` field on `State`, state
  accessor, `Dispatch` impls for `zwlr_screencopy_manager_v1` and
  `zwlr_screencopy_frame_v1`, `delegate_screencopy!` macro
- `src/backend/render.rs` — `capture_screencopy()` helper: renders scene to
  an offscreen `GlesTexture`, reads pixels via `ExportMem::copy_framebuffer()`
- `Cargo.toml` — enable `wayland_protocols_wlr` feature on smithay if needed

Follows the same pattern as existing protocol handlers (layer_shell,
session_lock, etc.).

## Data Flow

```
Client                           Compositor
  │                                    │
  │ bind zwlr_screencopy_manager_v1    │
  │───────────────────────────────────>│
  │                                    │
  │ capture_output(frame, cursor, out) │
  │───────────────────────────────────>│  → Create frame resource
  │                                    │  → Send buffer(format, w, h, stride)
  │          buffer(format,w,h,stride) │  → Send buffer_done()
  │          buffer_done()             │
  │<───────────────────────────────────│
  │                                    │
  │ copy(frame, wl_buffer)             │
  │───────────────────────────────────>│  → Create offscreen GlesTexture
  │                                    │  → render_scene_into(target, size)
  │                                    │  → ExportMem::copy_framebuffer()
  │                                    │  → map_texture() → &[u8]
  │                                    │  → with_buffer_contents_mut() → copy pixels
  │          flags(YInvert)            │  → frame.flags()
  │          ready(sec_hi, sec_lo, ns) │  → frame.ready() — frame done
  │<───────────────────────────────────│
```

## Capture Implementation (`render.rs`)

Uses Smithay's `Offscreen<GlesTexture>`, `Bind`, and `ExportMem` traits (all
available with the `renderer_gl` feature).

```rust
pub fn capture_screencopy(
    renderer: &mut GlesRenderer,
    state: &mut State,
    size: Size<i32, BufferCoord>,
) -> Result<Vec<u8>, ()> {
    use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
    use smithay::backend::allocator::Fourcc;

    // 1. Create offscreen texture at output size
    let mut tex = Offscreen::<GlesTexture>::create_buffer(
        renderer, Fourcc::Argb8888, size,
    )?;
    // 2. Bind as framebuffer target → GlesTarget
    let mut target = renderer.bind(&mut tex)?;
    // 3. Render current scene into the offscreen target
    render_scene_into(state, renderer, &mut target)?;
    // 4. Read pixels back to CPU
    let region = Rectangle::from_loc_and_size((0, 0), size);
    let mapping = renderer.copy_framebuffer(&target, region, Fourcc::Argb8888)?;
    let pixels: &[u8] = renderer.map_texture(&mapping)?;
    // 5. Return owned copy
    Ok(pixels.to_vec())
}
```

## Protocol Handler (`backend/mod.rs`)

Add to `State`:
```rust
screencopy_manager_state: ScreencopyManagerState<Self>,
```

State accessor:
```rust
fn screencopy_manager_state(&self) -> &ScreencopyManagerState<Self> { ... }
```

`Dispatch<ZwlrScreencopyManagerV1, ...>`:
- `capture_output`: create frame, send buffer/buffer_done events
- `capture_output_region`: not supported (return to client if sent)

`Dispatch<ZwlrScreencopyFrameV1, ...>`:
- `copy`: validate buffer, render offscreen, write to SHM
- `copy_with_damage`: not supported at V1 (protocol-level error)
- `destroy`: clean up frame state

## Error Handling

| Failure | Handling |
|---------|----------|
| SHM buffer wrong size | `frame.failed()` |
| GL/rendering error | `frame.failed()` |
| Client sends bad data | Disconnect client |
| Any error during capture | `frame.failed()` — client retries |

## Testing

One integration test in `tests/integration_tests.rs`:
- Bind `zwlr_screencopy_manager_v1`
- Create a `wl_shm_pool` + `wl_buffer` at output size
- Send `capture_output`, then `copy`
- Wait for `ready` event
- Assert non-zero pixel data (proves frame was captured)

## Future Considerations (not implemented)

- **Damage tracking (v2):** `copy_with_damage` request + `damage` events for
  incremental updates. Essential for streaming performance (wf-recorder).
- **Linux-dmabuf (v3):** Zero-copy capture via DMA-BUF. Avoids GPU→CPU→GPU
  round-trip. Useful for compositor-to-compositor capture.
- **Region capture:** `capture_output_region` for sub-rectangle capture.
- **Cursor overlay:** Separate cursor rendering via the `cursor` parameter.

None of these are needed for V1. Adding them later doesn't break clients.
