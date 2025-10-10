# Phase 2: Make It Good - Daily Driver Compositor

**Status**: Ready to Start 🚀  
**Timeline**: 3-4 weeks  
**Goal**: Transform Axiom into a usable daily driver compositor

---

## 🎯 Phase 2 Overview

Now that we have working client support (Phase 1), we need to make Axiom actually **usable** for daily work. This means:

1. **Visual polish** - Windows need title bars and borders
2. **Smart tiling** - Automatic window layout that works well
3. **Multi-monitor** - Support for multiple displays
4. **Workspace flow** - Smooth navigation between workspaces
5. **Keyboard shortcuts** - Efficient window management

---

## 📋 Task Breakdown

### 2.1 Window Decorations (3-4 days) 🎨

**Priority**: P1 - Users expect windows to have title bars!

#### What We're Building:
- Title bar at top of each window
- Window title text (from app_id)
- Three buttons: Close (×), Maximize (□), Minimize (−)
- Window border (subtle outline)
- Resize handles (invisible but functional)

#### Current State:
✅ DecorationManager exists (`src/decoration/mod.rs`)  
✅ Already tracks window focus  
⚠️ Not rendering decorations to screen yet

#### Implementation Plan:

**Step 1: Design the decoration rendering** (1 hour)
```rust
// In src/decoration/mod.rs
pub struct WindowDecoration {
    pub window_id: u64,
    pub title: String,
    pub has_focus: bool,
    pub buttons: Vec<DecorationButton>,
}

pub struct DecorationButton {
    pub kind: ButtonKind,  // Close, Maximize, Minimize
    pub rect: Rectangle,
    pub hovered: bool,
}
```

**Step 2: Render decorations as overlay quads** (4 hours)
```rust
// Hook into renderer
impl DecorationManager {
    pub fn render_decorations(&self, renderer: &mut AxiomRenderer) {
        for window in &self.windows {
            // Title bar background
            renderer.draw_rect(
                window.title_bar_rect(), 
                if window.focused { FOCUS_COLOR } else { UNFOCUS_COLOR }
            );
            
            // Title text
            renderer.draw_text(
                &window.title,
                window.title_position(),
                TITLE_FONT_SIZE
            );
            
            // Buttons
            for button in &window.buttons {
                renderer.draw_button(button);
            }
        }
    }
}
```

**Step 3: Handle button clicks** (3 hours)
```rust
// In compositor event loop
fn handle_decoration_click(&mut self, x: f64, y: f64) {
    if let Some(button) = self.decoration_manager.hit_test(x, y) {
        match button.kind {
            ButtonKind::Close => {
                // Send xdg_toplevel.close() to client
                self.close_window(button.window_id);
            }
            ButtonKind::Maximize => {
                self.toggle_maximize(button.window_id);
            }
            ButtonKind::Minimize => {
                self.minimize_window(button.window_id);
            }
        }
    }
}
```

**Step 4: Handle window resize via borders** (4 hours)
```rust
// Detect resize edge/corner
pub enum ResizeEdge {
    Top, Bottom, Left, Right,
    TopLeft, TopRight, BottomLeft, BottomRight,
}

impl DecorationManager {
    pub fn detect_resize_edge(&self, x: f64, y: f64) -> Option<(u64, ResizeEdge)> {
        // Check if near window edges (within 5px)
        // Return window_id and which edge/corner
    }
}

// In drag handler
if dragging_resize {
    self.resize_window_interactive(window_id, edge, dx, dy);
}
```

**Files to modify**:
- `src/decoration/mod.rs` - Add rendering methods
- `src/renderer/mod.rs` - Add draw_rect/draw_text helpers
- `src/smithay/server.rs` - Hook decoration clicks

**Success criteria**:
- [ ] All windows show title bars
- [ ] Title text is visible
- [ ] Close button works (sends close to client)
- [ ] Maximize/minimize buttons work
- [ ] Can resize by dragging borders
- [ ] Focused window has different color

---

### 2.2 Tiling Window Management (5-7 days) 📐

**Priority**: P1 - Core feature for productivity

#### What We're Building:
- Automatic horizontal tiling (side-by-side)
- Master/stack layout (one large + stack of small)
- Dynamic resizing (drag to adjust split)
- Gaps between windows (configurable)
- Floating mode toggle (for dialogs)

#### Current State:
✅ WindowManager exists  
✅ Workspace system ready  
⚠️ No automatic tiling logic yet

#### Implementation Plan:

