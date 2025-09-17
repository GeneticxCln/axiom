//! Core Window Management System
//!
//! This module implements comprehensive window management functionality including:
//! - Basic operations (move, resize, close)
//! - Window focus and decorations  
//! - Popup/dialog handling
//! - Z-order stacking
//! - Tiling layouts
//!
//! The window management system is designed to handle both Wayland and X11 windows
//! through a unified interface, with proper integration into the Axiom compositor.

use crate::config::WindowConfig;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use log::debug;

/// Window operation types for the compositor
#[derive(Debug, Clone, PartialEq)]
pub enum WindowOperation {
    Move { x: i32, y: i32 },
    Resize { width: u32, height: u32 },
    Close,
    Minimize,
    Maximize,
    Restore,
    ToggleFullscreen,
    Focus,
    Unfocus,
    MoveToWorkspace(u32),
    SetAlwaysOnTop(bool),
    SetOpacity(f32),
}

/// Window focus event types
#[derive(Debug, Clone, PartialEq)]
pub enum FocusEvent {
    WindowFocused(u64),
    WindowUnfocused(u64),
    FocusLost,
}

/// Window layer types for Z-ordering
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WindowLayer {
    Background = 0,
    Normal = 1,
    AboveNormal = 2,
    AlwaysOnTop = 3,
    Overlay = 4,
    Notification = 5,
}

/// Window type classification
#[derive(Debug, Clone, PartialEq)]
pub enum WindowType {
    Normal,
    Dialog,
    Modal,
    Popup,
    Tooltip,
    Menu,
    Notification,
    Splash,
    Utility,
    Toolbar,
}

/// Animation state for window transitions
#[derive(Debug, Clone, PartialEq)]
pub struct WindowAnimationState {
    pub animation_type: AnimationType,
    pub start_time: Instant,
    pub duration: Duration,
    pub start_rect: Rectangle,
    pub end_rect: Rectangle,
    pub progress: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnimationType {
    Move,
    Resize,
    FadeIn,
    FadeOut,
    Minimize,
    Maximize,
    Restore,
}

/// Window constraints for size and positioning
#[derive(Debug, Clone, PartialEq)]
pub struct WindowConstraints {
    pub min_width: Option<u32>,
    pub min_height: Option<u32>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub aspect_ratio: Option<(u32, u32)>,
    pub resizable: bool,
    pub movable: bool,
}

impl Default for WindowConstraints {
    fn default() -> Self {
        Self {
            min_width: Some(100),
            min_height: Some(50),
            max_width: None,
            max_height: None,
            aspect_ratio: None,
            resizable: true,
            movable: true,
        }
    }
}

// Backend window type

// Minimal fallback backend window when experimental-smithay is disabled
#[derive(Debug, Clone, PartialEq)]
pub struct BackendWindow {
    pub id: u64,
    pub title: String,
    pub position: (i32, i32),
    pub size: (u32, u32),
}

impl BackendWindow {
    pub fn new(id: u64, title: String) -> Self {
        Self { id, title, position: (0, 0), size: (800, 600) }
    }
    pub fn set_position(&mut self, x: i32, y: i32) { self.position = (x, y); }
    pub fn set_size(&mut self, width: u32, height: u32) { self.size = (width, height); }
}

/// Rectangle for window positioning and sizing
#[derive(Debug, Clone, PartialEq)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rectangle {
    #[allow(dead_code)]
    pub fn from_loc_and_size((x, y): (i32, i32), (width, height): (u32, u32)) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Enhanced window wrapper for Axiom-specific functionality
#[derive(Debug, Clone, PartialEq)]
pub struct AxiomWindow {
    /// The underlying backend window
    pub window: BackendWindow,

    /// Current workspace position (for scrollable workspaces)
    pub workspace_position: f64,

