# Axiom Compositor - Production Readiness Plan

**Created**: 2025-10-09  
**Current Phase**: Post-Crash Fix, Pre-Production  
**Status**: Foundation Stable, Client Integration Needed

---

## 📊 Current Project Analysis

### Project Stats
- **Lines of Code**: ~39,000 lines of Rust
- **Files**: 69 Rust source files
- **Architecture**: Modular, well-organized
- **Dependencies**: Modern stack (Smithay, wgpu, tokio)
- **Repository**: Clean and professional

### ✅ What's Working (Priority 1 - COMPLETED!)
1. **✅ Compositor Runs Stably** - No crashes, efficient rendering
2. **✅ Window System** - Opens and resizes without issues  
3. **✅ GPU Rendering** - WGPU pipeline working on NVIDIA RTX 3050
4. **✅ Wayland Server** - Running on `wayland-2`
5. **✅ XWayland** - Started on `DISPLAY=:2`
6. **✅ Performance** - Efficient resource usage, no tight loops
7. **✅ Repository** - Organized, documented, version controlled

### ❌ What's Missing (Critical Gaps)

#### **Gap 1: No Wayland Client Support** 🔴 CRITICAL
**Problem**: The compositor starts but clients can't connect and display windows.

**Evidence**:
```
renderer now has 0 windows
```

**Impact**: This is a **show-stopper**. Without this:
- Can't run any Wayland apps (weston-terminal, firefox, etc.)
- Can't test window management
- Can't use it as an actual compositor
- It's just an empty window

**What's Needed**:
1. Client buffer handling (wl_shm, dmabuf)
2. Surface lifecycle management
3. Content rendering from clients
4. Window decorations
5. Input event forwarding to clients

---

## 🎯 Production Roadiness

 Roadmap

### Priority Levels
- **P0** - Blocking (can't use without this)
- **P1** - Critical (needed for basic usability)
- **P2** - Important (needed for production)
- **P3** - Nice to have (polish)

---

## 🚀 Phase 1: Make It Usable (P0 - BLOCKING)

**Goal**: Users can run basic Wayland applications

### 1.1 Client Buffer Integration [P0]
**Time**: 3-5 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] Implement `wl_shm` buffer handling
  - [ ] Create shared memory pool management
  - [ ] Handle buffer attach/commit protocol
  - [ ] Copy buffer data to GPU textures
- [ ] Test with `weston-simple-shm`
- [ ] Verify window content displays

**Success Criteria**:
- `weston-simple-shm` shows its colorful square
- Buffer updates render correctly

### 1.2 Surface Lifecycle [P0]
**Time**: 2-3 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] Handle surface creation from clients
- [ ] Surface commit/attach protocol
- [ ] Surface destruction
- [ ] Map surfaces to windows in renderer
- [ ] Add window to Z-order stack

**Success Criteria**:
- Multiple windows can open simultaneously
- Windows appear in correct stacking order

### 1.3 Input Routing [P0]
**Time**: 2-3 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] Route keyboard events to focused window
- [ ] Route mouse events to window under cursor
- [ ] Implement focus management
- [ ] Handle keyboard/pointer enter/leave events

**Success Criteria**:
- Can type in terminal windows
- Can click buttons in GUI apps
- Focus follows correctly

### 1.4 XDG Shell Implementation [P0]
**Time**: 3-4 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] `xdg_toplevel` configure/ack cycle
- [ ] Window resize protocol
- [ ] Window move protocol
- [ ] Minimize/maximize/fullscreen states
- [ ] Window close requests

**Success Criteria**:
- Can run `weston-terminal`
- Can resize windows with mouse
- Can close windows properly

**Phase 1 Total Time**: ~2-3 weeks

---

## 🎨 Phase 2: Make It Good (P1 - CRITICAL)

**Goal**: Usable as daily driver compositor

### 2.1 Window Decorations [P1]
**Time**: 3-4 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] Server-side decorations (SSD)
- [ ] Title bars with window titles
- [ ] Close/minimize/maximize buttons
- [ ] Window borders
- [ ] Resize handles

**Success Criteria**:
- Windows have professional-looking decorations
- Buttons work correctly