**Step 1: Implement horizontal tiling algorithm** (6 hours)
```rust
// In src/window/mod.rs or new src/tiling/mod.rs
pub struct TilingLayout {
    pub mode: LayoutMode,
    pub gaps: u32,
    pub master_ratio: f32,  // 0.0-1.0
}

pub enum LayoutMode {
    Horizontal,      // All windows side-by-side
    MasterStack,     // One master + stack on right
    Floating,        // Manual positioning
}

impl TilingLayout {
    pub fn compute_layout(
        &self,
        window_ids: &[u64],
        workspace_rect: Rectangle,
    ) -> HashMap<u64, Rectangle> {
        match self.mode {
            LayoutMode::Horizontal => {
                self.layout_horizontal(window_ids, workspace_rect)
            }
            LayoutMode::MasterStack => {
                self.layout_master_stack(window_ids, workspace_rect)
            }
            LayoutMode::Floating => {
                // Keep manual positions
                HashMap::new()
            }
        }
    }
    
    fn layout_horizontal(&self, ids: &[u64], rect: Rectangle) -> HashMap<u64, Rectangle> {
        let n = ids.len();
        if n == 0 { return HashMap::new(); }
        
        let total_gaps = (n - 1) as u32 * self.gaps;
        let usable_width = rect.width.saturating_sub(total_gaps);
        let window_width = usable_width / n as u32;
        
        let mut layouts = HashMap::new();
        for (i, &id) in ids.iter().enumerate() {
            let x = rect.x + (i as u32 * (window_width + self.gaps)) as i32;
            layouts.insert(id, Rectangle {
                x,
                y: rect.y,
                width: window_width,
                height: rect.height,
            });
        }
        layouts
    }
}
```

**Step 2: Integrate with workspace manager** (4 hours)
```rust
// In src/workspace/mod.rs
impl ScrollableWorkspaces {
    pub fn apply_tiling(&mut self) {
        for workspace in &mut self.workspaces {
            let layout = workspace.tiling_layout.compute_layout(
                &workspace.window_ids,
                workspace.bounds,
            );
            
            // Apply computed positions to windows
            for (window_id, rect) in layout {
                self.set_window_geometry(window_id, rect);
            }
        }
    }
}
```

**Step 3: Handle interactive resizing** (6 hours)
```rust
// Resize by dragging window edges
pub fn resize_tiled_window(&mut self, window_id: u64, delta_x: i32, delta_y: i32) {
    // Find window's neighbors
    let neighbors = self.find_adjacent_windows(window_id);
    
    // Adjust this window and neighbors proportionally
    for neighbor in neighbors {
        // Shrink neighbor as we grow this window
    }
    
    // Recompute and apply layout
    self.apply_tiling();
}
```

**Step 4: Add floating mode** (3 hours)
```rust
impl WindowManager {
    pub fn toggle_floating(&mut self, window_id: u64) {
        if let Some(win) = self.get_window_mut(window_id) {
            win.floating = !win.floating;
        }
        // Floating windows excluded from tiling
    }
}
```

**Files to create/modify**:
- `src/tiling/mod.rs` (NEW) - Tiling algorithms
- `src/workspace/mod.rs` - Integration
- `src/window/mod.rs` - Add floating flag
- `src/smithay/server.rs` - Call apply_tiling() on window add/remove

**Success criteria**:
- [ ] New windows automatically tile side-by-side
- [ ] Windows resize to fill available space
- [ ] Can drag to resize tiled windows
- [ ] Master/stack mode works
- [ ] Can toggle floating mode (Super+F)
- [ ] Configurable gaps between windows

---

### 2.3 Multi-Monitor Support (3-5 days) 🖥️🖥️

**Priority**: P1 - Essential for many users

#### What We're Building:
- Detect multiple monitors
- Separate workspace sets per monitor
- Move windows between monitors
- Per-monitor DPI scaling
- Hotplug support (connect/disconnect)

#### Current State:
✅ Multiple outputs detected (run_present_winit.rs)  
✅ LogicalOutput infrastructure exists  
⚠️ Not fully utilized yet

#### Implementation Plan:

**Step 1: Per-monitor workspace management** (4 hours)
```rust
// In src/workspace/mod.rs
pub struct WorkspaceSet {
    pub monitor_id: usize,
    pub workspaces: Vec<Workspace>,
    pub active_index: usize,
}

pub struct MultiMonitorWorkspaces {
    pub workspace_sets: Vec<WorkspaceSet>,  // One set per monitor
    pub focused_monitor: usize,
}

impl MultiMonitorWorkspaces {
    pub fn add_window_to_focused_monitor(&mut self, window_id: u64) {
        let set = &mut self.workspace_sets[self.focused_monitor];
        let workspace = &mut set.workspaces[set.active_index];
        workspace.window_ids.push(window_id);
    }
}
```

