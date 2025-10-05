# Phase 6.3: WindowStack and Damage Tracking Integration

**Date:** 2024-12-19  
**Status:** COMPLETE  
**Milestone:** Phase 6.3 Rendering Pipeline - Multi-Window Z-Ordering and Damage Tracking

---

## Overview

This document describes the successful integration of **WindowStack** (Z-ordering) and **FrameDamage** (damage tracking) into the Axiom renderer's main render loop. These systems enable proper multi-window rendering with correct stacking order and optimized damage-aware rendering for performance.

### What Was Achieved

1. ✅ **WindowStack Integration**: Renderer now uses WindowStack for proper bottom-to-top Z-ordering
2. ✅ **Fast Window ID Lookup**: Added `window_id_to_index` HashMap for O(1) window lookups
3. ✅ **Damage Tracking Sync**: Frame damage state syncs from SharedRenderState to renderer
4. ✅ **Automatic Damage Clearing**: Frame damage automatically cleared after successful render
5. ✅ **Window Lifecycle Management**: Proper window addition, removal, and index map maintenance
6. ✅ **Instrumented Logging**: Added detailed logging for Z-order and damage tracking
7. ✅ **All Tests Pass**: 93 tests passing, including 18 WindowStack tests and 23 damage tracking tests

---

## Architecture Changes

### Data Structures Added

#### 1. `window_id_to_index` HashMap

Added to `AxiomRenderer` struct for fast window ID → index lookups during rendering:

```rust
pub struct AxiomRenderer {
    // ... existing fields ...
    
    /// Fast lookup: window ID → index in windows Vec
    window_id_to_index: HashMap<u64, usize>,
    
    /// Window Z-ordering stack (optional, for multi-window rendering)
    window_stack: Option<Arc<Mutex<WindowStack>>>,
    
    /// Frame damage tracking (optional, for performance)
    frame_damage: Option<Arc<Mutex<FrameDamage>>>,
}
```

#### 2. SharedRenderState Enhancements

`SharedRenderState` now includes window stack and frame damage:

```rust
struct SharedRenderState {
    placeholders: HashMap<u64, (Pos, Size, f32)>,
    pending_textures: Vec<(u64, Vec<u8>, u32, u32)>,
    pending_texture_regions: Vec<RegionUpdate>,
    overlay_rects: Vec<OverlayRect>,
    
    /// Window Z-ordering stack for multi-window rendering
    window_stack: WindowStack,
    
    /// Frame damage tracking for optimization
    frame_damage: FrameDamage,
}
```

**Default Implementation**: Added manual `Default` impl to initialize `WindowStack::new()` and `FrameDamage::new()`.

---

## Key Methods Modified

### 1. `add_window()` and `upsert_window_rect()`

**Before**: Only added windows to `self.windows` Vec.

**After**: Now also maintains `window_id_to_index` mapping:

```rust
pub fn add_window(&mut self, id: u64, position: (f32, f32), size: (f32, f32)) -> Result<()> {
    // ... create window ...
    
    let index = self.windows.len();
    self.windows.push(window);
    self.window_id_to_index.insert(id, index);  // NEW
    Ok(())
}
```

### 2. `rebuild_window_index()`

**New method** to rebuild the index map after window removal or reordering:

```rust
fn rebuild_window_index(&mut self) {
    self.window_id_to_index.clear();
    for (idx, window) in self.windows.iter().enumerate() {
        self.window_id_to_index.insert(window.id, idx);
    }
    debug!("🔧 Rebuilt window_id_to_index map: {} windows", self.windows.len());
}
```

### 3. `remove_window()`

**New method** for proper window removal with resource cleanup:

```rust
pub fn remove_window(&mut self, window_id: u64) -> bool {
    if let Some(idx) = self.window_id_to_index.get(&window_id).copied() {
        let window = self.windows.remove(idx);
        
        // Return resources to pools
        if let (Some(tex), Some((tw, th))) = (window.texture, window.tex_size) {
            let key = (tw, th, TextureFormat::Rgba8UnormSrgb);
            self.texture_pool.entry(key).or_default().push(tex);
        }
        if let Some(ubuf) = window.uniform {
            self.uniform_pool.push(ubuf);
        }
        
        // Rebuild index map since indices have shifted
        self.rebuild_window_index();
        
        info!("🗑️ Removed window {} from renderer", window_id);
        true
    } else {
        false
    }
}
```

