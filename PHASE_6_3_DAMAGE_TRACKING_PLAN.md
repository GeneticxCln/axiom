# Phase 6.3: Damage Tracking Optimization Implementation Plan

**Status**: 🔄 Planning  
**Priority**: MEDIUM-HIGH  
**Prerequisites**: Single-window rendering working  
**Estimated Time**: 4-6 hours  
**Target Completion**: After visual validation + multi-window integration

---

## Executive Summary

Damage tracking is a critical performance optimization that allows the compositor to only re-render portions of the screen that have actually changed, rather than redrawing everything every frame. This dramatically reduces GPU workload and improves battery life, especially when most of the screen is static.

**Goal**: Implement per-window damage tracking with region-based updates, achieving 60 FPS with minimal CPU/GPU usage when content is static.

**Expected Performance Gains**:
- Static screen: < 1% CPU usage (vs. 5-10% without damage tracking)
- Single window updating: Render only changed regions (10-50x speedup)
- Multiple windows: Skip windows with no changes (5-20x speedup)

---

## What is Damage Tracking?

### Concept

**Without Damage Tracking**:
```
Every frame:
  - Clear entire screen
  - Render all windows completely
  - Present to display
Result: Wastes GPU resources on unchanged content
```

**With Damage Tracking**:
```
Every frame:
  - Check which windows have damage
  - Only render damaged regions
  - Skip unchanged windows
  - Present only changed screen areas
Result: GPU only processes what changed
```

### Real-World Example

```
Desktop with 5 windows:
- 4 windows static (terminal, browser, file manager, etc.)
- 1 window animating (video player)

Without damage tracking:
  → Render all 5 windows every frame (100% work)

With damage tracking:
  → Render only the video player window (20% work)
  → 5x performance improvement!
```

---

## Current State Analysis

### What We Have ✅

**In Renderer** (`src/renderer/mod.rs`):
- `damage_regions: Vec<DamageRegion>` - Empty vec in RenderedWindow
- Partial damage tracking infrastructure exists
- Region-based texture updates supported

**In Smithay Server** (`src/smithay/server.rs`):
- Buffer commit handling
- Surface damage tracking (basic)
- Frame callbacks

### What's Missing ❌

1. **Proper Damage Accumulation**
   - Not tracking which windows have changed
   - Not accumulating damage across frames
   - Not propagating damage from clients

2. **Region-Based Rendering**
   - Render pass covers entire screen
   - No scissor rectangles for damaged regions
   - No optimization for unchanged windows

3. **Damage Reset Logic**
   - No clear point where damage is cleared
   - No tracking of "clean" vs "dirty" state

4. **Output Damage**
   - Not tracking which screen regions changed
   - Not optimizing presentation

---

## Architecture Design

### Core Concepts

#### 1. Damage Region

```rust
/// Represents a rectangular region that needs repainting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRegion {
    /// X coordinate (pixels)
    pub x: i32,
    /// Y coordinate (pixels)
    pub y: i32,
    /// Width (pixels)
    pub width: u32,
    /// Height (pixels)
    pub height: u32,
}

impl DamageRegion {
    /// Create a new damage region
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }
    
    /// Check if this region intersects another
    pub fn intersects(&self, other: &DamageRegion) -> bool {
        !(self.x + self.width as i32 <= other.x
            || other.x + other.width as i32 <= self.x
            || self.y + self.height as i32 <= other.y
            || other.y + other.height as i32 <= self.y)
    }
    
    /// Compute union of two regions (bounding box)
    pub fn union(&self, other: &DamageRegion) -> DamageRegion {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.width as i32).max(other.x + other.width as i32);
        let y2 = (self.y + self.height as i32).max(other.y + other.height as i32);
        
        DamageRegion {
            x: x1,
            y: y1,
            width: (x2 - x1) as u32,
            height: (y2 - y1) as u32,
        }
    }
    
    /// Convert to screen coordinates given window position
    pub fn to_screen_coords(&self, window_x: i32, window_y: i32) -> DamageRegion {
        DamageRegion {
            x: self.x + window_x,
            y: self.y + window_y,
            width: self.width,
            height: self.height,
        }
    }
}
```

#### 2. Window Damage State

