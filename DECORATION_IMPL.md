# Window Decoration Implementation - Step by Step

**Status**: Ready to Implement  
**Estimated Time**: 2-3 hours

---

## Overview

The DecorationManager is already fully implemented with all data structures and logic! We just need to:
1. Call `render_decorations()` from the compositor
2. Queue the overlay rectangles to the renderer
3. Test!

---

## Implementation Steps

### Step 1: Add Helper Function in smithay/server.rs

Add this function after `apply_layouts_inline()`:

```rust
/// Render decorations for all windows
fn render_decorations_inline(
    state: &CompositorState,
) {
    let dm = state.decoration_manager_handle.read();
    
    // Render decorations for each window with server-side decorations
    for (window_id, rect) in &state.last_layouts {
        if *window_id < 1_000_000 { // Skip layer surfaces
            if let Ok(render_data) = dm.render_decorations(*window_id, rect.clone(), None) {
                match render_data {
                    crate::decoration::DecorationRenderData::ServerSide {
                        titlebar_rect,
                        titlebar_bg,
                        border_width,
                        border_color,
                        corner_radius,
                        title,
                        text_color,
                        font_size,
                        buttons,
                    } => {
                        // Render titlebar background
                        if corner_radius > 0.0 {
                            crate::renderer::queue_overlay_fill_rounded(
                                *window_id,
                                titlebar_rect.x as f32,
                                titlebar_rect.y as f32,
                                titlebar_rect.width as f32,
                                titlebar_rect.height as f32,
                                titlebar_bg,
                                corner_radius,
                            );
                        } else {
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                titlebar_rect.x as f32,
                                titlebar_rect.y as f32,
                                titlebar_rect.width as f32,
                                titlebar_rect.height as f32,
                                titlebar_bg,
                            );
                        }
                        
                        // Render border
                        if border_width > 0 {
                            let bw = border_width as f32;
                            // Top border
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                rect.x as f32,
                                rect.y as f32,
                                rect.width as f32,
                                bw,
                                border_color,
                            );
                            // Bottom border
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                rect.x as f32,
                                (rect.y + rect.height as i32) as f32 - bw,
                                rect.width as f32,
                                bw,
                                border_color,
                            );
                            // Left border
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                rect.x as f32,
                                rect.y as f32,
                                bw,
                                rect.height as f32,
                                border_color,
                            );
                            // Right border
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                (rect.x + rect.width as i32) as f32 - bw,
                                rect.y as f32,
                                bw,
                                rect.height as f32,
                                border_color,
                            );
                        }
                        
                        // Render buttons
                        let theme = dm.theme();
                        
                        // Close button
                        if buttons.close.visible {
                            let color = if buttons.close.pressed {
                                theme.close_pressed
                            } else if buttons.close.hovered {
                                theme.close_hovered
                            } else {
                                theme.close_normal
                            };
                            
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                (titlebar_rect.x + buttons.close.bounds.x) as f32,
                                (titlebar_rect.y + buttons.close.bounds.y) as f32,
                                buttons.close.bounds.width as f32,
                                buttons.close.bounds.height as f32,
                                color,
                            );
                        }
                        
                        // Maximize button
                        if buttons.maximize.visible {
                            let color = if buttons.maximize.pressed {
                                theme.button_pressed
                            } else if buttons.maximize.hovered {
                                theme.button_hovered
                            } else {
                                theme.button_normal
                            };
                            
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                (titlebar_rect.x + buttons.maximize.bounds.x) as f32,
                                (titlebar_rect.y + buttons.maximize.bounds.y) as f32,
                                buttons.maximize.bounds.width as f32,
                                buttons.maximize.bounds.height as f32,
                                color,
                            );
                        }
                        
                        // Minimize button
                        if buttons.minimize.visible {
                            let color = if buttons.minimize.pressed {
                                theme.button_pressed
                            } else if buttons.minimize.hovered {
                                theme.button_hovered
                            } else {
                                theme.button_normal
                            };
                            
                            crate::renderer::queue_overlay_fill(
                                *window_id,
                                (titlebar_rect.x + buttons.minimize.bounds.x) as f32,
                                (titlebar_rect.y + buttons.minimize.bounds.y) as f32,
                                buttons.minimize.bounds.width as f32,
                                buttons.minimize.bounds.height as f32,
                                color,
                            );
                        }
                        
                        // TODO: Render title text (needs text rendering support)
                        // For now, we have colored rectangles which is good enough to see!
                    }
                    _ => {} // ClientSide or None - no server decorations
                }
            }
        }
    }
}
```

### Step 2: Call render_decorations_inline() in apply_layouts_inline()

After the window layout loop (around line 5970), add:

```rust
    // Render decorations for windows with server-side decorations
    render_decorations_inline(state);
```

### Step 3: Build and Test

```bash
cargo build --release --features="smithay,wgpu-present" --bin run_present_winit
./target/release/run_present_winit
```

You should see:
- Title bars on windows (dark gray rectangles)
- Borders (purple for focused, gray for unfocused)  
- Three buttons (red close, gray maximize/minimize)
- All rendered as colored rectangles on top of windows

---

## Expected Visual Result

```
┌──────────────────────────────────────┐
│ [─][□][×]                            │  ← Title bar (dark gray)
├──────────────────────────────────────┤
│                                       │
│                                       │
│         Window Content                │
│                                       │
│                                       │
└──────────────────────────────────────┘
   ↑ Border (purple if focused)
```

Buttons (right to left):
- `×` = Close (red)
- `□` = Maximize (gray)  
- `─` = Minimize (gray)

---

## Testing Plan

1. **Start compositor**: Should run without errors
2. **Check logs**: Look for "🎨 Generated decoration render data"
3. **Visual test**: See colored rectangles on windows
4. **Hover test**: Buttons should change color (if mouse tracking works)
5. **Click test**: Close button should work (sends xdg_toplevel.close())

---

## Next Steps After This Works

1. **Add title text rendering** - Use tiny-skia or similar
2. **Improve button icons** - Draw X, square, line symbols
3. **Add shadows** - Subtle drop shadow under titlebar
4. **Polish colors** - Match theme better
5. **Add resize handles** - Invisible zones at edges

---

## Files to Modify

1. **`src/smithay/server.rs`**
   - Add `render_decorations_inline()` function
   - Call it from `apply_layouts_inline()`

That's it! Just 2 changes to one file.

---

## Estimated Time

- **Adding function**: 15 minutes (copy/paste + adjustments)
- **Building**: 3-5 minutes  
- **Testing**: 5 minutes
- **Total**: ~25 minutes

Ready to implement!