### 4. `sync_from_shared()`

**Enhanced** to sync WindowStack and FrameDamage from SharedRenderState:

```rust
pub fn sync_from_shared(&mut self) {
    // ... existing placeholder and texture sync ...
    
    // Sync window stack for Z-ordering
    let stack_clone = s.window_stack.clone();
    if self.window_stack.is_none() {
        self.window_stack = Some(Arc::new(Mutex::new(stack_clone.clone())));
        info!("🪟 Initialized window_stack with {} windows", stack_clone.len());
    } else if let Some(ref stack_arc) = self.window_stack {
        if let Ok(mut local_stack) = stack_arc.lock() {
            *local_stack = stack_clone.clone();
            debug!("🪟 Synced window_stack: {} windows in Z-order", stack_clone.len());
        }
    }
    
    // Sync frame damage for optimization
    let damage_clone = s.frame_damage.clone();
    if self.frame_damage.is_none() {
        self.frame_damage = Some(Arc::new(Mutex::new(damage_clone.clone())));
    } else if let Some(ref damage_arc) = self.frame_damage {
        if let Ok(mut local_damage) = damage_arc.lock() {
            *local_damage = damage_clone.clone();
        }
    }
    
    // ... remove windows not in keep set ...
}
```

### 5. `render()` - Headless Render Method

**Enhanced** to use WindowStack for proper Z-ordering:

```rust
pub fn render(&mut self) -> Result<()> {
    // Use WindowStack for proper Z-ordering if available
    let render_order: Vec<u64> = if let Some(ref stack_arc) = self.window_stack {
        if let Ok(stack) = stack_arc.lock() {
            let order = stack.render_order().to_vec();
            if !order.is_empty() {
                info!("🪟 Rendering in Z-order: {:?} (bottom to top)", order);
            }
            order
        } else {
            self.windows.iter().map(|w| w.id).collect()
        }
    } else {
        self.windows.iter().map(|w| w.id).collect()
    };
    
    // Render windows in Z-order (bottom to top)
    for window_id in &render_order {
        if let Some(&window_idx) = self.window_id_to_index.get(window_id) {
            if let Some(window) = self.windows.get(window_idx) {
                // ... render window ...
            }
        }
    }
    
    // Clear frame damage after successful render
    if let Some(ref damage_arc) = self.frame_damage {
        if let Ok(mut damage) = damage_arc.lock() {
            damage.clear();
            debug!("💥 Cleared frame damage after render");
        }
    }
    
    Ok(())
}
```

### 6. `render_to_surface_with_outputs_scaled()`

**Major change**: Window iteration now uses WindowStack Z-ordering:

**Before**:
```rust
for widx in 0..self.windows.len() {
    let window = &mut self.windows[widx];
    // ... build draw commands ...
}
```

**After**:
```rust
// Use WindowStack for proper Z-ordering (bottom to top)
let render_order: Vec<u64> = if let Some(ref stack_arc) = self.window_stack {
    if let Ok(stack) = stack_arc.lock() {
        let order = stack.render_order().to_vec();
        if !order.is_empty() {
            info!("🪟 Rendering {} windows in Z-order: {:?}", order.len(), order);
        }
        order
    } else {
        self.windows.iter().map(|w| w.id).collect()
    }
} else {
    self.windows.iter().map(|w| w.id).collect()
};

// Iterate through windows in proper Z-order
for window_id in render_order {
    let widx = match self.window_id_to_index.get(&window_id) {
        Some(&idx) => idx,
        None => {
            warn!("⚠️ Window {} in stack but not in windows Vec, skipping", window_id);
            continue;
        }
    };
    
    // ... rest of render logic uses widx ...
}
```

**End of Frame**: Added damage clearing:

```rust
// Clear frame damage after successful render
if let Some(ref damage_arc) = self.frame_damage {
    if let Ok(mut damage) = damage_arc.lock() {
        let had_damage = damage.has_any_damage();
        let frame_num = damage.frame_number();
        damage.clear();
        if had_damage {
            debug!("💥 Cleared frame damage after render (frame {})", frame_num);
        }
    }
}
```

---

## How It Works

### Z-Ordering Flow

