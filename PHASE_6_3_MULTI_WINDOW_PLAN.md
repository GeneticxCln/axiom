# Phase 6.3: Multi-Window Support Implementation Plan

**Status**: 🔄 Planning  
**Priority**: HIGH  
**Prerequisites**: Single-window rendering validated  
**Estimated Time**: 8-12 hours  
**Target Completion**: Week 1 after visual validation

---

## Executive Summary

Multi-window support is the next critical feature after single-window rendering validation. This plan details how to extend the current rendering pipeline to handle multiple concurrent windows with proper Z-ordering, occlusion handling, and performance optimization.

**Goal**: Render multiple windows simultaneously with correct stacking order and efficient GPU utilization.

---

## Current State

### What's Working ✅

**Single Window Rendering**:
- Buffer reception from clients
- SHM format conversion (ARGB8888 → RGBA)
- Texture upload to GPU
- Bind group creation
- Uniform buffer updates
- Render pass execution
- Frame presentation

**Window Management**:
- Window creation and destruction
- Surface management (Smithay)
- XDG shell protocol handling
- Basic focus management

### Architecture Overview

```rust
// Current data structures (simplified)

struct Renderer {
    texture_pool: HashMap<(u32, u32, Format), Texture>,
    windows: HashMap<WindowId, RenderedWindow>,
}

struct RenderedWindow {
    id: WindowId,
    texture: Option<TextureId>,
    texture_view: Option<TextureView>,
    position: (i32, i32),
    size: (u32, u32),
    visible: bool,
}

struct SharedRenderState {
    pending_textures: Vec<TextureUpdate>,
    window_positions: HashMap<WindowId, (i32, i32)>,
}
```

---

## Requirements

### Functional Requirements

1. **Multiple Window Rendering**
   - Render 2+ windows simultaneously
   - Each window maintains independent texture
   - Windows update independently

2. **Z-Ordering (Stacking)**
   - Windows have explicit Z-order
   - Focus changes update Z-order
   - Top window receives input
   - Visual stacking matches logical stacking

3. **Occlusion Handling**
   - Overlapping windows render correctly
   - Transparent/translucent windows supported
   - No visual artifacts at boundaries

4. **Performance**
   - 60 FPS with 10+ windows
   - Efficient GPU memory usage
   - Minimal CPU overhead
   - Damage tracking for partial updates

5. **Window Lifecycle**
   - New windows appear correctly
   - Closing windows don't affect others
   - Window reordering is smooth
   - No flashing or artifacts

### Non-Functional Requirements

1. **Memory Efficiency**
   - Texture pool reuse across windows
   - Release unused textures
   - Bounded memory growth

2. **Rendering Efficiency**
   - Draw only visible portions
   - Skip fully occluded windows
   - Batch similar operations

3. **Thread Safety**
   - Safe concurrent access to window list
   - Proper synchronization with Wayland thread
   - No race conditions

---

## Architecture Design

### Window Stack Management

```rust
/// Represents the complete window stack with Z-ordering
pub struct WindowStack {
    /// Windows ordered from bottom to top
    /// Index 0 = bottom-most, last index = top-most
    windows: Vec<WindowId>,
    
    /// Fast lookup: WindowId → stack position
    positions: HashMap<WindowId, usize>,
}

impl WindowStack {
    /// Add window to top of stack
    pub fn push(&mut self, window_id: WindowId) {
        let position = self.windows.len();
        self.windows.push(window_id);
        self.positions.insert(window_id, position);
    }
    
    /// Remove window from stack
    pub fn remove(&mut self, window_id: WindowId) {
        if let Some(pos) = self.positions.remove(&window_id) {
            self.windows.remove(pos);
            // Rebuild position map
            self.rebuild_positions();
        }
    }
    
    /// Raise window to top
    pub fn raise_to_top(&mut self, window_id: WindowId) {
        self.remove(window_id);
        self.push(window_id);
    }
    
    /// Get windows in bottom-to-top order for rendering
    pub fn render_order(&self) -> &[WindowId] {
        &self.windows
    }
    
    /// Get top-most window (receives input)
    pub fn top(&self) -> Option<WindowId> {
        self.windows.last().copied()
    }
}
```

### Rendering Pipeline Changes

