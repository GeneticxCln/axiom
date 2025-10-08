# Phase 6.3: Rendering Pipeline - Implementation Plan

**Date**: October 5, 2025  
**Status**: Ready to Begin  
**Duration Estimate**: 2-3 weeks  
**Priority**: P0 - Critical Path  

---

## Executive Summary

Phase 6.3 is the **final major component** needed for Axiom production release. The Wayland protocol layer is complete and functional - clients can connect, create windows, and interact. The only remaining work is to actually **display the window content** on screen.

**Current State**: Buffer data is received, processed, and queued - but not rendered.  
**Goal**: Render actual window content using GPU acceleration.  
**Complexity**: Medium - Infrastructure exists, need to wire it together.

---

## Table of Contents

1. [Current State Analysis](#current-state-analysis)
2. [What Needs to Be Done](#what-needs-to-be-done)
3. [Implementation Roadmap](#implementation-roadmap)
4. [Technical Details](#technical-details)
5. [Testing Strategy](#testing-strategy)
6. [Timeline & Milestones](#timeline--milestones)
7. [Risk Assessment](#risk-assessment)
8. [Success Criteria](#success-criteria)

---

## Current State Analysis

### ✅ What's Already Working

**Buffer Processing**:
- ✅ SHM buffers received from clients
- ✅ DMA-BUF support implemented
- ✅ Buffer format conversion (ARGB, XRGB, etc.)
- ✅ Viewport support for scaling
- ✅ Damage tracking collected
- ✅ Buffer data converted to RGBA

**Renderer Infrastructure**:
- ✅ wgpu device and queue initialized
- ✅ Render pipeline structure exists
- ✅ Texture pool management
- ✅ Uniform buffer handling
- ✅ Vertex buffer generation
- ✅ Bind group creation

**Data Flow**:
```
Client Buffer → BufferRecord → convert_shm_to_rgba() → 
queue_texture_update() → SharedRenderState → [MISSING: Upload to GPU]
```

### ❌ What's Missing

The gap is in **3 specific locations**:

1. **Texture Upload**: `queue_texture_update()` stores data in memory but doesn't upload to GPU
2. **Texture Application**: Pending textures aren't applied to windows
3. **Actual Rendering**: `render_to_surface()` has structure but no real drawing

---

## What Needs to Be Done

### Core Tasks (Must Have)

#### Task 1: Texture Upload Pipeline
**File**: `src/renderer/mod.rs`  
**Function**: New `process_pending_texture_updates()`  
**What**: Take queued RGBA data and upload to GPU textures

```rust
// Pseudocode
fn process_pending_texture_updates(&mut self) {
    for (id, data, width, height) in pending_updates {
        // Find or create texture
        let texture = self.get_or_create_texture(id, width, height);
        
        // Upload data to GPU
        self.queue.write_texture(
            texture.as_image_copy(),
            &data,
            ImageDataLayout { ... },
            Extent3d { width, height, depth: 1 }
        );
        
        // Update window's texture reference
        if let Some(window) = self.find_window_mut(id) {
            window.texture = Some(texture);
            window.texture_view = Some(texture.create_view(...));
            window.dirty = true;
        }
    }
}
```

**Estimated Time**: 4-6 hours

#### Task 2: Bind Group Updates
**File**: `src/renderer/mod.rs`  
**Function**: Update `render_to_surface_with_outputs_scaled()`  
**What**: Create bind groups for textured windows

```rust
// For each window with a texture:
if let Some(view) = &window.texture_view {
    let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
        layout: &self.bind_group_layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            BindGroupEntry { binding: 1, resource: BindingResource::TextureView(view) },
            BindGroupEntry { binding: 2, resource: BindingResource::Sampler(&self.sampler) },
        ],
    });
    window.bind_group = Some(bind_group);
}
```

**Estimated Time**: 2-3 hours

#### Task 3: Actual Draw Calls
**File**: `src/renderer/mod.rs`  
**Function**: Complete `render_to_surface_with_outputs_scaled()`  
**What**: Execute actual GPU draw commands

```rust
{
    let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &view,
            ops: Operations {
                load: LoadOp::Clear(Color::BLACK),
                store: StoreOp::Store,
            },
            ...
        })],
        ...
    });
    
    render_pass.set_pipeline(&pipeline);
    
    for window in &self.windows {
        if let Some(bind_group) = &window.bind_group {
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint16);
            render_pass.draw_indexed(0..6, 0, 0..1);
        }
    }
}

self.queue.submit(std::iter::once(encoder.finish()));
```

**Estimated Time**: 4-6 hours

#### Task 4: Integration with Main Loop
**File**: `src/compositor.rs` or presenter code  
**What**: Call texture processing and rendering at right times

```rust
// In main render loop:
renderer.process_pending_texture_updates()?;
renderer.render_to_surface(&surface, &frame)?;
```

**Estimated Time**: 2-3 hours

---

## Implementation Roadmap

### Week 1: Core Rendering (Days 1-5)

#### Day 1: Setup & Planning
- [x] Review current renderer code thoroughly
- [x] Document exact data flow
- [x] Create implementation plan (this document)
- [ ] Set up test cases

#### Day 2: Texture Upload
- [ ] Implement `process_pending_texture_updates()`
- [ ] Add texture creation/reuse from pool
- [ ] Test with single static image
- [ ] Verify GPU memory usage

#### Day 3: Bind Groups & Uniforms
- [ ] Update bind group creation for textured windows
- [ ] Implement projection matrix calculations
- [ ] Add window-specific uniforms (position, size, opacity)
- [ ] Test with colored rectangles

#### Day 4: Draw Calls
- [ ] Complete render pass setup
- [ ] Implement vertex/index buffer generation
- [ ] Add actual draw commands
- [ ] Test with on-screen window

#### Day 5: Integration & Testing
- [ ] Wire up with main compositor loop
- [ ] Test with real Wayland client (alacritty)
- [ ] Debug any rendering issues
- [ ] Verify basic window display works

### Week 2: Optimization & Features (Days 6-10)

#### Day 6: Damage Tracking
- [ ] Implement partial texture updates
- [ ] Use damage regions to minimize uploads
- [ ] Test with scrolling/text editing
- [ ] Profile performance improvements

#### Day 7: Multi-Window Support
- [ ] Test with multiple overlapping windows
- [ ] Implement Z-ordering
- [ ] Add window stacking/focus visual feedback
- [ ] Test with 5+ windows

#### Day 8: Effects Integration
- [ ] Add blur shader for backgrounds
- [ ] Implement rounded corners
- [ ] Add drop shadows
- [ ] Test visual quality

#### Day 9: Performance Optimization
- [ ] Profile GPU usage
- [ ] Optimize texture uploads
- [ ] Reduce draw calls where possible
- [ ] Add frame pacing

#### Day 10: Bug Fixes & Polish
- [ ] Fix any visual glitches
- [ ] Handle edge cases (zero-size windows, etc.)
- [ ] Memory leak checking
- [ ] Documentation updates

### Week 3: Testing & Validation (Days 11-15)

#### Day 11-12: Application Testing
- [ ] Test with Firefox/Chrome
- [ ] Test with VSCode
- [ ] Test with multiple terminals
- [ ] Test with video players

#### Day 13: Stress Testing
- [ ] Test with 20+ windows
- [ ] Long-running stability (24+ hours)
- [ ] Memory leak detection
- [ ] Performance under load

#### Day 14: Visual Quality
- [ ] Screenshot comparison with other compositors
- [ ] Verify effects look good
- [ ] Test on different monitor types
- [ ] Color accuracy validation

#### Day 15: Documentation & Cleanup
- [ ] Update README with rendering status
- [ ] Document performance characteristics
- [ ] Code cleanup and comments
- [ ] Create demo video/screenshots

---

## Technical Details

### Texture Format

**Client Buffers**: Various formats (ARGB8888, XRGB8888, etc.)  
**Internal Format**: RGBA8UnormSrgb (already converted)  
**GPU Texture**: `TextureFormat::Rgba8UnormSrgb`

### Coordinate Systems

**Wayland**: Top-left origin, Y-down  
**wgpu**: Center origin, Y-up for normalized device coordinates  
**Conversion**: Already handled in vertex generation

### Shader Pipeline

**Vertex Shader**: Exists at `assets/shaders/window.vert`  
**Fragment Shader**: Exists at `assets/shaders/window.frag`  
**Uniforms**: MVP matrix, opacity, corner radius

### Memory Management

**Texture Pool**: Reuse textures of same size to avoid allocations  
**Buffer Pool**: Reuse uniform buffers across frames  
**Upload Strategy**: Use `write_texture` for simplicity, staging buffers for optimization

---

## Testing Strategy

### Unit Tests

```rust
#[test]
fn test_texture_upload() {
    let mut renderer = AxiomRenderer::new_headless().await.unwrap();
    let data = vec![255u8; 4 * 100 * 100]; // 100x100 white image
    renderer.upload_texture(1, data, 100, 100).unwrap();
    assert!(renderer.find_window(1).unwrap().texture.is_some());
}

#[test]
fn test_multiple_windows() {
    let mut renderer = AxiomRenderer::new_headless().await.unwrap();
    renderer.add_window(1, (0.0, 0.0), (800.0, 600.0)).unwrap();
    renderer.add_window(2, (100.0, 100.0), (400.0, 300.0)).unwrap();
    assert_eq!(renderer.windows.len(), 2);
}
```

### Integration Tests

1. **Single Window Test**: Launch alacritty, verify content displays
2. **Multi-Window Test**: Launch 3 terminals, verify all display
3. **Overlap Test**: Move windows over each other, verify correct stacking
4. **Damage Test**: Type in terminal, verify only damaged regions update
5. **Performance Test**: Measure FPS with 10 windows

### Visual Tests

- Screenshot comparison with Sway/Hyprland
- Video recording of animations
- Manual inspection of effects quality

### Performance Benchmarks

- Frame time: Target <16ms (60 FPS)
- Texture upload: <1ms for 1920x1080 image
- Memory usage: <100 MB for 10 windows
- CPU usage: <10% with idle windows

---

## Timeline & Milestones

### Milestone 1: Basic Rendering (End of Week 1)
**Goal**: Single window displays actual content  
**Success Criteria**:
- alacritty launches and displays terminal
- Text is readable
- No crashes or major glitches

### Milestone 2: Multi-Window (End of Week 2)
**Goal**: Multiple windows with effects work  
**Success Criteria**:
- 5+ windows display correctly
- Effects (blur, shadows) render
- Performance acceptable (30+ FPS)

### Milestone 3: Production Ready (End of Week 3)
**Goal**: Ready for real-world use  
**Success Criteria**:
- All major apps tested (Firefox, VSCode, etc.)
- No memory leaks
- 60 FPS with typical workload
- Visual quality matches/exceeds other compositors

---

## Risk Assessment

### Technical Risks: LOW

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| GPU driver issues | Low | Medium | Test on multiple GPUs, fallback options |
| Performance problems | Medium | Medium | Profile early, optimize incrementally |
| Visual glitches | Medium | Low | Extensive testing, easy to fix |
| Memory leaks | Low | Medium | Regular profiling with valgrind |

### Schedule Risks: LOW

- Task estimates include buffer time
- Core team member available full-time
- No external dependencies or blockers
- Clear rollback path if needed

---

## Success Criteria

### Must Have (Required for Release)
- ✅ Windows display actual client content
- ✅ Multiple windows work correctly
- ✅ Basic effects (blur, shadows) render
- ✅ 60 FPS with 5 windows
- ✅ No crashes or hangs
- ✅ Works with major applications

### Should Have (High Priority)
- ✅ Damage tracking optimization
- ✅ Proper Z-ordering
- ✅ Window animations smooth
- ✅ <50 MB memory per window

### Nice to Have (Can Defer)
- Advanced effects (complex shaders)
- Hardware overlay planes
- Zero-copy rendering
- 120+ Hz support

---

## Code Locations Quick Reference

**Key Files**:
- `src/renderer/mod.rs` - Main renderer implementation
- `src/smithay/server.rs` - Buffer processing (lines 1700-1760, 5300-5400)
- `assets/shaders/` - GLSL shaders
- `src/compositor.rs` - Main loop integration

**Key Functions**:
- `queue_texture_update()` - Line 145 in renderer/mod.rs
- `process_with_viewport()` - Line 4025 in smithay/server.rs  
- `render_to_surface_with_outputs_scaled()` - Line 1005 in renderer/mod.rs

**Important Structures**:
- `RenderedWindow` - Line 66 in renderer/mod.rs
- `BufferRecord` - Line 3673 in smithay/server.rs
- `SharedRenderState` - Line 104 in renderer/mod.rs

---

## Next Immediate Steps

### Tomorrow Morning:
1. Read through `src/renderer/mod.rs` completely (1 hour)
2. Study wgpu texture upload examples (30 mins)
3. Create simple test program for texture upload (1 hour)
4. Begin implementing `process_pending_texture_updates()` (2 hours)

### Tomorrow Afternoon:
1. Complete texture upload implementation
2. Add test with solid color texture
3. Verify GPU memory allocation works
4. Begin bind group updates

---

## Resources & References

### wgpu Documentation
- Texture Upload: https://docs.rs/wgpu/latest/wgpu/struct.Queue.html#method.write_texture
- Render Pipeline: https://sotrh.github.io/learn-wgpu/beginner/tutorial3-pipeline/
- Bind Groups: https://sotrh.github.io/learn-wgpu/beginner/tutorial5-textures/

### Smithay Examples
- Anvil Renderer: `/tmp/smithay/anvil-src/drawing.rs`
- Check texture handling and composition patterns

### Testing Tools
- `valgrind --leak-check=full` - Memory leak detection
- `perf record -F 99 -g` - Performance profiling
- `renderdoc` - GPU debugging (if needed)

---

## Communication Plan

### Daily Updates
- Morning standup: What's being worked on today
- Evening report: Progress, blockers, next steps
- Update `PHASE_6_3_PROGRESS.md` daily

### Milestone Reports
- End of Week 1: Basic rendering status
- End of Week 2: Multi-window and effects status  
- End of Week 3: Production readiness report

### Demo Schedule
- Day 5: First visual demo (single window)
- Day 10: Multi-window demo with effects
- Day 15: Full production demo

---

## Conclusion

Phase 6.3 is a **well-defined, achievable task** with clear objectives and a realistic timeline. The infrastructure is already in place - we just need to connect the pieces.

**Key Success Factors**:
1. ✅ Clear understanding of what needs to be done
2. ✅ Existing infrastructure to build on
3. ✅ No external blockers or dependencies
4. ✅ Realistic timeline with buffer
5. ✅ Clear success criteria

**Confidence Level**: ⭐⭐⭐⭐⭐ Very High

**Expected Completion**: 2-3 weeks (by October 26, 2025)

---

**Let's build something beautiful! 🎨**