```rust
/// Tracks damage state for a single window
pub struct WindowDamage {
    /// Window ID
    pub window_id: u64,
    
    /// Damaged regions in window coordinates
    pub regions: Vec<DamageRegion>,
    
    /// Is the entire window damaged?
    pub full_damage: bool,
    
    /// Frame number when damage was added
    pub frame_number: u64,
}

impl WindowDamage {
    /// Add damage to a specific region
    pub fn add_region(&mut self, region: DamageRegion) {
        if self.full_damage {
            return; // Already fully damaged
        }
        
        // TODO: Optimize by merging overlapping regions
        self.regions.push(region);
        
        // If too many regions, mark as fully damaged
        if self.regions.len() > 16 {
            self.full_damage = true;
            self.regions.clear();
        }
    }
    
    /// Mark entire window as damaged
    pub fn mark_full(&mut self) {
        self.full_damage = true;
        self.regions.clear();
    }
    
    /// Clear all damage
    pub fn clear(&mut self) {
        self.full_damage = false;
        self.regions.clear();
    }
    
    /// Check if window has any damage
    pub fn has_damage(&self) -> bool {
        self.full_damage || !self.regions.is_empty()
    }
}
```

#### 3. Frame Damage Accumulator

```rust
/// Accumulates damage across all windows for a frame
pub struct FrameDamage {
    /// Per-window damage
    pub window_damage: HashMap<u64, WindowDamage>,
    
    /// Output damage in screen coordinates
    pub output_regions: Vec<DamageRegion>,
    
    /// Current frame number
    pub frame_number: u64,
}

impl FrameDamage {
    /// Add damage for a specific window
    pub fn add_window_damage(&mut self, window_id: u64, region: DamageRegion) {
        let damage = self.window_damage.entry(window_id)
            .or_insert_with(|| WindowDamage {
                window_id,
                regions: Vec::new(),
                full_damage: false,
                frame_number: self.frame_number,
            });
        
        damage.add_region(region);
    }
    
    /// Mark entire window as damaged
    pub fn mark_window_damaged(&mut self, window_id: u64) {
        let damage = self.window_damage.entry(window_id)
            .or_insert_with(|| WindowDamage {
                window_id,
                regions: Vec::new(),
                full_damage: true,
                frame_number: self.frame_number,
            });
        
        damage.mark_full();
    }
    
    /// Compute output damage from window damage
    pub fn compute_output_damage(&mut self, windows: &HashMap<u64, RenderedWindow>) {
        self.output_regions.clear();
        
        for (window_id, damage) in &self.window_damage {
            if !damage.has_damage() {
                continue;
            }
            
            if let Some(window) = windows.get(window_id) {
                if damage.full_damage {
                    // Entire window damaged
                    let region = DamageRegion::new(
                        window.position.0 as i32,
                        window.position.1 as i32,
                        window.size.0 as u32,
                        window.size.1 as u32,
                    );
                    self.output_regions.push(region);
                } else {
                    // Specific regions damaged
                    for region in &damage.regions {
                        let screen_region = region.to_screen_coords(
                            window.position.0 as i32,
                            window.position.1 as i32,
                        );
                        self.output_regions.push(screen_region);
                    }
                }
            }
        }
        
        // TODO: Merge overlapping regions
    }
    
    /// Clear damage after rendering
    pub fn clear(&mut self) {
        self.window_damage.clear();
        self.output_regions.clear();
        self.frame_number += 1;
    }
}
```

---

## Implementation Steps

### Step 1: Core Damage Types (1 hour)

**Tasks**:
1. Create `src/renderer/damage.rs` module
2. Implement `DamageRegion` struct with geometry operations
3. Implement `WindowDamage` struct
4. Implement `FrameDamage` accumulator
5. Add unit tests for damage operations

**Deliverables**:
- `damage.rs` module (~300 lines)
- Geometry operations (intersection, union, etc.)
- Unit tests (~150 lines)

**Success Criteria**:
```rust
#[test]
fn test_damage_region_intersection() {
    let r1 = DamageRegion::new(0, 0, 100, 100);
    let r2 = DamageRegion::new(50, 50, 100, 100);
    assert!(r1.intersects(&r2));
}

#[test]
fn test_damage_region_union() {
    let r1 = DamageRegion::new(0, 0, 100, 100);
    let r2 = DamageRegion::new(50, 50, 100, 100);
    let union = r1.union(&r2);
    assert_eq!(union, DamageRegion::new(0, 0, 150, 150));
}
```