```rust
/// Enhanced renderer with multi-window support
impl Renderer {
    /// Render all windows in correct Z-order
    pub fn render_frame(
        &mut self,
        window_stack: &WindowStack,
        output_size: (u32, u32),
    ) -> Result<()> {
        // Get windows in bottom-to-top order
        let windows = window_stack.render_order();
        
        // Begin render pass
        let mut render_pass = self.begin_render_pass(output_size)?;
        
        // Render each window in order
        for &window_id in windows {
            if let Some(window) = self.windows.get(&window_id) {
                if window.visible && window.texture.is_some() {
                    self.render_window(&mut render_pass, window)?;
                }
            }
        }
        
        drop(render_pass);
        
        // Submit commands
        self.queue.submit(std::iter::once(encoder.finish()));
        
        Ok(())
    }
    
    /// Render a single window within the render pass
    fn render_window(
        &mut self,
        render_pass: &mut RenderPass,
        window: &RenderedWindow,
    ) -> Result<()> {
        // Update uniforms for this window
        let uniforms = WindowUniforms {
            position: window.position,
            size: window.size,
            // ... other fields
        };
        self.update_window_uniforms(window.id, &uniforms)?;
        
        // Bind window texture
        render_pass.set_bind_group(0, &window.bind_group, &[]);
        
        // Draw window quad
        render_pass.draw_indexed(0..6, 0, 0..1);
        
        Ok(())
    }
}
```

### Occlusion Detection (Optional Optimization)

```rust
/// Check if window is fully occluded by windows above it
fn is_fully_occluded(
    window: &RenderedWindow,
    windows_above: &[&RenderedWindow],
) -> bool {
    let window_rect = Rect {
        x: window.position.0,
        y: window.position.1,
        width: window.size.0 as i32,
        height: window.size.1 as i32,
    };
    
    // Check if any single window above fully covers this one
    for above in windows_above {
        if !above.transparent {
            let above_rect = Rect {
                x: above.position.0,
                y: above.position.1,
                width: above.size.0 as i32,
                height: above.size.1 as i32,
            };
            
            if above_rect.contains(&window_rect) {
                return true;
            }
        }
    }
    
    false
}
```

---

## Implementation Steps

### Step 1: Window Stack Data Structure (2 hours)

**Tasks**:
1. Create `WindowStack` struct in `src/renderer/window_stack.rs`
2. Implement Z-order management methods
3. Add thread-safe wrapper (Arc<Mutex<WindowStack>>)
4. Integrate with SharedRenderState

**Deliverables**:
- `window_stack.rs` module
- Unit tests for stack operations
- Integration with renderer

**Validation**:
```rust
#[test]
fn test_window_stack_ordering() {
    let mut stack = WindowStack::new();
    stack.push(WindowId(1));
    stack.push(WindowId(2));
    stack.push(WindowId(3));
    
    assert_eq!(stack.render_order(), &[WindowId(1), WindowId(2), WindowId(3)]);
    assert_eq!(stack.top(), Some(WindowId(3)));
    
    stack.raise_to_top(WindowId(1));
    assert_eq!(stack.render_order(), &[WindowId(2), WindowId(3), WindowId(1)]);
}
```

### Step 2: Multi-Window Render Loop (3 hours)

**Tasks**:
1. Modify `render_to_surface()` to iterate over window stack
2. Update bind group creation for per-window textures
3. Implement per-window uniform updates
4. Handle window visibility flags

**Changes in `src/renderer/mod.rs`**:

```rust
// Before (single window):
pub fn render_to_surface(&mut self, ...) {
    // Render THE window
    if let Some(window) = self.windows.get(&focused_window) {
        // ... render this window
    }
}

// After (multi-window):
pub fn render_to_surface(&mut self, window_stack: &WindowStack, ...) {
    let mut encoder = ...;
    let mut render_pass = ...;
    
    // Render all windows in Z-order
    for &window_id in window_stack.render_order() {
        if let Some(window) = self.windows.get(&window_id) {
            if window.visible && window.texture.is_some() {
                self.render_window_to_pass(&mut render_pass, window)?;
            }
        }
    }
    
    drop(render_pass);
    self.queue.submit(...);
}
```

**Deliverables**:
- Updated render loop
- Per-window draw calls
- Proper state management

**Validation**:
- Render 2 test windows
- Verify both appear
- Check Z-order is correct

### Step 3: Focus and Z-Order Integration (2 hours)

**Tasks**:
1. Update focus manager to modify window stack
2. Raise focused window to top
3. Handle window close events
4. Update input routing

**Changes in `src/smithay/server.rs`**:

