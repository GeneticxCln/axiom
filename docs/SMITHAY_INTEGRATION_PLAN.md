# Smithay Backend Integration Plan

## Overview
This document outlines the integration of Smithay into the Axiom compositor to provide real Wayland protocol support and hardware acceleration.

## Current Status
- ✅ Complete modular architecture
- ✅ Scrollable workspace system
- ✅ Visual effects engine
- ✅ Window management system
- ✅ Input handling system
- ✅ Server-side decoration system
- ✅ AI optimization client
- ⏳ Smithay backend (placeholder implementation)
- ⏳ GPU rendering pipeline
- ⏳ Wayland protocol support

## Integration Phases

### Phase 1: Core Smithay Setup (Week 1)
1. **Replace placeholder backend with real Smithay**
   - [ ] Set up Smithay compositor state
   - [ ] Initialize display and event loop
   - [ ] Configure backend (winit for development, DRM for production)
   - [ ] Set up renderer (OpenGL/Vulkan)

2. **Basic Protocol Support**
   - [ ] wl_compositor
   - [ ] wl_shm (shared memory buffers)
   - [ ] wl_seat (input devices)
   - [ ] wl_output (display management)

### Phase 2: Window Management Integration (Week 2)
1. **Surface Management**
   - [ ] Map Smithay surfaces to our Window structs
   - [ ] Handle surface commits and damage
   - [ ] Implement surface positioning
   - [ ] Connect to our WindowManager

2. **XDG Shell Support**
   - [ ] xdg_wm_base
   - [ ] xdg_surface
   - [ ] xdg_toplevel
   - [ ] xdg_popup

3. **Decoration Protocol**
   - [ ] Connect xdg_decoration to our DecorationManager
   - [ ] Handle SSD/CSD negotiation
   - [ ] Render server-side decorations

### Phase 3: Input System Integration (Week 3)
1. **Input Device Management**
   - [ ] Keyboard input through Smithay
   - [ ] Pointer (mouse) input
   - [ ] Touch input support
   - [ ] Connect to our InputManager

2. **Focus Management**
   - [ ] Keyboard focus handling
   - [ ] Pointer focus and enter/leave events
   - [ ] Focus follows mouse option

### Phase 4: GPU Rendering Pipeline (Week 4)
1. **OpenGL Renderer Setup**
   - [ ] Initialize OpenGL context
   - [ ] Texture management for surfaces
   - [ ] Shader pipeline for effects

2. **Effects Implementation**
   - [ ] Blur shaders (dual-pass Gaussian)
   - [ ] Shadow rendering
   - [ ] Rounded corners with anti-aliasing
   - [ ] Opacity and scaling transforms

3. **Performance Optimization**
   - [ ] Damage tracking
   - [ ] Partial redraws
   - [ ] Frame scheduling
   - [ ] Adaptive sync support

### Phase 5: Advanced Features (Week 5-6)
1. **Additional Protocols**
   - [ ] wlr-layer-shell (panels, overlays)
   - [ ] wp-viewporter (scaling)
   - [ ] wp-presentation-time (frame timing)
   - [ ] zwp-linux-dmabuf (zero-copy buffers)

2. **XWayland Integration**
   - [ ] Connect XWayland to Smithay
   - [ ] X11 window management
   - [ ] Clipboard synchronization

3. **Multi-GPU Support**
   - [ ] GPU selection
   - [ ] Prime offloading
   - [ ] Buffer sharing

## Implementation Strategy

### 1. Start with Winit Backend
- Use winit backend for development (window mode)
- Easier debugging and testing
- No need for TTY switching

### 2. Incremental Integration
- Keep existing architecture intact
- Replace components one by one
- Maintain working demos throughout

### 3. Testing Approach
- Unit tests for each component
- Integration tests with real Wayland clients
- Performance benchmarks
- Visual regression tests

## Key Files to Modify

1. `src/smithay_backend.rs` - Main integration point
2. `src/compositor.rs` - Connect Smithay event loop
3. `src/window.rs` - Map surfaces to windows
4. `src/input.rs` - Route Smithay input events
5. `src/effects.rs` - GPU rendering integration
6. `src/decoration.rs` - Protocol integration

## Dependencies to Add

```toml
[dependencies]
smithay = { version = "0.3", features = ["backend_winit", "backend_drm", "backend_libinput", "renderer_gl", "xwayland", "wayland_frontend"] }
smithay-client-toolkit = "0.18"
wayland-server = "0.31"
wayland-protocols = { version = "0.31", features = ["unstable", "staging"] }
gl = "0.14"
```

## Success Criteria

1. **Basic Functionality**
   - Can run simple Wayland clients (weston-terminal, etc.)
   - Keyboard and mouse input working
   - Windows properly positioned and rendered

2. **Effects Working**
   - All visual effects functioning with GPU acceleration
   - Performance targets met (60+ FPS)
   - Adaptive quality working

3. **Protocol Compliance**
   - Pass Wayland protocol conformance tests
   - Support common applications
   - Stable operation

4. **Performance**
   - Sub-16ms frame times under normal load
   - Efficient memory usage
   - Low CPU usage when idle

## Timeline

- **Week 1-2**: Core Smithay setup and basic protocols
- **Week 3-4**: Window and input integration
- **Week 5-6**: GPU rendering pipeline
- **Week 7-8**: Advanced features and XWayland
- **Week 9-10**: Testing, optimization, and polish
- **Week 11-12**: Production readiness and documentation

Total: 3 months to production-ready state