**Step 2: Window positioning per monitor** (3 hours)
```rust
// Windows remember which monitor they belong to
pub struct AxiomWindow {
    pub monitor_id: usize,
    pub position_in_monitor: (i32, i32),  // Relative to monitor
    // ...
}

// Convert to global coordinates for rendering
pub fn to_global_coords(&self, monitors: &[Monitor]) -> (i32, i32) {
    let monitor = &monitors[self.monitor_id];
    (
        monitor.x + self.position_in_monitor.0,
        monitor.y + self.position_in_monitor.1,
    )
}
```

**Step 3: Move windows between monitors** (4 hours)
```rust
impl WindowManager {
    pub fn move_window_to_monitor(&mut self, window_id: u64, target_monitor: usize) {
        if let Some(win) = self.get_window_mut(window_id) {
            // Remove from old monitor's workspace
            self.workspaces.remove_window_from_monitor(window_id, win.monitor_id);
            
            // Add to new monitor's active workspace
            win.monitor_id = target_monitor;
            self.workspaces.add_window_to_monitor(window_id, target_monitor);
            
            // Recalculate tiling on both monitors
            self.workspaces.retile_monitor(win.monitor_id);
            self.workspaces.retile_monitor(target_monitor);
        }
    }
}
```

**Step 4: Monitor hotplug handling** (4 hours)
```rust
// In compositor event loop
fn handle_monitor_change(&mut self, event: MonitorEvent) {
    match event {
        MonitorEvent::Connected(info) => {
            // Add new workspace set for this monitor
            self.workspaces.add_monitor(info);
            
            // Create wl_output for clients
            self.advertise_output(info);
        }
        MonitorEvent::Disconnected(id) => {
            // Move all windows from this monitor to primary
            let orphaned_windows = self.workspaces.remove_monitor(id);
            for window_id in orphaned_windows {
                self.move_window_to_monitor(window_id, 0);  // 0 = primary
            }
            
            // Destroy wl_output
            self.remove_output(id);
        }
    }
}
```

**Step 5: DPI scaling** (3 hours)
```rust
pub struct Monitor {
    pub scale: f32,  // 1.0, 1.5, 2.0, etc.
    // ...
}

// Scale window coordinates
fn render_window(&self, window: &AxiomWindow, monitor: &Monitor) {
    let scaled_pos = (
        (window.position.0 as f32 * monitor.scale) as i32,
        (window.position.1 as f32 * monitor.scale) as i32,
    );
    // Render at scaled position
}
```

**Files to modify**:
- `src/workspace/mod.rs` - Multi-monitor workspaces
- `src/window/mod.rs` - Monitor affinity
- `src/smithay/server.rs` - Hotplug events
- `src/bin/run_present_winit.rs` - Already has monitor detection!

**Success criteria**:
- [ ] Each monitor has its own workspace set
- [ ] Windows open on focused monitor
- [ ] Can move windows between monitors (Super+Shift+Arrow)
- [ ] DPI scaling works correctly
- [ ] Monitors can be hotplugged without crash
- [ ] wl_output advertises all monitors correctly

---

### 2.4 Workspace Management (4-5 days) 🗂️

**Priority**: P1 - Core Axiom feature

#### What We're Building:
- Scroll smoothly between workspaces
- Move windows between workspaces
- Render preview of adjacent workspaces
- Per-workspace window visibility
- Wrap-around scrolling (optional)

#### Current State:
✅ ScrollableWorkspaces struct exists  
✅ Basic infrastructure ready  
⚠️ Scrolling animation not implemented  
⚠️ Window movement not implemented

#### Implementation Plan:

**Step 1: Implement workspace scrolling animation** (5 hours)
```rust
// In src/workspace/mod.rs
pub struct WorkspaceTransition {
    pub from_index: usize,
    pub to_index: usize,
    pub progress: f32,  // 0.0 to 1.0
    pub duration_ms: u64,
    pub start_time: Instant,
}

impl ScrollableWorkspaces {
    pub fn scroll_to(&mut self, direction: ScrollDirection) {
        let target = match direction {
            ScrollDirection::Left => self.active_index.saturating_sub(1),
            ScrollDirection::Right => (self.active_index + 1).min(self.workspaces.len() - 1),
        };
        
        self.transition = Some(WorkspaceTransition {
            from_index: self.active_index,
            to_index: target,
            progress: 0.0,
            duration_ms: 300,  // 300ms animation
            start_time: Instant::now(),
        });
    }
    
    pub fn update_transition(&mut self) -> bool {
        if let Some(trans) = &mut self.transition {
            let elapsed = trans.start_time.elapsed().as_millis() as u64;
            trans.progress = (elapsed as f32 / trans.duration_ms as f32).min(1.0);
            
            // Ease out cubic for smooth deceleration
            trans.progress = 1.0 - (1.0 - trans.progress).powi(3);
            
            if trans.progress >= 1.0 {
                self.active_index = trans.to_index;
                self.transition = None;
                return true;  // Animation complete
            }
        }
        false
    }
    
    pub fn get_render_offset(&self) -> f32 {
        if let Some(trans) = &self.transition {
            let workspace_width = self.viewport_width as f32;
            let from_x = trans.from_index as f32 * workspace_width;
            let to_x = trans.to_index as f32 * workspace_width;
            
            // Interpolate between from and to
            from_x + (to_x - from_x) * trans.progress
        } else {
            (self.active_index as f32) * (self.viewport_width as f32)
        }
    }
}
```

**Step 2: Render with scroll offset** (3 hours)
```rust
// In renderer
impl AxiomRenderer {
    pub fn render_workspaces(&mut self, workspaces: &ScrollableWorkspaces) {
        let scroll_offset = workspaces.get_render_offset();
        
        // Render all visible workspaces
        for (i, workspace) in workspaces.workspaces.iter().enumerate() {
            let workspace_x = (i as f32 * workspaces.viewport_width as f32) - scroll_offset;
            
            // Only render if visible (or adjacent for smooth scrolling)
            if workspace_x > -workspaces.viewport_width as f32 
                && workspace_x < workspaces.viewport_width as f32 * 2.0 {
                
                for &window_id in &workspace.window_ids {
                    self.render_window_with_offset(window_id, workspace_x, 0.0);
                }
            }
        }
    }
}
```

**Step 3: Move windows between workspaces** (4 hours)
```rust
impl ScrollableWorkspaces {
    pub fn move_window_to_workspace(&mut self, window_id: u64, target_index: usize) {
        // Remove from current workspace
        for workspace in &mut self.workspaces {
            workspace.window_ids.retain(|&id| id != window_id);
        }
        
        // Add to target workspace
        if target_index < self.workspaces.len() {
            self.workspaces[target_index].window_ids.push(window_id);
            
            // Retile both workspaces
            self.retile_workspace(self.active_index);
            self.retile_workspace(target_index);
        }
    }
    
    pub fn move_focused_window(&mut self, direction: ScrollDirection) {
        if let Some(focused_id) = self.get_focused_window() {
            let target = match direction {
                ScrollDirection::Left => self.active_index.saturating_sub(1),
                ScrollDirection::Right => self.active_index + 1,
            };
            
            self.move_window_to_workspace(focused_id, target);
            
            // Follow the window
            self.scroll_to(direction);
        }
    }
}
```

**Step 4: Workspace visibility** (2 hours)
```rust
impl WorkspaceManager {
    pub fn is_window_visible(&self, window_id: u64) -> bool {
        // Window is visible if:
        // 1. On active workspace, OR
        // 2. On adjacent workspace during transition
        
        let workspace_index = self.find_workspace_for_window(window_id);
        
        if let Some(trans) = &self.transition {
            workspace_index == trans.from_index || workspace_index == trans.to_index
        } else {
            workspace_index == self.active_index
        }
    }
}
```

**Files to modify**:
- `src/workspace/mod.rs` - Animation and movement
- `src/renderer/mod.rs` - Offset rendering
- `src/smithay/server.rs` - Update transition each frame
- `src/input/mod.rs` - Workspace switching keybinds

**Success criteria**:
- [ ] Super+Left/Right scrolls between workspaces smoothly
- [ ] 300ms animation with easing
- [ ] Windows on non-active workspaces are hidden
- [ ] Super+Shift+Left/Right moves focused window
- [ ] Window follows to new workspace
- [ ] Multiple workspaces can be active simultaneously

---

### 2.5 Keyboard Shortcuts (1-2 days) ⌨️

**Priority**: P1 - Essential for usability

#### What We're Building:
- Super+Arrow: Move focus between windows
- Super+Shift+Arrow: Move focused window / Switch workspace
- Super+F: Toggle fullscreen
- Super+Q: Close window
- Super+Enter: Launch terminal (configurable)
- Super+Space: Toggle floating
- Super+1-9: Jump to workspace N

#### Current State:
✅ InputManager exists  
✅ Keybinding infrastructure present  
⚠️ Not all actions wired up