### Step 2: Damage Tracking in Compositor (1.5 hours)

**Tasks**:
1. Add damage tracking to surface commit handler
2. Extract damage from Wayland surface damage
3. Propagate damage to renderer
4. Handle full window damage on first commit

**Changes in `src/smithay/server.rs`**:

```rust
fn handle_surface_commit(&mut self, surface: &WlSurface) {
    with_states(surface, |states| {
        let window_id = /* get window id from surface */;
        
        // Get damage from surface
        let damage = states.cached_state.current::<SurfaceAttributes>().damage;
        
        if damage.is_empty() {
            // First commit or no damage specified - mark entire window damaged
            self.frame_damage.mark_window_damaged(window_id);
        } else {
            // Add each damage region
            for rect in damage {
                let region = DamageRegion::new(
                    rect.x,
                    rect.y,
                    rect.width as u32,
                    rect.height as u32,
                );
                self.frame_damage.add_window_damage(window_id, region);
            }
        }
    });
}
```

**Deliverables**:
- Damage propagation from Wayland to renderer
- Integration with surface commit
- Proper damage tracking for new windows

### Step 3: Damage-Aware Rendering (2 hours)

**Tasks**:
1. Update render loop to check for damage
2. Skip rendering windows with no damage
3. Use scissor rectangles for partial damage
4. Clear damage after successful render

**Changes in `src/renderer/mod.rs`**:

```rust
pub fn render_frame_with_damage(
    &mut self,
    window_stack: &WindowStack,
    frame_damage: &mut FrameDamage,
    encoder: &mut CommandEncoder,
    view: &TextureView,
    output_size: (u32, u32),
) -> Result<()> {
    // Compute output damage from window damage
    frame_damage.compute_output_damage(&self.windows);
    
    // If no damage, skip rendering
    if frame_damage.output_regions.is_empty() {
        debug!("No damage, skipping frame render");
        return Ok(());
    }
    
    // Begin render pass
    let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("damage-aware-render"),
        color_attachments: &[/* ... */],
        depth_stencil_attachment: None,
    });
    
    // Render each window that has damage
    for &window_id in window_stack.render_order() {
        // Check if this window has damage
        if let Some(damage) = frame_damage.window_damage.get(&window_id) {
            if !damage.has_damage() {
                continue; // Skip clean windows
            }
            
            if let Some(window) = self.windows.get(&window_id) {
                if damage.full_damage {
                    // Render entire window
                    self.render_window_internal(&mut render_pass, window)?;
                } else {
                    // Render only damaged regions using scissor
                    for region in &damage.regions {
                        self.render_window_region(
                            &mut render_pass,
                            window,
                            region,
                        )?;
                    }
                }
            }
        }
    }
    
    drop(render_pass);
    
    // Clear damage after rendering
    frame_damage.clear();
    
    Ok(())
}

fn render_window_region(
    &self,
    render_pass: &mut RenderPass,
    window: &RenderedWindow,
    region: &DamageRegion,
) -> Result<()> {
    // Convert to screen coordinates
    let screen_region = region.to_screen_coords(
        window.position.0 as i32,
        window.position.1 as i32,
    );
    
    // Set scissor rectangle
    render_pass.set_scissor_rect(
        screen_region.x as u32,
        screen_region.y as u32,
        screen_region.width,
        screen_region.height,
    );
    
    // Render window (will be clipped to scissor rect)
    render_pass.set_bind_group(0, &window.bind_group.as_ref().unwrap(), &[]);
    render_pass.draw_indexed(0..6, 0, 0..1);
    
    Ok(())
}
```

**Deliverables**:
- Damage-aware render loop
- Scissor rectangle usage
- Skip clean windows optimization

### Step 4: Integration with Main Loop (0.5 hours)

**Tasks**:
1. Add FrameDamage to main event loop
2. Pass damage info to renderer
3. Handle initial full-screen damage
4. Ensure damage survives across frames

**Changes in `src/bin/run_present_winit.rs`**:

```rust
let mut frame_damage = FrameDamage::new();

loop {
    // Process events...
    
    // Process texture updates
    process_pending_texture_updates(&mut renderer, &render_state);
    
    // Render frame with damage tracking
    renderer.render_frame_with_damage(
        &window_stack,
        &mut frame_damage,
        &mut encoder,
        &view,
        output_size,
    )?;
    
    // Present
    frame.present();
}
```