    /// Window-specific properties
    pub properties: WindowProperties,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowProperties {
    /// Whether the window is floating
    pub floating: bool,

    /// Whether the window is fullscreen
    pub fullscreen: bool,

    /// Whether the window is maximized
    pub maximized: bool,

    /// Whether the window is minimized
    pub minimized: bool,

    /// Custom window opacity (0.0 - 1.0)
    pub opacity: f32,

    /// Custom border radius (for effects)
    pub border_radius: u32,

    /// Window layer for Z-ordering
    pub layer: WindowLayer,

    /// Window type classification
    pub window_type: WindowType,

    /// Whether window should always stay on top
    pub always_on_top: bool,

    /// Whether window has decorations
    pub decorated: bool,

    /// Whether window is modal
    pub modal: bool,

    /// Parent window ID for dialogs/popups
    pub parent_id: Option<u64>,

    /// Window constraints
    pub constraints: WindowConstraints,

    /// Saved position and size for restore operations
    pub saved_rect: Option<Rectangle>,

    /// Current animation state
    pub animation_state: Option<WindowAnimationState>,
}

impl Default for WindowProperties {
    fn default() -> Self {
        Self {
            floating: false,
            fullscreen: false,
            maximized: false,
            minimized: false,
            opacity: 1.0,
            border_radius: 0,
            layer: WindowLayer::Normal,
            window_type: WindowType::Normal,
            always_on_top: false,
            decorated: true,
            modal: false,
            parent_id: None,
            constraints: WindowConstraints::default(),
            saved_rect: None,
            animation_state: None,
        }
    }
}

impl AxiomWindow {
    /// Create a new AxiomWindow
    pub fn new(id: u64, title: String) -> Self {
        Self {
            window: BackendWindow::new(id, title),
            workspace_position: 0.0,
            properties: WindowProperties::default(),
        }
    }
}

pub struct WindowManager {
    config: WindowConfig,

    /// Window tracking by ID
    windows: HashMap<u64, AxiomWindow>,

    /// Window stacking order (bottom to top)
    stacking_order: VecDeque<u64>,

    /// Windows by layer for Z-ordering
    layers: HashMap<WindowLayer, Vec<u64>>,

    /// Next window ID
    next_window_id: u64,

    /// Currently focused window
    focused_window: Option<u64>,

    /// Focus history for cycling
    focus_history: VecDeque<u64>,

    /// Modal window stack
    modal_stack: Vec<u64>,

    /// Popup/dialog tracking
    popups: HashMap<u64, Vec<u64>>, // parent_id -> [child_ids]

    /// Pending operations queue
    pending_operations: VecDeque<(u64, WindowOperation)>,

    /// Focus event listeners
    focus_listeners: Vec<Box<dyn Fn(FocusEvent) + Send + Sync>>,
}

impl WindowManager {
    /// Create a new WindowManager with comprehensive window management capabilities
    pub fn new(config: &WindowConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            windows: HashMap::new(),
            stacking_order: VecDeque::new(),
            layers: HashMap::new(),
            next_window_id: 1,
            focused_window: None,
            focus_history: VecDeque::new(),
            modal_stack: Vec::new(),
            popups: HashMap::new(),
            pending_operations: VecDeque::new(),
            focus_listeners: Vec::new(),
        })
    }

    /// Set whether a window should be decorated by the server (SSD) or not (CSD)
    /// Returns true if the value changed.
    pub fn set_window_decorated(&mut self, window_id: u64, decorated: bool) -> bool {
        if let Some(win) = self.windows.get_mut(&window_id) {
            if win.properties.decorated != decorated {
                win.properties.decorated = decorated;
                debug!("Window {} decorated set to {}", window_id, decorated);
                return true;
            }
        }
        false
    }