```rust
// When window gains focus:
fn handle_window_focus(&mut self, window_id: WindowId) {
    // Update Wayland focus
    self.seat.keyboard_handle().set_focus(surface);
    
    // Update Z-order
    let mut stack = self.window_stack.lock().unwrap();
    stack.raise_to_top(window_id);
    
    // Trigger redraw
    self.request_redraw();
}

// When window closes:
fn handle_window_destroy(&mut self, window_id: WindowId) {
    // Remove from stack
    let mut stack = self.window_stack.lock().unwrap();
    stack.remove(window_id);
    
    // Clean up resources
    self.windows.remove(&window_id);
}
```

**Deliverables**:
- Focus integration
- Window lifecycle handling
- Stack consistency

**Validation**:
- Click between windows
- Verify focused window comes to front
- Close window, verify others unaffected

### Step 4: Performance Optimization (2 hours)

**Tasks**:
1. Implement damage tracking per window
2. Skip fully occluded windows
3. Batch GPU operations where possible
4. Profile rendering performance

**Optimizations**:

```rust
// Damage tracking
struct WindowDamage {
    window_id: WindowId,
    damaged_regions: Vec<Rect>,
    full_damage: bool,
}

// Only render damaged windows
fn render_frame_optimized(&mut self, damage: &[WindowDamage]) {
    for damaged_window in damage {
        if damaged_window.full_damage {
            // Re-render entire window
        } else {
            // Render only damaged regions
        }
    }
}

// Occlusion culling
fn should_render_window(&self, window: &RenderedWindow, stack: &WindowStack) -> bool {
    if !window.visible {
        return false;
    }
    
    // Get windows above this one
    let windows_above = self.get_windows_above(window.id, stack);
    
    // Skip if fully occluded
    !self.is_fully_occluded(window, &windows_above)
}
```

**Deliverables**:
- Damage tracking implementation
- Occlusion culling
- Performance metrics

**Validation**:
- Profile with 10+ windows
- Verify 60 FPS maintained
- Check CPU/GPU usage

### Step 5: Testing and Validation (2-3 hours)

**Tasks**:
1. Create multi-window test client
2. Test various scenarios
3. Fix bugs and edge cases
4. Document behavior

**Test Scenarios**:

1. **Basic Multi-Window**
   ```bash
   # Run 3 test clients simultaneously
   ./shm_test_client &
   ./shm_test_client &
   ./shm_test_client &
   ```
   - Verify all 3 windows appear
   - Check Z-order is correct
   - Ensure no visual artifacts

2. **Window Lifecycle**
   - Create window → appears correctly
   - Focus window → raises to top
   - Close window → others unaffected

3. **Overlapping Windows**
   - Position windows to overlap
   - Verify top window fully visible
   - Check transparency/occlusion

4. **Performance Test**
   - Open 10+ windows
   - Monitor FPS (should maintain 60)
   - Check memory usage (should be reasonable)

5. **Stress Test**
   - Rapidly create/destroy windows
   - Switch focus frequently
   - Verify stability

**Deliverables**:
- Test suite
- Bug fixes
- Performance validation
- Documentation updates

---

## Data Structure Details

### Complete Window Stack

