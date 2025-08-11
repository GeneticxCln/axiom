# Axiom - Hybrid Wayland Compositor Project Plan

**Vision:** The first Wayland compositor combining niri's scrollable workspace innovation with Hyprland's visual effects system.

## 🎯 Project Overview

### Core Philosophy
- **Productivity**: Infinite scrollable workspaces for seamless workflow
- **Aesthetics**: Beautiful animations, blur, shadows, rounded corners
- **Performance**: Optimized for both tiling efficiency and visual effects
- **Compatibility**: Full XWayland support for real-world usage

### Target Audience
- Power users who want both innovative tiling AND eye candy
- Developers transitioning from traditional WMs who want modern visuals
- Users frustrated by having to choose between productivity vs aesthetics

## 🏗️ Technical Architecture

### Foundation Choice: **wlroots + Rust**
```rust
// Core structure
struct AxiomCompositor {
    // wlroots backend (proven, stable)
    wlr_backend: WlrBackend,
    wlr_renderer: WlrRenderer,
    
    // Axiom's hybrid systems
    workspace_manager: ScrollableWorkspaces,  // niri-inspired
    effects_engine: EffectsRenderer,         // hyprland-inspired
    window_manager: HybridWindowManager,     // best of both
    
    // Essential components
    xwayland: XWaylandManager,
    input_manager: InputHandler,
    config: AxiomConfig,
}
```

## 📋 Development Phases

### Phase 1: Foundation (Months 1-3)
**Goal: Basic functional compositor**

**Milestones:**
- [ ] Set up wlroots-rs bindings and basic compositor structure
- [ ] Basic window management (open/close/focus)
- [ ] Simple keyboard/mouse input handling
- [ ] Basic XWayland support
- [ ] Minimal configuration system

**Deliverable:** Can launch applications and manage windows (no tiling yet)

### Phase 2: Scrollable Workspaces (Months 4-6)
**Goal: Implement niri's core innovation**

**Milestones:**
- [ ] Infinite horizontal workspace scrolling
- [ ] Dynamic window placement algorithm
- [ ] Keyboard navigation (Super+Left/Right to scroll)
- [ ] Window following (move windows between workspace positions)
- [ ] Basic workspace indicators

**Deliverable:** Working scrollable tiling system

### Phase 3: Visual Effects System (Months 7-9)
**Goal: Add Hyprland-style visual polish**

**Milestones:**
- [ ] Animation framework
- [ ] Window open/close animations
- [ ] Workspace transition animations
- [ ] Blur effects (background, window borders)
- [ ] Rounded corners and shadows
- [ ] Configurable animation curves

**Deliverable:** Beautiful animated scrollable compositor

### Phase 4: Advanced Features (Months 10-12)
**Goal: Production-ready compositor**

**Milestones:**
- [ ] Multi-monitor support with independent scrollable workspaces
- [ ] Advanced configuration system (TOML-based)
- [ ] Plugin/extension system
- [ ] IPC interface for external tools
- [ ] Gesture support (touchpad scrolling)
- [ ] Workspace thumbnails/overview mode

**Deliverable:** Feature-complete Axiom v1.0

## 🛠️ Technical Implementation Plan

### Core Components

#### 1. Workspace Manager
```rust
struct ScrollableWorkspaces {
    current_position: f64,        // Smooth scrolling position
    windows: Vec<WindowLayout>,   // Window arrangement
    scroll_velocity: f64,         // Animation momentum
    workspace_width: u32,         // Virtual workspace size
}

impl ScrollableWorkspaces {
    fn scroll_to(&mut self, position: f64) { /* smooth animation */ }
    fn place_window(&mut self, window: Window) { /* smart placement */ }
    fn remove_window(&mut self, window: Window) { /* reflow layout */ }
}
```

#### 2. Effects Engine
```rust
struct EffectsRenderer {
    blur_shader: BlurShader,
    animation_system: AnimationSystem,
    shadow_renderer: ShadowRenderer,
}

impl EffectsRenderer {
    fn render_workspace_transition(&self, from: f64, to: f64, progress: f64);
    fn render_window_animation(&self, window: &Window, animation_type: AnimationType);
    fn apply_blur_effect(&self, surface: &Surface, radius: f32);
}
```