    /// Execute a window operation
    pub fn execute_operation(&mut self, window_id: u64, operation: WindowOperation) -> Result<()> {
        debug!("Executing window operation: {:?} on window {}", operation, window_id);
        
        if !self.windows.contains_key(&window_id) {
            return Err(anyhow!("Window {} not found", window_id));
        }

        match operation {
            WindowOperation::Move { x, y } => self.move_window(window_id, x, y),
            WindowOperation::Resize { width, height } => self.resize_window(window_id, width, height),
            WindowOperation::Close => self.close_window(window_id),
            WindowOperation::Minimize => self.minimize_window(window_id),
            WindowOperation::Maximize => self.maximize_window(window_id),
            WindowOperation::Restore => self.restore_window(window_id),
            WindowOperation::ToggleFullscreen => self.toggle_fullscreen(window_id),
            WindowOperation::Focus => { self.focus_window(window_id)?; Ok(()) },
            WindowOperation::Unfocus => self.unfocus_window(window_id),
            WindowOperation::MoveToWorkspace(workspace) => self.move_window_to_workspace(window_id, workspace),
            WindowOperation::SetAlwaysOnTop(always_on_top) => self.set_always_on_top(window_id, always_on_top),
            WindowOperation::SetOpacity(opacity) => self.set_window_opacity(window_id, opacity),
        }
    }

    /// Move a window to new coordinates
    pub fn move_window(&mut self, window_id: u64, x: i32, y: i32) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if !window.properties.constraints.movable {
                return Err(anyhow!("Window {} is not movable", window_id));
            }
            
            window.window.set_position(x, y);
            debug!("Moved window {} to ({}, {})", window_id, x, y);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Resize a window with constraint validation
    pub fn resize_window(&mut self, window_id: u64, width: u32, height: u32) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if !window.properties.constraints.resizable {
                return Err(anyhow!("Window {} is not resizable", window_id));
            }

            // Apply constraints
            let constraints = &window.properties.constraints;
            let mut final_width = width;
            let mut final_height = height;

            // Apply minimum constraints
            if let Some(min_width) = constraints.min_width {
                final_width = final_width.max(min_width);
            }
            if let Some(min_height) = constraints.min_height {
                final_height = final_height.max(min_height);
            }

            // Apply maximum constraints
            if let Some(max_width) = constraints.max_width {
                final_width = final_width.min(max_width);
            }
            if let Some(max_height) = constraints.max_height {
                final_height = final_height.min(max_height);
            }

            // Apply aspect ratio constraints
            if let Some((aspect_w, aspect_h)) = constraints.aspect_ratio {
                let aspect_ratio = aspect_w as f32 / aspect_h as f32;
                let current_ratio = final_width as f32 / final_height as f32;
                
                if (current_ratio - aspect_ratio).abs() > 0.01 {
                    // Adjust to maintain aspect ratio, preferring width
                    final_height = (final_width as f32 / aspect_ratio) as u32;
                }
            }

            window.window.set_size(final_width, final_height);
            debug!("Resized window {} to {}x{}", window_id, final_width, final_height);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Close a window (marks for removal)
    pub fn close_window(&mut self, window_id: u64) -> Result<()> {
        // Remove from all tracking structures
        self.remove_window_from_stacking(window_id);
        self.remove_window_from_layers(window_id);
        self.remove_from_focus_history(window_id);
        
        // Handle modal/popup cleanup
        self.cleanup_modal_stack(window_id);
        self.cleanup_popup_hierarchy(window_id);
        
        // If focused window, focus next in stack
        if self.focused_window == Some(window_id) {
            self.focus_next_window();
        }
        
        debug!("Closed window {}", window_id);
        Ok(())
    }

