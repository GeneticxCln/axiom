# Phase 6.4: Visual Validation & Damage-Aware Rendering

**Start Date:** December 19, 2024  
**Estimated Duration:** 8-13 days  
**Status:** READY TO BEGIN  
**Dependencies:** Phase 6.3 (92% complete)

---

## Overview

Phase 6.4 focuses on **validating** the rendering pipeline with real applications and **optimizing** performance through damage-aware rendering. This phase transitions Axiom from "code complete" to "production validated."

### Goals

1. ✅ **Visual Validation** - Verify end-to-end rendering works correctly
2. ✅ **Damage-Aware Rendering** - Optimize to only redraw changed regions
3. ✅ **Smithay Integration** - Wire WindowStack/damage calls into protocol handlers
4. ✅ **Performance Validation** - Achieve 60 FPS with multiple windows
5. ✅ **Real Application Testing** - Validate compatibility with common applications

### Success Criteria

- [ ] Window rendering visually verified with test client
- [ ] Multi-window Z-ordering correct on screen
- [ ] Damage-aware rendering reduces CPU/GPU load by 50%+
- [ ] 60 FPS maintained with 10+ windows
- [ ] Real applications (terminal, browser) render correctly
- [ ] No memory leaks in 24-hour stress test
- [ ] Documentation updated with visual validation results

---

## Task 1: Visual Validation (Priority: CRITICAL)

**Estimated Time:** 1-2 days  
**Dependencies:** Display environment (TTY/Xephyr/standalone Wayland)  
**Status:** BLOCKED (awaiting display environment)

### 1.1 Setup Display Environment

#### Option A: TTY with KMS/DRM
```bash
# Switch to TTY (Ctrl+Alt+F2)
# Set environment variables
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=wayland-1

# Run compositor
cd /home/quinton/axiom
cargo run --release --bin run_present_winit
```

**Advantages:**
- Full hardware access
- Best performance
- Real-world scenario

**Disadvantages:**
- Requires physical access
- Can't capture screenshots easily

#### Option B: Xephyr Nested Server
```bash
# Install Xephyr
sudo apt install xserver-xephyr

# Start Xephyr with Wayland backend
Xephyr :1 -ac -screen 1920x1080 &
export DISPLAY=:1

# Run Wayland compositor in nested X
cd /home/quinton/axiom
cargo run --release --bin run_present_winit
```

**Advantages:**
- Works in current environment
- Easy to capture screenshots
- Safe to test

**Disadvantages:**
- Nested overhead
- Not true hardware rendering

#### Option C: Standalone Wayland Session
```bash
# From a login manager or console
# Select "Axiom" session (requires session file)

# /usr/share/wayland-sessions/axiom.desktop:
[Desktop Entry]
Name=Axiom
Comment=Axiom Wayland Compositor
Exec=/usr/local/bin/axiom
Type=Application
```

**Advantages:**
- Production environment
- Full hardware access
- Clean session

**Disadvantages:**
- Requires installation
- Needs session manager integration

### 1.2 Run Visual Validation Tests

#### Execute Automated Test Suite
```bash
cd /home/quinton/axiom
./test_shm_rendering.sh
```

#### Expected Output
```
🧪 Phase 6.3 SHM Rendering Test Suite
=======================================

✅ Step 1: Client connects successfully
✅ Step 2: Client binds to required protocols
✅ Step 3: SHM buffer created (256x256 ARGB)
✅ Step 4: Test pattern drawn to buffer
✅ Step 5: Surface configured by compositor
✅ Step 6: Buffer attached and committed
✅ Step 7: Window visible on screen
✅ Step 8: Correct rendering of test pattern

SUCCESS: All 8 success criteria met! 🎉
```

### 1.3 Visual Verification Checklist

