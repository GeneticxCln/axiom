# Axiom Compositor Production Implementation Plan

## Executive Summary

This plan transforms Axiom from a functional Wayland compositor to a production-ready system suitable for daily use. The plan is structured in 4 phases over ~90 days, prioritizing core functionality, stability, and user experience.

## Current State Analysis

### ✅ Strengths
- **Solid Foundation**: Smithay-based server loop with calloop integration
- **Modern Rendering**: WGPU-based effects pipeline (shadows, blur)
- **Quality Assurance**: Visual golden testing framework established
- **Clean Architecture**: Structured logging, resource lifecycle tests
- **Innovation**: Scrollable workspaces (niri-inspired)

### ⚠️ Production Gaps
- **Protocol Coverage**: Missing clipboard, screencopy, fractional scaling
- **Input Handling**: Incomplete gesture support, missing IME
- **Multi-Output**: Basic hotplug, needs scale/transform correctness
- **Performance**: Full-frame redraws, unoptimized blur
- **Security**: Root dependencies, no sandboxing
- **CI/CD**: Manual testing, no automated visual validation

## Phase 1: Core Protocols & Stability (Weeks 1-3)

### Priority 1.1: Essential Protocols
**Target**: Apps like Firefox, VSCode, terminals work reliably

#### xdg-shell Robustness
- [ ] **Configure/Ack Cycle Validation**
  - Implement proper configure → ack → commit state machine
  - Add timeout handling for unresponsive clients
  - Test with rapid resize scenarios
  
- [ ] **Surface Lifecycle Edge Cases**
  - Handle unmap during resize/move operations
  - Proper cleanup on client disconnect
  - Subsurface z-order correctness

```rust
// Example implementation target
pub struct SurfaceState {
    configure_serial: Option<u32>,
    pending_configure: Option<Configure>,
    ack_deadline: Option<Instant>,
    lifecycle_state: SurfaceLifecycle,
}

enum SurfaceLifecycle {
    Created,
    Configured { serial: u32 },
    Mapped,
    Unmapped,
    Destroyed,
}
```

#### Data Device (Clipboard)
- [ ] **Basic Copy/Paste**
  - Implement `wl_data_device_manager`
  - MIME type negotiation (text/plain, text/html)
  - Selection ownership tracking
  
- [ ] **Primary Selection**
  - Add `wp_primary_selection_device_manager_v1`
  - Middle-click paste support

#### Layer Shell Completion
- [ ] **Z-Order Management**
  - Validate overlay > top > bottom > background ordering
  - Handle exclusive zone calculations
  - Test with multiple panels/bars

### Priority 1.2: Multi-Output Correctness
**Target**: Reliable dual-monitor setups with different scales

- [ ] **Hotplug Handling**
  - DRM connector state monitoring
  - Graceful surface migration between outputs
  - Persistent workspace assignments

- [ ] **Per-Output Scaling**
  - Fractional scale support (1.25x, 1.5x, 2.0x)
  - Surface scale negotiation via `wp_fractional_scale_v1`
  - HiDPI cursor scaling

### Priority 1.3: Input System Hardening

- [ ] **Focus Model Consistency**
  - Keyboard focus follows pointer click
  - Decoration button focus behavior
  - Focus memory across workspace switches

- [ ] **Gesture Support**
  - Touch tap/swipe recognition
  - Pinch-to-zoom passthrough
  - Gesture cancellation on timeout

### Deliverables Phase 1
- [ ] Protocol test suite covering all implemented protocols
- [ ] Multi-output test rig with different scales
- [ ] Input focus edge case tests
- [ ] Documentation: "Supported Applications" compatibility matrix

## Phase 2: Performance & Rendering Pipeline (Weeks 4-6)

### Priority 2.1: Damage-Aware Rendering
**Target**: 120fps on simple scenes, 60fps with heavy blur

- [ ] **Surface Damage Tracking**
```rust
pub struct DamageTracker {
    accumulated_damage: Vec<Rectangle>,
    output_damage: HashMap<u64, Vec<Rectangle>>, // per output
    last_frame_damage: Vec<Rectangle>,
}

impl DamageTracker {
    fn accumulate_surface_damage(&mut self, surface_id: u64, damage: Rectangle);
    fn get_output_damage(&self, output_id: u64) -> &[Rectangle];
    fn optimize_damage_rects(&self, rects: &[Rectangle]) -> Vec<Rectangle>;
}
```

- [ ] **Scissor-Based Rendering**
  - Skip unchanged regions
  - Merge overlapping damage rectangles
  - Per-output damage accumulation

### Priority 2.2: Blur Pipeline Optimization

