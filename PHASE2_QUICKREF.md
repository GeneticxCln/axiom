# Phase 2 Quick Reference

**Goal**: Make Axiom usable as a daily driver  
**Duration**: 3-4 weeks  
**Status**: Ready to start

---

## 📋 Task Checklist

### 2.1 Window Decorations (3-4 days)
- [ ] Design decoration data structures
- [ ] Render title bars as overlay quads
- [ ] Add window title text rendering
- [ ] Implement close/maximize/minimize buttons
- [ ] Handle button click detection
- [ ] Add border rendering
- [ ] Implement resize by dragging edges
- [ ] Test with multiple windows

### 2.2 Tiling Management (5-7 days)
- [ ] Implement horizontal tiling algorithm
- [ ] Implement master/stack layout
- [ ] Integrate with workspace manager
- [ ] Add gap management
- [ ] Implement interactive resize
- [ ] Add floating mode toggle
- [ ] Test with 3+ windows

### 2.3 Multi-Monitor (3-5 days)
- [ ] Per-monitor workspace sets
- [ ] Window positioning per monitor
- [ ] Move windows between monitors
- [ ] Monitor hotplug handling
- [ ] DPI scaling support
- [ ] wl_output advertisement
- [ ] Test with 2 monitors

### 2.4 Workspace Management (4-5 days)
- [ ] Implement scrolling animation
- [ ] Render with scroll offset
- [ ] Move windows between workspaces
- [ ] Window visibility management
- [ ] Easing function for smooth animation
- [ ] Test transitions

### 2.5 Keyboard Shortcuts (1-2 days)
- [ ] Define all compositor actions
- [ ] Wire up keybindings in config
- [ ] Implement action handlers
- [ ] Test all shortcuts
- [ ] Document in user guide

---

## 🎯 Success Metrics

**Phase 2 is complete when**:
- ✅ All windows have visible title bars
- ✅ Windows automatically tile
- ✅ Works on multiple monitors
- ✅ Smooth workspace scrolling
- ✅ All keyboard shortcuts work
- ✅ Can use for 8+ hours daily

---

## 🚀 Start Here

**Recommended first task**: Window Decorations  
**Why**: Visual impact, builds confidence, foundation for other features

**Command to start**:
```bash
cd /home/quinton/axiom
git checkout -b phase2-decorations
# Start implementing!
```

---

## 📚 Key Files

- `src/decoration/mod.rs` - Decoration manager (already exists!)
- `src/window/mod.rs` - Window management
- `src/workspace/mod.rs` - Workspace system
- `src/renderer/mod.rs` - GPU rendering
- `src/smithay/server.rs` - Compositor core
- `src/input/mod.rs` - Input handling
- `axiom.toml` - Configuration

---

## 💡 Quick Tips

1. **Test frequently** - Build and run after each feature
2. **Use logging** - Add debug!() statements liberally
3. **Reference existing code** - Axiom already has great patterns
4. **One feature at a time** - Don't try to do everything at once
5. **Visual feedback** - Make changes visible immediately

---

See `PHASE2_PLAN.md` for detailed implementation plans!