```rust
// src/renderer/window_stack.rs

use std::collections::HashMap;
use crate::window::WindowId;

#[derive(Debug, Clone)]
pub struct WindowStack {
    /// Windows in Z-order (bottom to top)
    windows: Vec<WindowId>,
    
    /// Fast lookup for position
    positions: HashMap<WindowId, usize>,
    
    /// Total number of windows
    count: usize,
}

impl WindowStack {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            positions: HashMap::new(),
            count: 0,
        }
    }
    
    pub fn push(&mut self, window_id: WindowId) {
        let position = self.windows.len();
        self.windows.push(window_id);
        self.positions.insert(window_id, position);
        self.count += 1;
    }
    
    pub fn remove(&mut self, window_id: WindowId) -> Option<usize> {
        let pos = self.positions.remove(&window_id)?;
        self.windows.remove(pos);
        self.rebuild_positions();
        self.count -= 1;
        Some(pos)
    }
    
    pub fn raise_to_top(&mut self, window_id: WindowId) {
        if self.remove(window_id).is_some() {
            self.push(window_id);
        }
    }
    
    pub fn lower_to_bottom(&mut self, window_id: WindowId) {
        if self.remove(window_id).is_some() {
            self.windows.insert(0, window_id);
            self.rebuild_positions();
            self.count += 1;
        }
    }
    
    pub fn render_order(&self) -> &[WindowId] {
        &self.windows
    }
    
    pub fn top(&self) -> Option<WindowId> {
        self.windows.last().copied()
    }
    
    pub fn len(&self) -> usize {
        self.count
    }
    
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    
    pub fn contains(&self, window_id: WindowId) -> bool {
        self.positions.contains_key(&window_id)
    }
    
    pub fn position(&self, window_id: WindowId) -> Option<usize> {
        self.positions.get(&window_id).copied()
    }
    
    fn rebuild_positions(&mut self) {
        self.positions.clear();
        for (i, &window_id) in self.windows.iter().enumerate() {
            self.positions.insert(window_id, i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_operations() {
        let mut stack = WindowStack::new();
        assert!(stack.is_empty());
        
        stack.push(WindowId(1));
        stack.push(WindowId(2));
        stack.push(WindowId(3));
        
        assert_eq!(stack.len(), 3);
        assert_eq!(stack.top(), Some(WindowId(3)));
        assert_eq!(stack.render_order(), &[WindowId(1), WindowId(2), WindowId(3)]);
    }
    
    #[test]
    fn test_raise_to_top() {
        let mut stack = WindowStack::new();
        stack.push(WindowId(1));
        stack.push(WindowId(2));
        stack.push(WindowId(3));
        
        stack.raise_to_top(WindowId(1));
        
        assert_eq!(stack.render_order(), &[WindowId(2), WindowId(3), WindowId(1)]);
        assert_eq!(stack.top(), Some(WindowId(1)));
    }
    
    #[test]
    fn test_remove() {
        let mut stack = WindowStack::new();
        stack.push(WindowId(1));
        stack.push(WindowId(2));
        stack.push(WindowId(3));
        
        stack.remove(WindowId(2));
        
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.render_order(), &[WindowId(1), WindowId(3)]);
        assert!(!stack.contains(WindowId(2)));
    }
}
```

---

## Integration Points

### 1. Smithay Server Integration

**Location**: `src/smithay/server.rs`

```rust
pub struct AxiomCompositor {
    // ... existing fields
    
    /// Window Z-order stack
    window_stack: Arc<Mutex<WindowStack>>,
}

impl AxiomCompositor {
    // When new window created:
    fn handle_new_toplevel(&mut self, surface: WlSurface) {
        let window_id = self.create_window(surface);
        
        // Add to stack
        self.window_stack.lock().unwrap().push(window_id);
    }
    
    // When window destroyed:
    fn handle_destroy_toplevel(&mut self, window_id: WindowId) {
        // Remove from stack
        self.window_stack.lock().unwrap().remove(window_id);
        
        // Clean up resources
        self.windows.remove(&window_id);
    }
    
    // When focus changes:
    fn handle_focus_change(&mut self, window_id: WindowId) {
        // Update Z-order
        self.window_stack.lock().unwrap().raise_to_top(window_id);
        
        // Update keyboard focus
        self.update_keyboard_focus(window_id);
    }
}
```

### 2. Renderer Integration

**Location**: `src/renderer/mod.rs`

```rust
impl Renderer {
    pub fn render_frame(
        &mut self,
        window_stack: &Arc<Mutex<WindowStack>>,
        encoder: &mut CommandEncoder,
        view: &TextureView,
        output_size: (u32, u32),
    ) -> Result<()> {
        // Get window order
        let stack = window_stack.lock().unwrap();
        let windows = stack.render_order();
        
        // Begin render pass
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("multi-window-render-pass"),
            color_attachments: &[/* ... */],
            depth_stencil_attachment: None,
        });
        
        // Render each window
        for &window_id in windows {
            if let Some(window) = self.windows.get(&window_id) {
                if self.should_render(window) {
                    self.render_window_internal(&mut render_pass, window)?;
                }
            }
        }
        
        drop(render_pass);
        Ok(())
    }
    
    fn should_render(&self, window: &RenderedWindow) -> bool {
        window.visible && window.texture.is_some()
    }
}
```

### 3. Main Loop Integration

**Location**: `src/bin/run_present_winit.rs`

```rust
// In main event loop:
loop {
    // Process pending texture updates
    process_pending_texture_updates(&mut renderer, &render_state);
    
    // Render all windows
    let window_stack = compositor.window_stack.clone();
    renderer.render_frame(&window_stack, &mut encoder, &view, output_size)?;
    
    // Present
    frame.present();
}
```

---

## Testing Strategy

### Unit Tests

**File**: `tests/window_stack_tests.rs`