- [ ] **Complete Visual Test Integration**
  - Fix CommandEncoder handling in VisualTestContext
  - Implement proper TextureView usage
  - Generate all golden baseline images

- [ ] **Performance Optimizations**
  - Dual Kawase blur for large radii (> 20px)
  - Downsample chain for blur radius > 50px
  - Cache pipeline/sampler objects per configuration

- [ ] **Quality Controls**
  - Adaptive quality based on frame time budget
  - Blur LOD based on distance/occlusion
  - Optional compute shader path for supported hardware

### Priority 2.3: Frame Pacing

- [ ] **VSync and Present Mode Control**
  - Mailbox mode for low latency
  - FIFO for battery life
  - Immediate mode for benchmarking

- [ ] **Frame Time Budgeting**
```rust
pub struct FramePacer {
    target_framerate: f64, // 60, 120, 144, etc.
    frame_budget: Duration,
    render_budget: Duration, // 80% of frame budget
    last_frame_times: VecDeque<Duration>,
}

impl FramePacer {
    fn should_skip_frame(&self) -> bool;
    fn get_quality_scale(&self) -> f32; // 0.5-1.0 for adaptive quality
    fn update_frame_stats(&mut self, render_time: Duration);
}
```

### Deliverables Phase 2
- [ ] Performance benchmark suite (automated FPS measurement)
- [ ] Visual regression CI with GPU runners
- [ ] Blur optimization comparison (before/after metrics)
- [ ] Frame pacing configuration options

## Phase 3: Advanced Features & UX (Weeks 7-9)

### Priority 3.1: Advanced Input

- [ ] **Text Input System**
  - `zwp_text_input_manager_v3` implementation
  - IME support framework (IBus integration)
  - Virtual keyboard protocol support

- [ ] **Pointer Constraints**
  - `zwp_pointer_constraints_v1` for gaming
  - `zwp_relative_pointer_manager_v1`
  - Cursor lock/hide modes

### Priority 3.2: Screen Sharing & Capture

- [ ] **Screencopy Protocol**
  - `wlr_screencopy_manager_v1` implementation
  - Per-output capture
  - Application window capture

- [ ] **DMA-BUF Support**
  - `zwp_linux_dmabuf_v1` for zero-copy
  - GPU texture sharing
  - Video decode acceleration path

### Priority 3.3: Window Management Polish

- [ ] **Advanced Tiling**
  - Master-stack layout implementation
  - Spiral/grid layout modes
  - Per-workspace layout persistence

- [ ] **Animation System**
  - Easing curves implementation
  - Workspace transition animations
  - Window open/close effects

```rust
pub struct AnimationController {
    active_animations: HashMap<u64, Animation>,
    easing_curves: HashMap<EasingType, Box<dyn Fn(f32) -> f32>>,
}

pub enum Animation {
    WindowScale { target: f32, duration: Duration },
    WorkspaceScroll { target: f64, curve: EasingType },
    Opacity { target: f32, duration: Duration },
}
```

### Deliverables Phase 3
- [ ] Advanced window management demo videos
- [ ] Screen sharing integration test with OBS
- [ ] IME support validation with international input
- [ ] Animation configuration UI/API

## Phase 4: Production Hardening (Weeks 10-12)

### Priority 4.1: Security & Stability

- [ ] **Permission Model**
  - Remove root requirements
  - udev rules for device access
  - Optional user group configuration

- [ ] **Crash Resilience**
```rust
pub struct CrashHandler {
    panic_hook: Box<dyn Fn(&std::panic::PanicInfo) + Send + Sync>,
    backtrace_buffer: Arc<Mutex<Vec<u8>>>,
    crash_dump_path: PathBuf,
}

impl CrashHandler {
    fn install_panic_hook(&self);
    fn generate_crash_report(&self, info: &std::panic::PanicInfo) -> CrashReport;
    fn attempt_graceful_shutdown(&self);
}
```

- [ ] **Resource Limits**
  - Memory usage monitoring
  - Surface/texture count limits
  - Client connection throttling

### Priority 4.2: Observability & Debugging

- [ ] **Structured Tracing**
```rust
use tracing::{instrument, info_span, Instrument};

#[instrument(skip(self, encoder))]
pub async fn render_frame(&mut self, encoder: &mut CommandEncoder) -> Result<()> {
    let _span = info_span!("render_frame", frame_id = self.frame_counter).entered();
    
    let compositor_span = info_span!("composite_surfaces");
    self.composite_surfaces().instrument(compositor_span).await?;
    
    let effects_span = info_span!("apply_effects");
    self.apply_effects(encoder).instrument(effects_span).await?;
    
    Ok(())
}
```