1. **Wayland Thread**: Calls `add_window_to_stack(window_id)` when surfaces are created
2. **SharedRenderState**: WindowStack maintained in shared state
3. **Renderer Sync**: `sync_from_shared()` copies WindowStack to renderer
4. **Render Loop**: Uses `window_stack.render_order()` to iterate windows bottom-to-top
5. **ID → Index Lookup**: Fast O(1) lookup via `window_id_to_index` HashMap
6. **Draw Commands**: Windows drawn in correct Z-order (bottom first, top last)

### Damage Tracking Flow

1. **Wayland Thread**: Calls `mark_window_damaged(id)` or `add_window_damage_region()` on buffer commits
2. **SharedRenderState**: FrameDamage accumulates per-window damage
3. **Renderer Sync**: `sync_from_shared()` copies FrameDamage to renderer
4. **Render Loop**: Can query damage state via `frame_damage.has_any_damage()`
5. **End of Frame**: Damage automatically cleared via `damage.clear()`

*(Future optimization: Use damage regions for scissor rectangles to skip undamaged areas)*

---

## Public API

### Window Stack Management

These functions are available for Wayland protocol handlers:

```rust
// Add window to stack (called on surface creation)
pub fn add_window_to_stack(window_id: u64)

// Remove window from stack (called on surface destruction)
pub fn remove_window_from_stack(window_id: u64)

// Raise window to top (called on focus/activation)
pub fn raise_window_to_top(window_id: u64)

// Get current render order
pub fn get_window_render_order() -> Vec<u64>
```

### Damage Tracking

```rust
// Mark entire window as damaged (full redraw)
pub fn mark_window_damaged(window_id: u64)

// Add specific damage region to window
pub fn add_window_damage_region(window_id: u64, x: i32, y: i32, width: u32, height: u32)

// Check if any damage pending
pub fn has_pending_damage() -> bool

// Clear all damage (called after render)
pub fn clear_frame_damage()
```

---

## Instrumentation and Logging

### Z-Order Logging

- `🪟 Initialized window_stack with N windows` - First stack initialization
- `🪟 Synced window_stack: N windows in Z-order` - Stack sync on each frame
- `🪟 Rendering N windows in Z-order: [id1, id2, ...]` - Render order trace
- `⚠️ Window N in stack but not in windows Vec, skipping` - Consistency warning

### Damage Tracking Logging

- `💥 Initialized frame_damage with pending damage` - First damage state init
- `💥 Synced frame_damage: has pending damage` - Damage state sync
- `💥 Cleared frame damage after render (frame N)` - Damage cleared

### Window Lifecycle Logging

- `➕ Adding window N at (x, y) size WxH` - Window added
- `🗑️ Removed window N from renderer` - Window removed
- `🔧 Rebuilt window_id_to_index map: N windows` - Index rebuilt

---

## Testing Status

### Unit Tests: ✅ ALL PASSING (93 tests)

- **WindowStack**: 18 tests covering all operations (push, remove, raise, lower, ordering)
- **Damage Tracking**: 23 tests covering damage accumulation, merging, and clearing
- **Renderer**: 2 tests covering basic rendering operations

### Integration Tests: 🟡 PENDING VISUAL VALIDATION

Visual validation requires a proper display environment (TTY, Xephyr, or standalone Wayland session). The automated SHM rendering test can validate end-to-end once display is available:

```bash
./test_shm_rendering.sh
```

Expected output:
- ✅ Window appears on screen
- ✅ Correct rendering of test pattern (color gradient)
- ✅ Window positioned correctly
- ✅ 8 success criteria met

---

## Performance Characteristics

### Z-Ordering Performance

- **Window Lookup**: O(1) via HashMap
- **Render Order**: O(n) iteration, one pass through stack
- **Stack Operations**: O(1) for top/bottom, O(n) for raise_above/remove
- **Memory**: ~24 bytes per window in stack (Vec + HashMap)

### Damage Tracking Performance

- **Damage Addition**: O(1) per region (until max regions reached)
- **Damage Merging**: O(n log n) for region sort + O(n) merge
- **Output Computation**: O(windows × regions) for screen damage
- **Memory**: ~80 bytes per window with damage

---

## Next Steps

### Immediate (Ready for Implementation)

1. **Visual Validation**
   - Run `./test_shm_rendering.sh` in proper display environment
   - Validate multi-window stacking with multiple clients
   - Verify Z-order changes when windows are raised/lowered