#### Implementation Plan:

**Step 1: Define all compositor actions** (1 hour)
```rust
// In src/input/mod.rs
#[derive(Debug, Clone, PartialEq)]
pub enum CompositorAction {
    // Window management
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    MoveWindowLeft,
    MoveWindowRight,
    MoveWindowUp,
    MoveWindowDown,
    
    // Window operations
    CloseWindow,
    ToggleFullscreen,
    ToggleFloating,
    ToggleMaximize,
    
    // Workspace
    WorkspaceLeft,
    WorkspaceRight,
    WorkspaceJump(u32),  // Jump to specific workspace
    MoveToWorkspaceLeft,
    MoveToWorkspaceRight,
    MoveToWorkspace(u32),
    
    // System
    LaunchTerminal,
    Quit,
}
```

**Step 2: Wire up keybindings** (3 hours)
```rust
// In config file (axiom.toml)
[bindings]
"Super+Left" = "FocusLeft"
"Super+Right" = "FocusRight"
"Super+Shift+Left" = "WorkspaceLeft"
"Super+Shift+Right" = "WorkspaceRight"
"Super+Control+Left" = "MoveWindowLeft"
"Super+Control+Right" = "MoveWindowRight"
"Super+F" = "ToggleFullscreen"
"Super+Q" = "CloseWindow"
"Super+Space" = "ToggleFloating"
"Super+Enter" = "LaunchTerminal"
"Super+1" = { WorkspaceJump = 0 }
"Super+2" = { WorkspaceJump = 1 }
// ... etc
```

**Step 3: Implement action handlers** (6 hours)
```rust
// In compositor main loop
fn handle_action(&mut self, action: CompositorAction) {
    match action {
        CompositorAction::FocusLeft => {
            let current = self.window_manager.focused_window_id();
            if let Some(next) = self.find_window_to_left(current) {
                self.window_manager.focus_window(next);
                self.send_focus_events(next);
            }
        }
        
        CompositorAction::CloseWindow => {
            if let Some(id) = self.window_manager.focused_window_id() {
                // Send xdg_toplevel.close() to client
                self.close_window_gracefully(id);
            }
        }
        
        CompositorAction::WorkspaceLeft => {
            self.workspace_manager.scroll_to(ScrollDirection::Left);
        }
        
        CompositorAction::MoveToWorkspaceLeft => {
            self.workspace_manager.move_focused_window(ScrollDirection::Left);
        }
        
        CompositorAction::LaunchTerminal => {
            std::process::Command::new("weston-terminal")
                .env("WAYLAND_DISPLAY", &self.socket_name)
                .spawn()
                .ok();
        }
        
        // ... etc
    }
}
```

**Files to modify**:
- `src/input/mod.rs` - Action definitions
- `src/config/mod.rs` - Keybinding parsing
- `src/smithay/server.rs` - Action handlers
- `axiom.toml` - Default keybindings

**Success criteria**:
- [ ] All keybindings work as expected
- [ ] Actions are configurable via config file
- [ ] Visual feedback when switching focus
- [ ] Smooth workspace transitions
- [ ] Terminal launches with correct WAYLAND_DISPLAY

---

## 📊 Phase 2 Timeline

| Task | Duration | Dependencies |
|------|----------|--------------|
| 2.1 Window Decorations | 3-4 days | None |
| 2.2 Tiling Management | 5-7 days | 2.1 (decorations need correct sizes) |
| 2.3 Multi-Monitor | 3-5 days | 2.2 (tiling per monitor) |
| 2.4 Workspace Management | 4-5 days | 2.2, 2.3 |
| 2.5 Keyboard Shortcuts | 1-2 days | All above |

**Total**: 16-23 days (~3-4 weeks)

---

## 🎯 Phase 2 Success Criteria

At the end of Phase 2, Axiom should be:

- [ ] **Visually complete**: Windows have title bars and borders
- [ ] **Intelligently tiling**: Windows automatically arrange themselves
- [ ] **Multi-monitor ready**: Works correctly with 2+ displays
- [ ] **Workspace fluent**: Smooth transitions between workspaces
- [ ] **Keyboard efficient**: Can manage windows without mouse
- [ ] **Daily driveable**: Can use it for 8+ hours of work

---

## 🚀 Let's Start!

**Recommended order**:
1. Start with **Window Decorations** - Visual impact, builds confidence
2. Then **Tiling** - Core functionality
3. Then **Keyboard Shortcuts** - Makes testing easier
4. Then **Multi-Monitor** - Expand capability
5. Finally **Workspace animations** - Polish

Would you like me to start implementing window decorations?