- [ ] **Debug HUD**
  - FPS overlay
  - Memory usage graphs
  - Active surface counts
  - Render time breakdown

- [ ] **Performance Profiling**
  - Tracy integration (optional)
  - Frame time histograms
  - GPU timing queries

### Priority 4.3: Configuration & Runtime Control

- [ ] **Typed Configuration**
```rust
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct AxiomConfig {
    pub input: InputConfig,
    pub effects: EffectsConfig,
    pub workspaces: WorkspaceConfig,
    pub outputs: Vec<OutputConfig>,
    pub keybindings: Vec<KeyBinding>,
}
```

- [ ] **Hot Reload**
  - Configuration file watching
  - Shader hot reload for development
  - Runtime effect toggling

- [ ] **IPC Control Interface**
```rust
pub enum ControlCommand {
    ReloadConfig,
    ToggleEffect(EffectType),
    SetLogLevel(Level),
    DumpState,
    Shutdown,
}

pub struct ControlServer {
    socket_path: PathBuf,
    command_rx: Receiver<ControlCommand>,
}
```

### Deliverables Phase 4
- [ ] Security audit checklist
- [ ] Performance profiling setup guide
- [ ] Configuration schema documentation
- [ ] Production deployment guide

## Continuous Integration Strategy

### Automated Testing Pipeline
```yaml
# .github/workflows/ci.yml structure
jobs:
  test:
    - Unit tests (cargo test)
    - Integration tests (resource lifecycle)
    - Protocol conformance tests
  
  visual:
    - Visual golden tests with GPU
    - Upload diff artifacts on failure
    - Auto-approve minor differences
  
  performance:
    - Benchmark suite execution
    - Regression detection (>5% slowdown fails)
    - Performance report generation

  security:
    - cargo audit
    - cargo deny (licenses)
    - Static analysis (clippy --deny warnings)
```

### Quality Gates
- **All tests pass**: Unit, integration, visual (with tolerance)
- **Performance budgets**: Frame time < 8.3ms for 120Hz simple scenes
- **Security checks**: No critical advisories, license compliance
- **Code quality**: rustfmt, clippy clean, documentation coverage > 80%

## Success Metrics

### Technical KPIs
- **Compatibility**: 95% of common Linux apps work correctly
- **Performance**: 60fps sustained with 2 4K monitors + effects
- **Stability**: MTBF > 24 hours during normal usage
- **Resource Usage**: < 100MB RAM baseline, < 200MB with heavy effects

### User Experience KPIs
- **Startup Time**: < 2 seconds from launch to usable desktop
- **Input Latency**: < 10ms click-to-response for UI interactions  
- **Visual Quality**: No artifacts in golden test suite
- **Configuration**: Hot-reload config changes without restart

## Risk Mitigation

### Technical Risks
- **GPU Compatibility**: Maintain WGPU backend fallbacks
- **Wayland Ecosystem Changes**: Track upstream Smithay closely
- **Performance Regressions**: Automated benchmark CI with alerts

### Project Risks
- **Scope Creep**: Stick to roadmap phases, defer non-essential features
- **Testing Coverage**: Prioritize automated tests over manual validation
- **Documentation Debt**: Write docs concurrent with implementation

## Getting Started

### Immediate Actions (Next 7 Days)
1. **Set up development environment**:
   ```bash
   # Install additional dependencies
   sudo pacman -S tracy wayland-protocols
   
   # Set up development tools
   cargo install cargo-deny cargo-audit
   ```

2. **Create branch structure**:
   ```bash
   git checkout -b phase-1/core-protocols
   git checkout -b phase-2/performance  
   git checkout -b phase-3/advanced-features
   git checkout -b phase-4/production-hardening
   ```

3. **Implement first protocol test**:
   - Start with xdg-shell configure/ack validation
   - Create baseline test that captures current behavior
   - Identify and fix edge cases

## Recommended Starting Point

I recommend beginning with **Phase 1.1: xdg-shell robustness** because:

1. **Immediate Impact**: Fixes compatibility issues with existing apps
2. **Foundation**: Other protocols build on xdg-shell correctness  
3. **Testable**: Clear pass/fail criteria for surface lifecycle
4. **User Visible**: Users will notice fewer app crashes/hangs

Would you like me to:

1. **Create the xdg-shell test framework** with specific surface lifecycle tests
2. **Implement damage tracking** for immediate performance gains
3. **Complete the visual test integration** building on your existing work
4. **Set up the CI pipeline** to automate testing

Choose your preferred starting point and I'll provide the detailed implementation with code examples!