- [ ] Window appears on screen at expected position
- [ ] Color gradient renders correctly (no corruption)
- [ ] Window size matches expected dimensions (256x256)
- [ ] Window decorations appear if configured
- [ ] No flickering or tearing during render
- [ ] Window remains stable (doesn't disappear)

### 1.4 Multi-Window Testing

#### Test Multiple Windows
```bash
# Terminal 1: Start compositor
cargo run --release --bin run_present_winit

# Terminal 2: Start first client
./tests/shm_test_client

# Terminal 3: Start second client
./tests/shm_test_client

# Terminal 4: Start third client
./tests/shm_test_client
```

#### Z-Ordering Verification
- [ ] Windows stack correctly (last created on top)
- [ ] Clicking a window raises it to top
- [ ] Overlapping areas render correctly
- [ ] No Z-fighting or flickering

### 1.5 Document Results

#### Create Visual Validation Report
```markdown
# File: PHASE_6_4_VISUAL_VALIDATION_REPORT.md

## Environment
- Display: TTY/Xephyr/Standalone
- Hardware: GPU model, driver version
- Resolution: 1920x1080

## Test Results
- Test 1: Single window - PASS/FAIL
- Test 2: Multiple windows - PASS/FAIL
- Test 3: Z-ordering - PASS/FAIL

## Screenshots
![Single Window](screenshots/single_window.png)
![Multi Window](screenshots/multi_window.png)

## Issues Found
1. Issue description
2. Steps to reproduce
3. Expected vs. actual behavior
```

---

## Task 2: Damage-Aware Rendering (Priority: HIGH)

**Estimated Time:** 2-3 days  
**Dependencies:** None (can start immediately)  
**Status:** READY TO BEGIN

### 2.1 Implement Scissor Rectangle Optimization

#### Location: `axiom/src/renderer/mod.rs`

#### Step 1: Compute Output Damage Before Rendering

Add to `render_to_surface_with_outputs_scaled()` before building draw commands:

```rust
// After syncing window stack and before building vertices
// Compute output damage regions for scissor optimization
let mut output_damage_regions: Vec<DamageRegion> = Vec::new();
let should_use_damage_optimization = if let Some(ref damage_arc) = self.frame_damage {
    if let Ok(mut damage) = damage_arc.lock() {
        if damage.has_any_damage() {
            // Build window position and size maps
            let mut positions: HashMap<u64, (i32, i32)> = HashMap::new();
            let mut sizes: HashMap<u64, (u32, u32)> = HashMap::new();
            
            for window in &self.windows {
                positions.insert(window.id, (
                    window.position.0 as i32,
                    window.position.1 as i32
                ));
                sizes.insert(window.id, (
                    window.size.0 as u32,
                    window.size.1 as u32
                ));
            }
            
            // Compute output damage
            damage.compute_output_damage(&positions, &sizes);
            output_damage_regions = damage.output_regions().to_vec();
            
            info!("💥 Frame has {} damage regions to render", output_damage_regions.len());
            true
        } else {
            debug!("💥 No damage this frame, skipping render");
            return Ok(());
        }
    } else {
        false
    }
} else {
    false
};
```

#### Step 2: Apply Scissor Rectangles During Rendering

Modify the render pass to use damage regions:

```rust
// In render pass, when rendering windows
if should_use_damage_optimization && !output_damage_regions.is_empty() {
    // Render only damaged regions
    for damage_region in &output_damage_regions {
        let scissor_x = damage_region.x.max(0) as u32;
        let scissor_y = damage_region.y.max(0) as u32;
        let scissor_w = damage_region.width.min(self.size.0.saturating_sub(scissor_x));
        let scissor_h = damage_region.height.min(self.size.1.saturating_sub(scissor_y));
        
        rpass.set_scissor_rect(scissor_x, scissor_y, scissor_w, scissor_h);
        
        // Draw all windows in this damage region
        // (existing draw commands)
    }
} else {
    // Full-frame rendering (fallback)
    rpass.set_scissor_rect(0, 0, self.size.0, self.size.1);
    // (existing draw commands)
}
```

### 2.2 Implement Occlusion Culling

#### Location: `axiom/src/renderer/mod.rs`

Add function to detect fully occluded windows:

```rust
/// Checks if a window is fully occluded by windows above it
fn is_window_occluded(&self, window_id: u64, render_order: &[u64]) -> bool {
    // Find position of this window in Z-order
    let window_pos = render_order.iter().position(|&id| id == window_id);
    if window_pos.is_none() {
        return false;
    }
    
    let window_pos = window_pos.unwrap();
    let window_idx = match self.window_id_to_index.get(&window_id) {
        Some(&idx) => idx,
        None => return false,
    };
    
    let window = &self.windows[window_idx];
    let window_rect = (
        window.position.0 as i32,
        window.position.1 as i32,
        window.size.0 as u32,
        window.size.1 as u32,
    );
    
    // Check all windows above this one
    for &upper_id in &render_order[window_pos + 1..] {
        if let Some(&upper_idx) = self.window_id_to_index.get(&upper_id) {
            let upper = &self.windows[upper_idx];
            
            // Skip if upper window is transparent
            if upper.opacity < 1.0 {
                continue;
            }
            
            // Check if upper window fully covers this window
            let upper_rect = (
                upper.position.0 as i32,
                upper.position.1 as i32,
                upper.size.0 as u32,
                upper.size.1 as u32,
            );
            
            if rect_contains(upper_rect, window_rect) {
                debug!("🚫 Window {} fully occluded by window {}", window_id, upper_id);
                return true;
            }
        }
    }
    
    false
}

/// Helper: Check if rect1 fully contains rect2
fn rect_contains(
    rect1: (i32, i32, u32, u32),
    rect2: (i32, i32, u32, u32),
) -> bool {
    let (x1, y1, w1, h1) = rect1;
    let (x2, y2, w2, h2) = rect2;
    
    x2 >= x1 && y2 >= y1
        && x2 + w2 as i32 <= x1 + w1 as i32
        && y2 + h2 as i32 <= y1 + h1 as i32
}
```

Use occlusion culling in render loop:

```rust
// In render_to_surface_with_outputs_scaled()
for window_id in render_order {
    // Skip if window is fully occluded
    if self.is_window_occluded(window_id, &render_order) {
        debug!("🚫 Skipping occluded window {}", window_id);
        continue;
    }
    
    // Render window (existing code)
    // ...
}
```

### 2.3 Performance Measurement

#### Add Benchmarking Code

```rust
// At start of render_to_surface_with_outputs_scaled()
let render_start = std::time::Instant::now();
let stats_before = RenderStats {
    windows_rendered: 0,
    pixels_drawn: 0,
    draw_calls: 0,
};

// During rendering, track stats
stats.windows_rendered += 1;
stats.pixels_drawn += (window.size.0 * window.size.1) as u64;
stats.draw_calls += 1;

// At end of render
let render_duration = render_start.elapsed();
info!(
    "🎨 Rendered {} windows, {} pixels, {} draw calls in {:.2}ms",
    stats.windows_rendered,
    stats.pixels_drawn,
    stats.draw_calls,
    render_duration.as_secs_f64() * 1000.0
);
```

#### Create Benchmark Script

File: `axiom/benchmark_damage_optimization.sh`

```bash
#!/bin/bash

echo "🔬 Damage-Aware Rendering Benchmark"
echo "===================================="

# Test 1: Full-frame rendering (baseline)
echo "Test 1: Full-frame rendering..."
AXIOM_DISABLE_DAMAGE=1 cargo run --release --bin run_present_winit &
PID=$!
sleep 5
# Measure FPS, CPU, GPU
pkill -P $PID

# Test 2: Damage-aware rendering
echo "Test 2: Damage-aware rendering..."
cargo run --release --bin run_present_winit &
PID=$!
sleep 5
# Measure FPS, CPU, GPU
pkill -P $PID

echo "Results saved to benchmark_results.txt"
```

---

## Task 3: Smithay Handler Integration (Priority: HIGH)

**Estimated Time:** 3-5 days  
**Dependencies:** None  
**Status:** READY TO BEGIN

### 3.1 Wire WindowStack Calls

#### Location: `axiom/src/smithay/server.rs`

#### Add to Surface Commit Handler

Find the `wl_surface::commit` handler and add:

```rust
impl Dispatch<wl_surface::WlSurface, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        surface: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Commit => {
                let surface_id = surface.id().protocol_id();
                
                // Find window for this surface
                if let Some(window) = state.windows.iter().find(|w| w.surface_id == surface_id) {
                    let window_id = window.window_id;
                    
                    // Add to window stack if not already there
                    crate::renderer::add_window_to_stack(window_id);
                    
                    // Mark window as damaged (full redraw)
                    crate::renderer::mark_window_damaged(window_id);
                    
                    debug!("📝 Surface {} committed, window {} added to stack and damaged", 
                           surface_id, window_id);
                }
                
                // Existing commit handling...
            }
            // ... other requests
        }
    }
}
```

#### Add to XDG Toplevel Handler

```rust
impl Dispatch<xdg_toplevel::XdgToplevel, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        toplevel: &xdg_toplevel::XdgToplevel,
        request: xdg_toplevel::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_toplevel::Request::SetAppId { app_id } => {
                // Find window and update
                if let Some(window) = state.find_window_by_toplevel(toplevel) {
                    window.app_id = Some(app_id.clone());
                    
                    // Ensure window is in stack
                    crate::renderer::add_window_to_stack(window.window_id);
                }
            }
            
            // ... other requests
        }
    }
}
```

#### Add to Window Activation

```rust
// When window receives focus
pub fn focus_window(&mut self, window_id: u64) {
    // Raise to top of Z-order
    crate::renderer::raise_window_to_top(window_id);
    
    // Update focus state
    self.focused_window_id = Some(window_id);
    
    // Send keyboard enter events
    // ...
    
    debug!("🎯 Focused window {} and raised to top", window_id);
}
```

#### Add to Window Destruction

```rust
// When window is destroyed
pub fn remove_window(&mut self, window_id: u64) {
    // Remove from window stack
    crate::renderer::remove_window_from_stack(window_id);
    
    // Remove from compositor state
    self.windows.retain(|w| w.window_id != window_id);
    
    debug!("🗑️ Removed window {} from stack and state", window_id);
}
```

### 3.2 Wire Damage Tracking Calls

#### Partial Updates (Optional Optimization)

If client provides damage regions via `wl_surface.damage` or `wl_surface.damage_buffer`:

```rust
wl_surface::Request::Damage { x, y, width, height } => {
    let surface_id = surface.id().protocol_id();
    
    if let Some(window) = state.windows.iter().find(|w| w.surface_id == surface_id) {
        // Add specific damage region
        crate::renderer::add_window_damage_region(
            window.window_id,
            x,
            y,
            width as u32,
            height as u32
        );
        
        debug!("💥 Added damage region {}x{} at ({},{}) for window {}", 
               width, height, x, y, window.window_id);
    }
}
```

---

## Task 4: Real Application Testing (Priority: MEDIUM)

**Estimated Time:** 2-3 days  
**Dependencies:** Visual validation complete  
**Status:** BLOCKED (awaiting visual validation)

### 4.1 Test Applications List

#### Tier 1: Simple Applications (Must Work)
- [ ] **weston-terminal** - Simple terminal emulator
- [ ] **foot** - Minimal Wayland terminal
- [ ] **weston-simple-shm** - Basic SHM client
- [ ] **weston-simple-egl** - Basic EGL client

#### Tier 2: Common Applications (Should Work)
- [ ] **alacritty** - GPU-accelerated terminal
- [ ] **kitty** - Feature-rich terminal
- [ ] **gedit** - Text editor
- [ ] **nautilus** - File manager
- [ ] **mpv** - Video player

#### Tier 3: Complex Applications (Nice to Have)
- [ ] **Firefox** - Web browser
- [ ] **Chromium** - Web browser
- [ ] **VSCode** - IDE
- [ ] **GIMP** - Image editor
- [ ] **LibreOffice** - Office suite

### 4.2 Testing Procedure

For each application:

1. **Launch Test**
   ```bash
   WAYLAND_DISPLAY=wayland-1 <application> 2>&1 | tee app_test.log
   ```

2. **Verify Basic Functionality**
   - [ ] Application window appears
   - [ ] Content renders correctly
   - [ ] Input (keyboard/mouse) works
   - [ ] Resizing works
   - [ ] Focus changes work
   - [ ] Window closes cleanly

3. **Document Issues**
   - Screenshot of issues
   - Log output analysis
   - Steps to reproduce
   - Expected vs. actual behavior

### 4.3 Compatibility Report

Create `PHASE_6_4_APPLICATION_COMPATIBILITY.md`:

```markdown
# Application Compatibility Report

## Test Environment
- Axiom Version: Phase 6.4
- Date: YYYY-MM-DD
- Hardware: GPU model, driver

## Results Summary
- Tier 1: 4/4 working (100%)
- Tier 2: 3/5 working (60%)
- Tier 3: 1/5 working (20%)

## Detailed Results

### weston-terminal
- Status: ✅ PASS
- Issues: None
- Notes: Works perfectly

### Firefox
- Status: ❌ FAIL
- Issues: Window decorations missing
- Workaround: Use client-side decorations
- Priority: HIGH
```

---

## Task 5: Performance Validation (Priority: MEDIUM)

**Estimated Time:** 2-3 days  
**Dependencies:** Damage-aware rendering complete  
**Status:** CAN START AFTER TASK 2

### 5.1 Performance Benchmarks

#### Benchmark Suite

File: `axiom/benches/render_performance.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_window_rendering(c: &mut Criterion) {
    c.bench_function("render 1 window", |b| {
        b.iter(|| {
            // Render single window
            black_box(render_single_window())
        })
    });
    
    c.bench_function("render 5 windows", |b| {
        b.iter(|| {
            // Render 5 windows
            black_box(render_multiple_windows(5))
        })
    });
    
    c.bench_function("render 10 windows", |b| {
        b.iter(|| {
            // Render 10 windows
            black_box(render_multiple_windows(10))
        })
    });
}

criterion_group!(benches, benchmark_window_rendering);
criterion_main!(benches);
```

#### Run Benchmarks

```bash
cargo bench --bench render_performance
```

### 5.2 Performance Targets

| Metric | Target | Measurement Method |
|--------|--------|-------------------|
| Frame Time | < 16ms | Average over 1000 frames |
| FPS | 60+ | Sustained with 5 windows |
| CPU Usage | < 10% | Single core, idle windows |
| GPU Usage | < 20% | With basic effects |
| Memory | < 150MB | Baseline with 10 windows |
| Latency | < 10ms | Input to screen update |

### 5.3 Profiling

#### CPU Profiling with perf

```bash
# Record performance data
perf record -g cargo run --release --bin run_present_winit

# Generate flamegraph
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg
```

#### GPU Profiling

```bash
# NVIDIA
nvidia-smi dmon -s um

# AMD
radeontop

# Intel
intel_gpu_top
```

### 5.4 Memory Leak Testing

```bash
# Run with valgrind
valgrind --leak-check=full \
         --show-leak-kinds=all \
         --track-origins=yes \
         cargo run --bin run_present_winit
```

---

## Task 6: Documentation & Polish (Priority: LOW)

**Estimated Time:** 1-2 days  
**Dependencies:** All other tasks complete  
**Status:** CAN START ANYTIME

### 6.1 Update Documentation

- [ ] Update PHASE_6_4_PROGRESS.md with results
- [ ] Create PHASE_6_4_SUCCESS_REPORT.md
- [ ] Update README.md with Phase 6.4 status
- [ ] Add screenshots to documentation
- [ ] Update API documentation

### 6.2 Code Cleanup

- [ ] Remove debug logging or make conditional
- [ ] Fix remaining clippy warnings
- [ ] Run rustfmt on all files
- [ ] Remove unused imports
- [ ] Update code comments

### 6.3 Final Testing

- [ ] Run full test suite: `cargo test`
- [ ] Run benchmarks: `cargo bench`
- [ ] Check for memory leaks
- [ ] Verify no regressions

---

## Timeline & Milestones

### Week 1: Foundation
- **Day 1-2:** Visual validation (blocked by display environment)
- **Day 2-3:** Implement damage-aware rendering
- **Day 3-4:** Wire Smithay handler integration
- **Day 4-5:** Begin real application testing

### Week 2: Validation
- **Day 6-7:** Performance testing and profiling
- **Day 8-9:** Fix issues found in testing
- **Day 10-11:** Application compatibility testing
- **Day 12-13:** Documentation and polish

### Milestones

- [ ] **M1:** Visual validation complete (Day 2)
- [ ] **M2:** Damage optimization working (Day 4)
- [ ] **M3:** Real apps rendering (Day 7)
- [ ] **M4:** Performance targets met (Day 10)
- [ ] **M5:** Phase 6.4 complete (Day 13)

---

## Risk Mitigation

### Risk 1: Visual Validation Delayed
**Probability:** High  
**Impact:** Medium  
**Mitigation:** Continue with Task 2 (damage optimization) and Task 3 (Smithay integration) which don't require display environment

### Risk 2: Performance Below Target
**Probability:** Low  
**Impact:** Medium  
**Mitigation:** Profiling and optimization already planned; fallback to simpler effects

### Risk 3: Application Compatibility Issues
**Probability:** Medium  
**Impact:** Medium  
**Mitigation:** Test with simple apps first; have protocol debugging tools ready

---

## Success Metrics

Phase 6.4 is **COMPLETE** when:

- [ ] Visual validation passed (8/8 criteria)
- [ ] Damage-aware rendering reduces GPU load by 50%+
- [ ] 60 FPS with 10+ windows
- [ ] 4/4 Tier 1 applications work
- [ ] 3/5 Tier 2 applications work
- [ ] No memory leaks in 24h test
- [ ] All tests passing (93+)
- [ ] Documentation updated

**Current:** 0/8 criteria met  
**Target:** 8/8 criteria met by end of Phase 6.4

---

## Next Phase Preview: 6.5 Effects Integration

After Phase 6.4, focus shifts to visual effects:
- Blur shader implementation
- Rounded corners with anti-aliasing
- Drop shadows with soft edges
- Animation system integration
- Spring physics for natural motion

**Estimated Duration:** 2-4 weeks  
**Priority:** Medium (polish feature)

---

**Document Version:** 1.0  
**Last Updated:** December 19, 2024  
**Owner:** Axiom Development Team