### 2.2 Workspace Management [P1]
**Time**: 4-5 days  
**Status**: 🟡 PARTIAL (infrastructure exists)

**Tasks**:
- [ ] Actually move windows between workspaces
- [ ] Scroll animation between workspaces
- [ ] Render multiple workspaces correctly
- [ ] Handle window visibility per workspace
- [ ] Keyboard shortcuts for workspace navigation

**Success Criteria**:
- Can move windows left/right with Super+Shift+Arrow
- Smooth scrolling animation
- Windows only visible on correct workspace

### 2.3 Tiling Window Management [P1]
**Time**: 5-7 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] Automatic window tiling algorithm
- [ ] Window resize with neighbors
- [ ] Gap management
- [ ] Floating window mode
- [ ] Master/stack layout

**Success Criteria**:
- Windows tile automatically
- Can resize tiles with mouse
- Professional tiling behavior

### 2.4 Multi-Monitor Support [P1]
**Time**: 3-5 days  
**Status**: 🟡 PARTIAL (detection working)

**Tasks**:
- [ ] Proper output management
- [ ] Per-monitor workspace sets
- [ ] Window movement between monitors
- [ ] DPI scaling per monitor
- [ ] Monitor hotplug

**Success Criteria**:
- Works correctly with 2+ monitors
- Can move windows between monitors
- Handles monitor plug/unplug

**Phase 2 Total Time**: ~3-4 weeks

---

## 🚀 Phase 3: Make It Beautiful (P2 - IMPORTANT)

**Goal**: Polished, production-quality compositor

### 3.1 Visual Effects [P2]
**Time**: 7-10 days  
**Status**: 🟡 PARTIAL (infrastructure exists)

**Tasks**:
- [ ] Window open/close animations
- [ ] Window move animations
- [ ] Workspace scroll animations
- [ ] Blur effects (backgrounds, windows)
- [ ] Shadow rendering
- [ ] Rounded corners

**Success Criteria**:
- Smooth 60fps animations
- Beautiful blur effects
- Professional visual polish

### 3.2 Performance Optimization [P2]
**Time**: 5-7 days  
**Status**: 🟢 GOOD (damage tracking infrastructure ready)

**Tasks**:
- [ ] Damage tracking optimization
- [ ] Only redraw changed regions
- [ ] GPU memory optimization
- [ ] Frame rate limiting
- [ ] CPU usage optimization

**Success Criteria**:
- <5% CPU idle
- <100MB GPU memory for empty desktop
- Maintains 60fps with effects

### 3.3 Configuration System [P2]
**Time**: 3-4 days  
**Status**: 🟢 GOOD (TOML config exists)

**Tasks**:
- [ ] Runtime config reload
- [ ] Per-app rules
- [ ] Keybinding customization
- [ ] Theme system
- [ ] Config validation

**Success Criteria**:
- Can reload config without restart
- Full customization available

### 3.4 Session Management [P2]
**Time**: 4-5 days  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] systemd/logind integration
- [ ] VT switching
- [ ] Session locking
- [ ] Power management integration
- [ ] Idle detection

**Success Criteria**:
- Proper session integration
- Can lock screen
- VT switching works

**Phase 3 Total Time**: ~4-5 weeks

---

## 🎯 Phase 4: Make It Production (P2-P3)

**Goal**: Enterprise-ready, stable, well-tested

### 4.1 Stability & Testing [P2]
**Time**: 5-7 days  
**Status**: 🟡 PARTIAL (some tests exist)

**Tasks**:
- [ ] Comprehensive integration tests
- [ ] Crash recovery
- [ ] Memory leak detection
- [ ] Fuzzing for protocol handlers
- [ ] CI/CD pipeline

### 4.2 Documentation [P2]
**Time**: 3-4 days  
**Status**: 🟢 GOOD (docs organized)

**Tasks**:
- [ ] User manual
- [ ] Configuration guide
- [ ] Troubleshooting guide
- [ ] API documentation
- [ ] Migration guide

### 4.3 Packaging [P3]
**Time**: 3-5 days  
**Status**: 🟡 PARTIAL (structure exists)

**Tasks**:
- [ ] Arch Linux package (AUR)
- [ ] NixOS package
- [ ] Debian package
- [ ] Install script
- [ ] Desktop entry files