    /// Minimize a window
    pub fn minimize_window(&mut self, window_id: u64) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if !window.properties.minimized {
                // Save current state for restore
                window.properties.saved_rect = Some(Rectangle {
                    x: window.window.position.0,
                    y: window.window.position.1,
                    width: window.window.size.0,
                    height: window.window.size.1,
                });
                
                window.properties.minimized = true;
                
                // Remove from stacking order when minimized
                self.remove_window_from_stacking(window_id);
                
                // Focus next window if this was focused
                if self.focused_window == Some(window_id) {
                    self.focus_next_window();
                }
                
                debug!("Minimized window {}", window_id);
            }
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Maximize a window
    pub fn maximize_window(&mut self, window_id: u64) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if !window.properties.maximized {
                // Save current state for restore
                window.properties.saved_rect = Some(Rectangle {
                    x: window.window.position.0,
                    y: window.window.position.1,
                    width: window.window.size.0,
                    height: window.window.size.1,
                });
                
                window.properties.maximized = true;
                window.properties.minimized = false; // Can't be both
                
                // Geometry will be provided by the layout engine (workspace/compositor)
                debug!("Maximized window {}", window_id);
            }
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Restore a window from minimized/maximized state
    pub fn restore_window(&mut self, window_id: u64) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            if window.properties.minimized || window.properties.maximized {
                // Restore saved state if available
                if let Some(saved_rect) = window.properties.saved_rect.take() {
                    window.window.set_position(saved_rect.x, saved_rect.y);
                    window.window.set_size(saved_rect.width, saved_rect.height);
                }
                
                window.properties.minimized = false;
                window.properties.maximized = false;
                
                // Re-add to stacking if was minimized
                if !self.stacking_order.contains(&window_id) {
                    self.add_window_to_stacking(window_id);
                }
                
                debug!("Restored window {}", window_id);
            }
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Set window always on top status
    pub fn set_always_on_top(&mut self, window_id: u64, always_on_top: bool) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.properties.always_on_top = always_on_top;
            
            // Update layer based on always on top
            let new_layer = if always_on_top {
                WindowLayer::AlwaysOnTop
            } else {
                WindowLayer::Normal
            };
            