#### 3. Configuration System
```toml
# ~/.config/axiom/axiom.toml
[workspace]
scroll_speed = 1.0
auto_scroll = true
infinite_scroll = true

[animations]
enabled = true
duration = 300  # milliseconds
curve = "ease-out"

[effects]
blur_radius = 10
rounded_corners = 8
shadow_size = 20

[bindings]
scroll_left = "Super_L+Left"
scroll_right = "Super_L+Right"
move_window_left = "Super_L+Shift+Left"
```

## 🔧 Development Setup

### Prerequisites
```bash
# Required dependencies
sudo pacman -S rustup wlroots-git libxkbcommon wayland wayland-protocols
rustup default stable

# Development tools  
sudo pacman -S git cmake pkgconf meson ninja
```

### Repository Structure
```
axiom/
├── src/
│   ├── main.rs                 # Entry point
│   ├── compositor.rs           # Core compositor logic
│   ├── workspace/              # Scrollable workspace system
│   ├── effects/                # Visual effects engine
│   ├── window/                 # Window management
│   ├── input/                  # Input handling
│   ├── config/                 # Configuration system
│   └── xwayland/              # XWayland integration
├── shaders/                    # GLSL shaders for effects
├── config/                     # Default configuration files
├── docs/                       # Documentation
├── examples/                   # Example configurations
└── tests/                      # Test suite
```

## 🎨 Key Features Specification

### Scrollable Workspaces (from niri)
- **Infinite horizontal scrolling** with smooth animations
- **Smart window placement** that adapts to workflow
- **Dynamic workspace sizing** based on window content
- **Keyboard navigation** optimized for productivity

### Visual Effects (from Hyprland)
- **Smooth animations** for all window operations
- **Blur effects** with configurable intensity
- **Rounded corners** with anti-aliasing
- **Drop shadows** with realistic lighting
- **Workspace transition effects**

### Hybrid Innovations
- **Animated workspace scrolling** (unique to Axiom)
- **Context-aware effects** (reduced effects during scrolling for performance)
- **Smart performance scaling** (automatically adjust effects based on system load)

## 🚀 Milestones & Timeline

| Phase | Duration | Key Deliverable | Status |
|-------|----------|-----------------|---------|
| Foundation | M1-M3 | Basic compositor | 🟡 Planning |
| Scrollable WS | M4-M6 | Working tiling | 🔴 Not Started |
| Visual Effects | M7-M9 | Animated compositor | 🔴 Not Started |
| Advanced Features | M10-M12 | Production ready | 🔴 Not Started |

## 🎯 Success Metrics

### Technical Goals
- [ ] Stable 60fps scrolling with 10+ windows
- [ ] <100ms window operation latency
- [ ] <200MB memory footprint
- [ ] 100% XWayland compatibility

### Community Goals
- [ ] 1000+ GitHub stars in first year
- [ ] Active contributor community (5+ regular contributors)
- [ ] Package availability in major distros
- [ ] Positive reception in r/unixporn community

## 🤝 Community & Contribution

### Open Source Strategy
- **License:** GPLv3 (same as Hyprland)
- **Repository:** GitHub with comprehensive documentation
- **Communication:** Discord server + GitHub discussions
- **Contribution:** Welcoming to new contributors with good first issues

### Marketing Plan
- [ ] Development blog documenting progress
- [ ] Demo videos showcasing unique features
- [ ] Presentations at Linux conferences
- [ ] Collaboration with existing WM communities

## 📚 Learning Resources

### Required Knowledge Areas
1. **Rust programming** - Advanced level needed
2. **Wayland protocol** - Understanding of compositor architecture
3. **Graphics programming** - OpenGL/Vulkan for effects
4. **wlroots library** - Core compositor functionality
5. **XWayland** - X11 compatibility layer

### Study Materials
- wlroots documentation and examples
- niri source code analysis
- Hyprland architecture study
- Wayland protocol specifications

---

**Next Steps:**
1. Set up development environment
2. Create GitHub repository
3. Implement basic compositor skeleton
4. Begin Phase 1 development

*Axiom: Where productivity meets beauty* 🚀