```rust
#[test]
fn test_window_stack_ordering() { /* ... */ }

#[test]
fn test_window_removal() { /* ... */ }

#[test]
fn test_raise_lower() { /* ... */ }

#[test]
fn test_concurrent_access() {
    // Test thread-safe access
}
```

### Integration Tests

**File**: `tests/multi_window_rendering.rs`

```rust
#[test]
fn test_two_window_rendering() {
    // Create 2 windows
    // Verify both render
    // Check Z-order
}

#[test]
fn test_window_focus_changes_order() {
    // Create 3 windows
    // Focus middle window
    // Verify it moves to top
}

#[test]
fn test_window_close_doesnt_affect_others() {
    // Create 3 windows
    // Close middle one
    // Verify others still render
}
```

### Manual Tests

1. **Visual Test**: Run multiple SHM test clients
2. **Performance Test**: Open 20+ windows, check FPS
3. **Stress Test**: Rapidly create/destroy windows
4. **Focus Test**: Click between windows rapidly

---

## Performance Targets

### Frame Rate
- **2-5 windows**: Solid 60 FPS
- **6-10 windows**: 60 FPS (may drop to 58-59 occasionally)
- **11-20 windows**: 45-60 FPS
- **20+ windows**: 30+ FPS acceptable

### Memory
- **Per window overhead**: < 1 MB (texture + metadata)
- **Total overhead**: Linear with window count
- **Texture reuse**: Active for same-size windows

### CPU Usage
- **Idle (no updates)**: < 1%
- **Active rendering**: 5-15% (single core)
- **Window updates**: Spike to 20-30% acceptable

---

## Risk Assessment

### Low Risk ✅
- Window stack data structure (straightforward)
- Render loop iteration (simple change)
- Z-order management (well-defined)

### Medium Risk ⚠️
- Performance with many windows (needs testing)
- Thread synchronization (careful locking needed)
- Memory management (texture pool complexity)

### Mitigation Strategies
1. **Performance**: Implement damage tracking, occlusion culling
2. **Threading**: Minimize lock duration, use lock-free where possible
3. **Memory**: Aggressive texture reuse, garbage collection

---

## Success Criteria

### Must Have
- [ ] Render 2+ windows simultaneously
- [ ] Correct Z-ordering (top window on top visually)
- [ ] Focus changes raise window to top
- [ ] Window close doesn't affect others
- [ ] No crashes or panics
- [ ] 60 FPS with 5 windows

### Should Have
- [ ] Efficient texture reuse
- [ ] Damage tracking per window
- [ ] 60 FPS with 10 windows
- [ ] Clean memory management

### Nice to Have
- [ ] Occlusion culling optimization
- [ ] 60 FPS with 20+ windows
- [ ] Smooth window animations

---

## Timeline

### Day 1 (4 hours)
- Implement WindowStack data structure
- Add unit tests
- Integrate with SharedRenderState

### Day 2 (4 hours)
- Update render loop for multiple windows
- Implement per-window rendering
- Test with 2 windows

### Day 3 (3 hours)
- Integrate focus management
- Handle window lifecycle
- Test with multiple windows

### Day 4 (2 hours)
- Performance optimization
- Damage tracking
- Final testing

**Total**: 13 hours (buffer included)

---

## Dependencies

### Prerequisites
- ✅ Phase 6.3 single-window rendering validated
- ✅ SHM test clients working
- ✅ Proper display environment available

### Blocks
- None (can proceed immediately after validation)

### Blocked By
- Visual validation completion (to test)

---

## Documentation Updates

After completion, update:
1. `PHASE_6_3_PROGRESS.md` - Mark multi-window complete
2. `README.md` - Add multi-window support note
3. `tests/README_SHM_TESTING.md` - Add multi-window test instructions
4. Create `MULTI_WINDOW_SUCCESS_REPORT.md`

---

## Conclusion

Multi-window support is a natural extension of the single-window rendering pipeline. The implementation is straightforward, with well-defined data structures and clear integration points.

**Estimated Effort**: 8-12 hours  
**Confidence**: ⭐⭐⭐⭐⭐ Very High  
**Risk**: 🟢 Low  
**Priority**: HIGH (critical for usable compositor)

**Next Step**: Implement WindowStack data structure after visual validation completes.

---

**Status**: 📋 Planning Complete  
**Ready to Implement**: ✅ Yes (after visual validation)  
**Dependencies**: Clear  
**Confidence**: Very High