            self.move_window_to_layer(window_id, new_layer)?;
            debug!("Set window {} always on top: {}", window_id, always_on_top);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Set window opacity
    pub fn set_window_opacity(&mut self, window_id: u64, opacity: f32) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.properties.opacity = opacity.clamp(0.0, 1.0);
            debug!("Set window {} opacity to {:.2}", window_id, window.properties.opacity);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Move window to workspace
    pub fn move_window_to_workspace(&mut self, window_id: u64, workspace: u32) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.workspace_position = workspace as f64;
            debug!("Moved window {} to workspace {}", window_id, workspace);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }

    /// Unfocus a window
    pub fn unfocus_window(&mut self, window_id: u64) -> Result<()> {
        if self.focused_window == Some(window_id) {
            self.focused_window = None;
            self.emit_focus_event(FocusEvent::WindowUnfocused(window_id));
            debug!("Unfocused window {}", window_id);
        }
        Ok(())
    }

    /// Add a new window to management
    pub fn add_window(&mut self, title: String) -> u64 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let backend_window = BackendWindow::new(id, title);
        let axiom_window = AxiomWindow {
            window: backend_window,
            workspace_position: 0.0, // Start at workspace 0
            properties: WindowProperties::default(),
        };

        self.windows.insert(id, axiom_window);
        
        // Add to stacking order
        self.add_window_to_stacking(id);
        
        // Add to appropriate layer
        self.add_window_to_layer(id, WindowLayer::Normal);

        // Focus the new window if no window is currently focused
        if self.focused_window.is_none() {
            let _ = self.focus_window(id);
        }

        debug!("Added window {} with title: {}", id, self.windows[&id].window.title);
        id
    }

    /// Remove a window from management
    pub fn remove_window(&mut self, id: u64) -> Option<AxiomWindow> {
        if self.focused_window == Some(id) {
            self.focused_window = None;
        }
        self.windows.remove(&id)
    }

    /// Get a window by ID
    #[allow(dead_code)]
    pub fn get_window(&self, id: u64) -> Option<&AxiomWindow> {
        self.windows.get(&id)
    }

    /// Get a mutable window by ID
    pub fn get_window_mut(&mut self, id: u64) -> Option<&mut AxiomWindow> {
        self.windows.get_mut(&id)
    }

    /// Get all windows
    #[allow(dead_code)]
    pub fn windows(&self) -> impl Iterator<Item = &AxiomWindow> {
        self.windows.values()
    }

    /// Focus a window with comprehensive focus management
    pub fn focus_window(&mut self, id: u64) -> Result<()> {
        if !self.windows.contains_key(&id) {
            return Err(anyhow!("Window {} not found", id));
        }
        
        // Check if window can be focused (not minimized)
        if let Some(window) = self.windows.get(&id) {
            if window.properties.minimized {
                return Err(anyhow!("Cannot focus minimized window {}", id));
            }
        }
        
        // Unfocus current window if different
        if let Some(current_focus) = self.focused_window {
            if current_focus != id {
                self.emit_focus_event(FocusEvent::WindowUnfocused(current_focus));
                
                // Add current focus to history
                self.add_to_focus_history(current_focus);
            }
        }
        
        // Set new focus
        self.focused_window = Some(id);
        
        // Bring window to front of its layer
        self.bring_window_to_front(id);
        
        // Emit focus event
        self.emit_focus_event(FocusEvent::WindowFocused(id));
        
        debug!("Focused window {}", id);
        Ok(())
    }

    /// Get the currently focused window
    #[allow(dead_code)]
    pub fn focused_window(&self) -> Option<&AxiomWindow> {
        self.focused_window.and_then(|id| self.windows.get(&id))
    }

    /// Get the currently focused window id
    pub fn focused_window_id(&self) -> Option<u64> {
        self.focused_window
    }

    /// Calculate window layout for tiling
    /// Supports layouts: horizontal, vertical, master-stack, grid
    pub fn calculate_layout(&self, workspace_bounds: Rectangle) -> Vec<(u64, Rectangle)> {
        // Filter windows by workspace and state
        let mut windows_in_workspace: Vec<_> = self
            .windows
            .values()
            .filter(|w| !w.properties.floating && !w.properties.fullscreen && !w.properties.minimized)
            .collect();

        if windows_in_workspace.is_empty() {
            return Vec::new();
        }

        // Stable order based on stacking within Normal and AboveNormal layers
        windows_in_workspace.sort_by_key(|w| {
            // find index in stacking; default to 0
            self.stacking_order
                .iter()
                .position(|&id| id == w.window.id)
                .unwrap_or(0)
        });

        let mut layouts = Vec::new();
        let gap = self.config.gap as i32;

        match self.config.default_layout.as_str() {
            "horizontal" => {
                // Horizontal tiling layout (like niri)
                let n = windows_in_workspace.len() as i32;
                let available_width = workspace_bounds.width as i32 - gap * (n + 1);
                let window_width = if n > 0 { available_width / n } else { available_width };

                for (i, window) in windows_in_workspace.iter().enumerate() {
                    let x = workspace_bounds.x + gap + i as i32 * (window_width + gap);
                    let y = workspace_bounds.y + gap;
                    let w = window_width.max(1) as u32;
                    let h = (workspace_bounds.height as i32 - 2 * gap).max(1) as u32;
                    layouts.push((window.window.id, Rectangle::from_loc_and_size((x, y), (w, h))));
                }
            }
            "vertical" => {
                // Vertical tiling layout
                let n = windows_in_workspace.len() as i32;
                let available_height = workspace_bounds.height as i32 - gap * (n + 1);
                let window_height = if n > 0 { available_height / n } else { available_height };

                for (i, window) in windows_in_workspace.iter().enumerate() {
                    let x = workspace_bounds.x + gap;
                    let y = workspace_bounds.y + gap + i as i32 * (window_height + gap);
                    let w = (workspace_bounds.width as i32 - 2 * gap).max(1) as u32;
                    let h = window_height.max(1) as u32;
                    layouts.push((window.window.id, Rectangle::from_loc_and_size((x, y), (w, h))));
                }
            }
            "master-stack" => {
                // Master on left, stack on right
                let n = windows_in_workspace.len();
                let master_ratio = 0.6f32;
                let master_w = ((workspace_bounds.width as f32) * master_ratio) as u32;
                let stack_w = workspace_bounds.width - master_w - gap as u32;

                // Master
                let master = windows_in_workspace[0];
                let mx = workspace_bounds.x + gap;
                let my = workspace_bounds.y + gap;
                let mw = (master_w as i32 - gap).max(1) as u32;
                let mh = (workspace_bounds.height as i32 - 2 * gap).max(1) as u32;
                layouts.push((master.window.id, Rectangle::from_loc_and_size((mx, my), (mw, mh))));

                if n > 1 {
                    // Stack vertically on the right
                    let stack_count = n - 1;
                    let available_height = workspace_bounds.height as i32 - gap * (stack_count as i32 + 1);
                    let each_h = if stack_count > 0 { available_height / stack_count as i32 } else { available_height };
                    for (i, window) in windows_in_workspace.iter().enumerate().skip(1) {
                        let x = workspace_bounds.x + gap + master_w as i32 + gap;
                        let y = workspace_bounds.y + gap + (i as i32 - 1) * (each_h + gap);
                        let w = (stack_w as i32).max(1) as u32;
                        let h = each_h.max(1) as u32;
                        layouts.push((window.window.id, Rectangle::from_loc_and_size((x, y), (w, h))));
                    }
                }
            }
            "grid" => {
                // Approximate square grid
                let n = windows_in_workspace.len() as i32;
                let cols = (f32::sqrt(n as f32)).ceil() as i32;
                let rows = ((n as f32) / (cols as f32)).ceil() as i32;
                let cell_w = (workspace_bounds.width as i32 - gap * (cols + 1)) / cols.max(1);
                let cell_h = (workspace_bounds.height as i32 - gap * (rows + 1)) / rows.max(1);

                for (i, window) in windows_in_workspace.iter().enumerate() {
                    let r = (i as i32) / cols;
                    let c = (i as i32) % cols;
                    let x = workspace_bounds.x + gap + c * (cell_w + gap);
                    let y = workspace_bounds.y + gap + r * (cell_h + gap);
                    let w = cell_w.max(1) as u32;
                    let h = cell_h.max(1) as u32;
                    layouts.push((window.window.id, Rectangle::from_loc_and_size((x, y), (w, h))));
                }
            }
            _ => {
                // Default to horizontal
                let mut cfg = self.config.clone();
                cfg.default_layout = "horizontal".into();
                // Use a temporary manager to avoid changing self
                let mut tmp = self.clone_for_layout();
                tmp.config = cfg;
                return tmp.calculate_layout(workspace_bounds);
            }
        }

        layouts
    }

    fn clone_for_layout(&self) -> Self {
        Self {
            config: self.config.clone(),
            windows: self.windows.clone(),
            stacking_order: self.stacking_order.clone(),
            layers: self.layers.clone(),
            next_window_id: self.next_window_id,
            focused_window: self.focused_window,
            focus_history: self.focus_history.clone(),
            modal_stack: self.modal_stack.clone(),
            popups: self.popups.clone(),
            pending_operations: self.pending_operations.clone(),
            focus_listeners: Vec::new(), // listeners not needed for layout clone
        }
    }

    /// Set window properties
    #[allow(dead_code)]
    pub fn set_window_properties(&mut self, id: u64, properties: WindowProperties) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&id) {
            window.properties = properties;
        }
        Ok(())
    }

    /// Toggle fullscreen for a window
    pub fn toggle_fullscreen(&mut self, id: u64) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&id) {
            window.properties.fullscreen = !window.properties.fullscreen;
        }
        Ok(())
    }

    /// Focus next window in stacking order
    pub fn focus_next_window(&mut self) {
        if let Some(next_id) = self.get_next_focusable_window() {
            let _ = self.focus_window(next_id);
        } else {
            self.focused_window = None;
            self.emit_focus_event(FocusEvent::FocusLost);
        }
    }
    
    /// Focus previous window in history
    pub fn focus_previous_window(&mut self) {
        if let Some(prev_id) = self.focus_history.pop_back() {
            // Make sure the window still exists
            if self.windows.contains_key(&prev_id) {
                let _ = self.focus_window(prev_id);
            } else {
                // Try next in history
                self.focus_previous_window();
            }
        } else {
            self.focus_next_window();
        }
    }
    
    /// Cycle focus to next window
    pub fn cycle_focus_forward(&mut self) {
        if let Some(current) = self.focused_window {
            if let Some(next) = self.get_next_window_in_cycle(current) {
                let _ = self.focus_window(next);
            }
        } else {
            self.focus_next_window();
        }
    }
    
    /// Cycle focus to previous window
    pub fn cycle_focus_backward(&mut self) {
        if let Some(current) = self.focused_window {
            if let Some(prev) = self.get_previous_window_in_cycle(current) {
                let _ = self.focus_window(prev);
            }
        } else {
            self.focus_next_window();
        }
    }

    // === Stacking Order Management ===
    
    fn add_window_to_stacking(&mut self, window_id: u64) {
        if !self.stacking_order.contains(&window_id) {
            self.stacking_order.push_back(window_id);
        }
    }
    
    fn remove_window_from_stacking(&mut self, window_id: u64) {
        self.stacking_order.retain(|&id| id != window_id);
    }
    
    fn bring_window_to_front(&mut self, window_id: u64) {
        self.remove_window_from_stacking(window_id);
        self.stacking_order.push_back(window_id);
    }
    
    /// Get windows in stacking order (bottom to top)
    pub fn get_stacking_order(&self) -> Vec<u64> {
        self.stacking_order.iter().cloned().collect()
    }
    
    /// Get windows in reverse stacking order (top to bottom)
    pub fn get_reverse_stacking_order(&self) -> Vec<u64> {
        self.stacking_order.iter().rev().cloned().collect()
    }
    
    // === Layer Management ===
    
    fn add_window_to_layer(&mut self, window_id: u64, layer: WindowLayer) {
        let layer_windows = self.layers.entry(layer).or_insert_with(Vec::new);
        if !layer_windows.contains(&window_id) {
            layer_windows.push(window_id);
        }
    }
    
    fn remove_window_from_layers(&mut self, window_id: u64) {
        for layer_windows in self.layers.values_mut() {
            layer_windows.retain(|&id| id != window_id);
        }
    }
    
    pub fn set_window_layer(&mut self, window_id: u64, new_layer: WindowLayer) -> Result<()> {
        // Remove from all layers first
        self.remove_window_from_layers(window_id);
        
        // Add to new layer
        self.add_window_to_layer(window_id, new_layer);
        
        // Update window properties
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.properties.layer = new_layer;
        }
        
        Ok(())
    }
    
    pub fn move_window_to_layer(&mut self, window_id: u64, new_layer: WindowLayer) -> Result<()> {
        // Remove from all layers first
        self.remove_window_from_layers(window_id);
        
        // Add to new layer
        self.add_window_to_layer(window_id, new_layer);
        
        // Update window properties
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.properties.layer = new_layer;
        }
        
        Ok(())
    }
    
    /// Get windows in a specific layer
    pub fn get_windows_in_layer(&self, layer: WindowLayer) -> Vec<u64> {
        self.layers.get(&layer).cloned().unwrap_or_default()
    }
    
    /// Get all windows sorted by layer and stacking order
    pub fn get_windows_by_render_order(&self) -> Vec<u64> {
        let mut result = Vec::new();
        
        // Add windows from each layer in order (background to notification)
        for layer in [WindowLayer::Background, WindowLayer::Normal, WindowLayer::AboveNormal, 
                     WindowLayer::AlwaysOnTop, WindowLayer::Overlay, WindowLayer::Notification] {
            if let Some(layer_windows) = self.layers.get(&layer) {
                // Sort by stacking order within layer
                let mut sorted_windows: Vec<u64> = layer_windows
                    .iter()
                    .filter(|&&id| self.stacking_order.contains(&id))
                    .cloned()
                    .collect();
                
                sorted_windows.sort_by_key(|&id| {
                    self.stacking_order.iter().position(|&stacked_id| stacked_id == id).unwrap_or(0)
                });
                
                result.extend(sorted_windows);
            }
        }
        
        result
    }
    
    // === Focus History Management ===
    
    fn add_to_focus_history(&mut self, window_id: u64) {
        // Remove if already in history to avoid duplicates
        self.focus_history.retain(|&id| id != window_id);
        
        // Add to back (most recent)
        self.focus_history.push_back(window_id);
        
        // Keep history limited
        while self.focus_history.len() > 10 {
            self.focus_history.pop_front();
        }
    }
    
    fn remove_from_focus_history(&mut self, window_id: u64) {
        self.focus_history.retain(|&id| id != window_id);
    }
    
    // === Modal and Popup Management ===
    
    /// Add window as modal
    pub fn set_window_modal(&mut self, window_id: u64, parent_id: Option<u64>) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.properties.modal = true;
            window.properties.parent_id = parent_id;
            window.properties.window_type = WindowType::Modal;
            
            // Add to modal stack
            self.modal_stack.push(window_id);
            
            // Move to overlay layer
            self.move_window_to_layer(window_id, WindowLayer::Overlay)?;
            
            // Focus the modal window
            self.focus_window(window_id)?;
            
            debug!("Set window {} as modal", window_id);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }
    
    /// Add popup window
    pub fn add_popup(&mut self, window_id: u64, parent_id: u64) -> Result<()> {
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.properties.parent_id = Some(parent_id);
            window.properties.window_type = WindowType::Popup;
            
            // Add to popup tracking
            self.popups.entry(parent_id).or_insert_with(Vec::new).push(window_id);
            
            debug!("Added popup window {} for parent {}", window_id, parent_id);
            Ok(())
        } else {
            Err(anyhow!("Window {} not found", window_id))
        }
    }
    
    fn cleanup_modal_stack(&mut self, window_id: u64) {
        self.modal_stack.retain(|&id| id != window_id);
    }
    
    fn cleanup_popup_hierarchy(&mut self, window_id: u64) {
        // Remove as popup child
        for children in self.popups.values_mut() {
            children.retain(|&id| id != window_id);
        }
        
        // Remove as popup parent and close children
        if let Some(children) = self.popups.remove(&window_id) {
            for child_id in children {
                let _ = self.close_window(child_id);
            }
        }
    }
    
    // === Helper Methods ===
    
    fn get_next_focusable_window(&self) -> Option<u64> {
        // Try focus history first
        for &window_id in self.focus_history.iter().rev() {
            if let Some(window) = self.windows.get(&window_id) {
                if !window.properties.minimized {
                    return Some(window_id);
                }
            }
        }
        
        // Fallback to stacking order
        for &window_id in self.stacking_order.iter().rev() {
            if let Some(window) = self.windows.get(&window_id) {
                if !window.properties.minimized {
                    return Some(window_id);
                }
            }
        }
        
        None
    }
    
    fn get_next_window_in_cycle(&self, current_id: u64) -> Option<u64> {
        let focusable: Vec<u64> = self.stacking_order
            .iter()
            .filter(|&&id| {
                if let Some(window) = self.windows.get(&id) {
                    !window.properties.minimized
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        
        if let Some(current_pos) = focusable.iter().position(|&id| id == current_id) {
            let next_pos = (current_pos + 1) % focusable.len();
            focusable.get(next_pos).cloned()
        } else {
            focusable.first().cloned()
        }
    }
    
    fn get_previous_window_in_cycle(&self, current_id: u64) -> Option<u64> {
        let focusable: Vec<u64> = self.stacking_order
            .iter()
            .filter(|&&id| {
                if let Some(window) = self.windows.get(&id) {
                    !window.properties.minimized
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        
        if let Some(current_pos) = focusable.iter().position(|&id| id == current_id) {
            let prev_pos = if current_pos == 0 { 
                focusable.len() - 1 
            } else { 
                current_pos - 1 
            };
            focusable.get(prev_pos).cloned()
        } else {
            focusable.last().cloned()
        }
    }
    
    /// Emit focus event to listeners
    fn emit_focus_event(&self, event: FocusEvent) {
        for listener in &self.focus_listeners {
            listener(event.clone());
        }
    }
    
    /// Add focus event listener
    pub fn add_focus_listener<F>(&mut self, listener: F) 
    where
        F: Fn(FocusEvent) + Send + Sync + 'static,
    {
        self.focus_listeners.push(Box::new(listener));
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.windows.clear();
        self.stacking_order.clear();
        self.layers.clear();
        self.focus_history.clear();
        self.modal_stack.clear();
        self.popups.clear();
        self.pending_operations.clear();
        self.focus_listeners.clear();
        Ok(())
    }
}
