# WGPU Present Error Fix

## Problem

When running the Axiom compositor with the windowed test binary (`run_present_winit`) or the main binary, users were seeing this error repeatedly:

```
[2025-10-05T13:59:13Z ERROR wgpu_core::present] No work has been submitted for this frame
```

## Root Cause

The error occurred because the compositor's event loop was calling `frame.present()` on every redraw request, even when there were no windows to render. 

In the rendering pipeline:
1. The winit event loop requests redraws continuously (via `AboutToWait`)
2. On `RedrawRequested`, we get a surface frame with `surface.get_current_texture()`
3. We call `render_to_surface_with_outputs()` which creates a render pass
4. **When no windows exist**, no draw commands are submitted to the GPU
5. We call `frame.present()` anyway
6. WGPU detects no work was submitted and throws an error

## Solution

The fix checks whether there's actual content to render before presenting:

### In `src/main.rs` (main axiom binary):
```rust
// Only render and present if we have content to show
let has_content = renderer.window_count() > 0;

if has_content {
    // Render and present the frame
    renderer.render_to_surface_with_outputs(...)?;
    frame.present();
} else {
    // No windows yet - just drop the frame without presenting
    drop(frame);
}
```

### In `src/bin/run_present_winit.rs` (test binary):
Same logic applied to the windowed test binary.

## Benefits

1. **No more errors**: The "No work has been submitted" error is completely eliminated
2. **Better performance**: We don't waste GPU cycles when there's nothing to render
3. **Cleaner logs**: Error logs are now only real errors, not expected states
4. **Proper resource management**: Frames are explicitly dropped when unused

## How to Test

Run the compositor and verify no errors appear:

```bash
# Test with the windowed binary
cargo run --release --bin run_present_winit --features "smithay,wgpu-present"

# Or use the test script
./test_no_errors.sh
```

You should see a black window with no errors in the console, even when no Wayland clients are connected.

## When Content Appears

Once you connect a Wayland client:
```bash
# Find the Axiom socket
AXIOM_DISPLAY=wayland-2  # Check logs for actual display name

# Connect a client
WAYLAND_DISPLAY=$AXIOM_DISPLAY foot
```

The compositor will detect `has_content = true` and start rendering and presenting frames normally.

## Technical Details

The fix leverages the `AxiomRenderer::window_count()` method which returns the number of windows currently being managed by the renderer. This is updated by:
- `sync_from_shared()` - pulls window state from the Smithay server
- `upsert_window_rect()` - adds/updates windows
- `remove_window()` - removes windows

The check is very lightweight (just checking a Vec length) and happens every frame, ensuring we only render when needed.

## Related Files

- `src/main.rs` - Main compositor binary event loop
- `src/bin/run_present_winit.rs` - Windowed test binary event loop  
- `src/renderer/mod.rs` - Renderer with `window_count()` method
- `TESTING_WINDOWS.md` - Updated testing documentation
- `test_no_errors.sh` - Automated test script