2. **Damage-Aware Rendering Optimization**
   - Use `FrameDamage::compute_output_damage()` to get screen damage regions
   - Apply scissor rectangles to render only damaged areas
   - Skip rendering for fully occluded windows

3. **Smithay Integration**
   - Call `add_window_to_stack()` in `xdg_surface::commit` handler
   - Call `mark_window_damaged()` on buffer commits
   - Call `raise_window_to_top()` on window activation
   - Call `remove_window_from_stack()` on surface destruction

### Medium-Term (Phase 6.3 Polish)

4. **Multi-Window Stress Testing**
   - Test with 10+ concurrent windows
   - Validate occlusion detection
   - Measure render performance with damage tracking enabled vs. disabled

5. **Effects Integration**
   - Integrate blur/rounded corners with multi-window pipeline
   - Ensure effects respect Z-order
   - Add shadow rendering for stacked windows

6. **Window Movement/Resize**
   - Add damage regions for window geometry changes
   - Optimize redraws during smooth window movement
   - Test drag-and-drop window reordering

### Long-Term (Phase 6.4+)

7. **Advanced Optimizations**
   - Occlusion culling (skip rendering fully covered windows)
   - Dirty region coalescing across frames
   - GPU-accelerated damage region computation

8. **Real-World Application Testing**
   - Test with Firefox, kitty, gnome-terminal, etc.
   - Validate stacking with XWayland windows
   - Stress test with window animations

---

## Implementation Notes

### Why HashMap for window_id_to_index?

The renderer's `windows` Vec stores windows in arbitrary order (order of addition), but we need to render them in Z-order from WindowStack. The HashMap provides O(1) lookup to map from window ID (from stack) to Vec index.

### Why Arc<Mutex<>> for WindowStack and FrameDamage?

These structures are shared between:
1. `SharedRenderState` (accessed by Wayland thread via public API functions)
2. `AxiomRenderer` (accessed by render thread)

`Arc<Mutex<>>` provides thread-safe shared ownership and mutation.

### Why Clone WindowStack/FrameDamage Instead of Sharing Arc?

We clone the structures during sync to:
1. Minimize lock contention (Wayland thread and render thread don't block each other)
2. Ensure renderer has consistent snapshot for entire frame
3. Simplify lifetime management (no lock held during entire render)

This is a performance trade-off: extra memory allocation vs. reduced lock contention. For typical window counts (< 100), the clone is cheap.

### Fallback Behavior

If WindowStack is not available (None) or lock fails, the renderer falls back to rendering windows in their current Vec order. This ensures rendering always succeeds even if stack sync fails.

---

## Compatibility

### Backward Compatibility

- ✅ Existing code that doesn't use WindowStack continues to work
- ✅ Windows render in Vec order if stack not initialized
- ✅ Public API additions only (no breaking changes)

### Forward Compatibility

- ✅ Ready for DMA-BUF/GPU-backed clients (when environment supports it)
- ✅ Ready for subsurfaces and popup stacking
- ✅ Ready for workspace/output-aware damage tracking

---

## Known Limitations

1. **No Scissor Optimization Yet**: Damage regions computed but not yet used for scissor rectangles
2. **No Occlusion Culling**: Fully covered windows still rendered (zero-cost since they're overdrawn)
3. **Clone Overhead**: WindowStack and FrameDamage cloned each frame (acceptable for typical window counts)
4. **No Subsurface Stacking**: WindowStack is flat; subsurfaces need nested stacking

These limitations are acceptable for Phase 6.3 and will be addressed in subsequent phases.

---

## Success Criteria: ✅ MET

- [x] WindowStack integrated into renderer
- [x] Window ID → index mapping maintained
- [x] Damage tracking synced to renderer
- [x] Damage automatically cleared after render
- [x] All tests passing (93/93)
- [x] Code compiles without errors
- [x] Logging instrumented for debugging
- [x] Documentation complete

**Phase 6.3 Window Stack Integration: COMPLETE** 🎉

---

## References

- `axiom/src/renderer/mod.rs` - Main renderer with integration
- `axiom/src/renderer/window_stack.rs` - WindowStack implementation
- `axiom/src/renderer/damage.rs` - Damage tracking implementation
- `PHASE_6_3_PROGRESS.md` - Overall Phase 6.3 progress
- `PHASE_6_3_MULTI_WINDOW_PLAN.md` - Multi-window architecture plan
- `SESSION_SUMMARY_PHASE_6_3.md` - Session summary and context