### Step 5: Testing and Validation (1 hour)

**Test Scenarios**:

1. **Static Content Test**
   ```bash
   # Open window, don't interact
   # Expected: After initial render, < 1% CPU
   ./shm_test_client &
   sleep 2
   top -p $(pidof run_present_winit)
   ```

2. **Partial Update Test**
   ```bash
   # Window with small animated region
   # Expected: Only render changed region
   ```

3. **Multi-Window Test**
   ```bash
   # Multiple windows, only one updating
   # Expected: Only render updating window
   ./shm_test_client &
   ./shm_test_client &
   # Update only one
   ```

4. **Full Damage Test**
   ```bash
   # New window appears
   # Expected: Full window damage on first frame
   ```

**Deliverables**:
- Test suite for damage tracking
- Performance validation
- CPU usage measurements

---

## Performance Expectations

### Baseline (Without Damage Tracking)

- **Static screen**: 5-10% CPU (constantly rendering)
- **One updating window**: 100% of work (render all windows)
- **GPU**: Constant load even when idle

### With Damage Tracking

- **Static screen**: < 1% CPU (no rendering)
- **One updating window**: 20-50% of work (render only that window)
- **GPU**: Proportional to actual changes

### Measurements

| Scenario | CPU Usage | GPU Usage | FPS |
|----------|-----------|-----------|-----|
| Static (no damage) | < 1% | 0% | 0 |
| Small region updating | 2-5% | 10-30% | 60 |
| Full window updating | 5-10% | 50-80% | 60 |
| Multiple windows, one updating | 3-8% | 20-40% | 60 |

---

## Integration Points

### 1. Smithay Server

**Location**: `src/smithay/server.rs`

```rust
pub struct AxiomCompositor {
    // ... existing fields
    
    /// Frame damage accumulator
    pub frame_damage: Arc<Mutex<FrameDamage>>,
}

impl AxiomCompositor {
    fn handle_commit(&mut self, surface: &WlSurface) {
        // Extract damage from surface
        let damage_regions = self.get_surface_damage(surface);
        
        // Add to frame damage
        let window_id = self.get_window_id(surface);
        let mut frame_damage = self.frame_damage.lock().unwrap();
        
        if damage_regions.is_empty() {
            frame_damage.mark_window_damaged(window_id);
        } else {
            for region in damage_regions {
                frame_damage.add_window_damage(window_id, region);
            }
        }
    }
}
```

### 2. Renderer

**Location**: `src/renderer/mod.rs`

```rust
impl AxiomRenderer {
    pub fn render_frame_with_damage(
        &mut self,
        window_stack: &WindowStack,
        frame_damage: &mut FrameDamage,
        /* ... */
    ) -> Result<()> {
        // Check for damage
        if !frame_damage.has_any_damage() {
            return Ok(()); // Nothing to render
        }
        
        // Render damaged windows
        // ...
    }
}
```

### 3. Main Loop

**Location**: `src/bin/run_present_winit.rs`

```rust
// Shared damage state
let frame_damage = Arc::new(Mutex::new(FrameDamage::new()));

// Pass to compositor
compositor.set_frame_damage(frame_damage.clone());

// Use in render loop
loop {
    let mut damage = frame_damage.lock().unwrap();
    renderer.render_frame_with_damage(&window_stack, &mut damage, /* ... */)?;
}
```

---

## Optimization Strategies

### 1. Region Merging

**Problem**: Many small damage regions create overhead

**Solution**: Merge overlapping/adjacent regions

```rust
fn merge_regions(regions: &mut Vec<DamageRegion>) {
    // Sort by position
    regions.sort_by_key(|r| (r.y, r.x));
    
    let mut merged = Vec::new();
    let mut current = regions[0];
    
    for region in &regions[1..] {
        if current.intersects(region) || current.is_adjacent(region) {
            // Merge
            current = current.union(region);
        } else {
            merged.push(current);
            current = *region;
        }
    }
    merged.push(current);
    
    *regions = merged;
}
```

### 2. Damage Age Tracking

Track how old damage is to prioritize recent changes:

```rust
pub struct WindowDamage {
    pub regions: Vec<DamageRegion>,
    pub frame_number: u64, // When damage was added
}

// Skip very old damage (already rendered)
if damage.frame_number < current_frame - 2 {
    continue;
}
```

### 3. Coalescing Threshold

If too many regions, treat as full damage:

```rust
const MAX_DAMAGE_REGIONS: usize = 16;

if damage.regions.len() > MAX_DAMAGE_REGIONS {
    damage.mark_full();
}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_damage_accumulation() {
        let mut damage = WindowDamage::new(1);
        damage.add_region(DamageRegion::new(0, 0, 10, 10));
        damage.add_region(DamageRegion::new(20, 20, 10, 10));
        
        assert_eq!(damage.regions.len(), 2);
        assert!(!damage.full_damage);
    }
    
    #[test]
    fn test_full_damage_on_too_many_regions() {
        let mut damage = WindowDamage::new(1);
        
        for i in 0..20 {
            damage.add_region(DamageRegion::new(i * 10, 0, 10, 10));
        }
        
        assert!(damage.full_damage);
        assert!(damage.regions.is_empty());
    }
}
```

### Integration Tests

1. Static content test (verify no rendering)
2. Partial update test (verify only changed region)
3. Multi-window test (verify selective rendering)
4. Performance test (measure CPU/GPU usage)

### Visual Tests

1. Run SHM client with static content → verify no flicker
2. Run animated client → verify smooth updates
3. Multiple windows → verify correct damage propagation

---

## Success Criteria

### Must Have
- [ ] Core damage types implemented
- [ ] Damage propagation from Wayland to renderer
- [ ] Skip rendering clean windows
- [ ] Static screen: < 1% CPU usage
- [ ] Correct visual output (no artifacts)

### Should Have
- [ ] Scissor rectangle optimization
- [ ] Region merging
- [ ] Damage age tracking
- [ ] Performance metrics

### Nice to Have
- [ ] Advanced region optimization
- [ ] Damage visualization (debug mode)
- [ ] Per-window damage statistics

---

## Timeline

### Day 1 (2 hours)
- Implement core damage types
- Add unit tests
- Basic geometry operations

### Day 2 (2 hours)
- Integrate with compositor
- Damage propagation
- Basic render loop integration

### Day 3 (1.5 hours)
- Scissor rectangles
- Performance optimization
- Region merging

### Day 4 (0.5 hours)
- Testing and validation
- Performance measurements
- Bug fixes

**Total**: 6 hours (with buffer)

---

## Dependencies

### Prerequisites
- ✅ Single-window rendering working
- ✅ Multi-window WindowStack implemented
- ⏳ Visual validation complete

### Blocks
- None (can be implemented in parallel with multi-window integration)

### Blocked By
- Visual validation (for testing)

---

## Risk Assessment

### Low Risk ✅
- Core damage tracking logic (straightforward)
- Integration points well-defined
- Similar implementations exist in other compositors

### Medium Risk ⚠️
- Performance tuning (may need iteration)
- Edge cases in region merging
- Interaction with effects (future work)

### Mitigation
- Comprehensive unit tests
- Start simple, optimize later
- Performance benchmarks
- Can be disabled if issues arise

---

## Future Enhancements

### After Initial Implementation

1. **Damage Visualization** (Debug Mode)
   - Highlight damaged regions
   - Show damage statistics
   - Performance overlay

2. **Smart Region Merging**
   - Better algorithms for region optimization
   - Predictive damage tracking
   - Adaptive thresholds

3. **Damage History**
   - Track damage patterns
   - Optimize based on history
   - Predict future damage

4. **Integration with Effects**
   - Blur needs larger damage regions
   - Shadows extend damage area
   - Proper handling of effect boundaries

---

## Conclusion

Damage tracking is a critical performance optimization that will dramatically improve the compositor's efficiency. The implementation is straightforward with well-defined integration points and clear success criteria.

**Estimated Effort**: 4-6 hours  
**Confidence**: ⭐⭐⭐⭐⭐ Very High  
**Risk**: 🟢 Low  
**Priority**: MEDIUM-HIGH (important for production)

**Next Step**: Implement core damage types after multi-window integration or in parallel.

---

**Status**: 📋 Planning Complete  
**Ready to Implement**: ✅ Yes (after visual validation)  
**Dependencies**: Clear  
**Confidence**: Very High