### 4.4 Community [P3]
**Time**: Ongoing  
**Status**: 🔴 NOT STARTED

**Tasks**:
- [ ] Issue templates
- [ ] Contributing guide
- [ ] Code of conduct
- [ ] Discord/Matrix channel
- [ ] Website/landing page

**Phase 4 Total Time**: ~3-4 weeks

---

## 📅 Timeline Summary

| Phase | Duration | Target Completion |
|-------|----------|-------------------|
| **Phase 1** - Make It Usable | 2-3 weeks | Week 3 |
| **Phase 2** - Make It Good | 3-4 weeks | Week 7 |
| **Phase 3** - Make It Beautiful | 4-5 weeks | Week 12 |
| **Phase 4** - Make It Production | 3-4 weeks | Week 16 |

**Total Time to Production**: **12-16 weeks** (~3-4 months)

---

## 🎯 Immediate Next Steps (This Week)

### Day 1-2: Buffer Integration
1. Study Smithay `wl_shm` buffer handling
2. Implement buffer copying to GPU textures
3. Test with `weston-simple-shm`

### Day 3-4: Surface Lifecycle
1. Hook surface creation into window manager
2. Map surfaces to renderer windows
3. Test with multiple windows

### Day 5-7: Input Routing
1. Implement focus management
2. Route keyboard/mouse to clients
3. Test with `weston-terminal`

---

## 🔥 Critical Decisions

### Architecture Decisions Needed:

1. **Compositor Protocol Choice**
   - Current: Smithay framework
   - Decision: ✅ Keep Smithay (good choice)

2. **Rendering Backend**
   - Current: wgpu
   - Decision: ✅ Keep wgpu (modern, flexible)

3. **Buffer Strategy**
   - Options: Copy to GPU texture vs zero-copy
   - Recommendation: Start with copy, optimize later

4. **Window Stack**
   - Current: WindowStack infrastructure ready
   - Decision: ✅ Use existing infrastructure

---

## 📊 Success Metrics

### Minimum Viable Compositor (Phase 1 Complete)
- ✅ Starts without crashing
- 🔴 Can run 3+ Wayland apps simultaneously
- 🔴 Can type and click in windows
- 🔴 Windows display actual content

### Daily Driver (Phase 2 Complete)
- 🔴 Can use as primary compositor for 8+ hours
- 🔴 Smooth window management
- 🔴 Multi-monitor support
- 🔴 Stable under load

### Production Ready (Phase 3-4 Complete)
- 🔴 Beautiful visual effects at 60fps
- 🔴 <5% CPU idle usage
- 🔴 Comprehensive documentation
- 🔴 Active user community

---

## 🚧 Known Technical Debt

1. **Error Handling**: Some unwrap()s should be proper error handling
2. **Memory Management**: Texture pool could be optimized
3. **Protocol Coverage**: Many Wayland protocols not implemented
4. **Testing**: Limited integration test coverage
5. **Performance**: Damage tracking not fully utilized

---

## 💡 Recommendations

### For Immediate Progress:
1. **Focus on Phase 1** - Get clients working FIRST
2. **Test continuously** - Use weston-simple-shm, weston-terminal
3. **One feature at a time** - Don't try to do everything
4. **Learn from examples** - Study Smithay anvil example

### For Long-term Success:
1. **Build community early** - Document as you go
2. **Release early** - Alpha releases for feedback
3. **Prioritize stability** - Don't add features until core is solid
4. **Performance matters** - Profile regularly

---

## 🎓 Learning Resources

### Essential Reading:
- Smithay documentation
- Wayland protocol specification
- wgpu guide
- Anvil compositor source code

### Testing Tools:
- `weston-simple-shm` - Basic buffer test
- `weston-terminal` - Full app test
- `weston-info` - Protocol introspection
- `wayland-scanner` - Protocol debugging

---

## ✅ Next Session Checklist

Ready to start Phase 1:
- [ ] Review Smithay buffer handling examples
- [ ] Set up buffer infrastructure in renderer
- [ ] Implement wl_shm protocol handlers
- [ ] Create buffer-to-texture pipeline
- [ ] Test with weston-simple-shm

**Let's make Axiom usable! 🚀**
