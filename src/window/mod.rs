//! Window management system
//! Handles window placement, focusing, and layout algorithms

use crate::config::WindowConfig;
use anyhow::Result;
use std::collections::HashMap;

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

    /// Custom window opacity (0.0 - 1.0)
    pub opacity: f32,

    /// Custom border radius (for effects)
    pub border_radius: u32,
}

impl Default for WindowProperties {
    fn default() -> Self {
        Self {
            floating: false,
            fullscreen: false,
            maximized: false,
            opacity: 1.0,
            border_radius: 0,
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

#[derive(Debug)]
pub struct WindowManager {
    #[allow(dead_code)]
    config: WindowConfig,

    /// Window tracking
    windows: HashMap<u64, AxiomWindow>,

    /// Next window ID
    next_window_id: u64,

    /// Currently focused window
    focused_window: Option<u64>,
}

impl WindowManager {
    pub fn new(config: &WindowConfig) -> Result<Self> {
        Ok(Self {
            config: config.clone(),
            windows: HashMap::new(),
            next_window_id: 1,
            focused_window: None,
        })
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

        // Focus the new window if no window is currently focused
        if self.focused_window.is_none() {
            self.focused_window = Some(id);
        }

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

    /// Focus a window
    #[allow(dead_code)]
    pub fn focus_window(&mut self, id: u64) -> Result<()> {
        if self.windows.contains_key(&id) {
            self.focused_window = Some(id);
        }
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
    #[allow(dead_code)]
    pub fn calculate_layout(&self, workspace_bounds: Rectangle) -> Vec<(u64, Rectangle)> {
        let windows_in_workspace: Vec<_> = self
            .windows
            .values()
            .filter(|w| !w.properties.floating && !w.properties.fullscreen)
            .collect();

        if windows_in_workspace.is_empty() {
            return Vec::new();
        }

        let mut layouts = Vec::new();
        let gap = self.config.gap as i32;

        match self.config.default_layout.as_str() {
            "horizontal" => {
                // Horizontal tiling layout (like niri)
                let available_width =
                    workspace_bounds.width as i32 - (gap * (windows_in_workspace.len() as i32 + 1));
                let window_width = available_width / windows_in_workspace.len() as i32;

                for (i, window) in windows_in_workspace.iter().enumerate() {
                    let x = workspace_bounds.x + gap + i as i32 * (window_width + gap);
                    let y = workspace_bounds.y + gap;
                    let w = window_width as u32;
                    let h = workspace_bounds.height - 2 * gap as u32;

                    layouts.push((
                        window.window.id,
                        Rectangle::from_loc_and_size((x, y), (w, h)),
                    ));
                }
            }
            "vertical" => {
                // Vertical tiling layout
                let available_height = workspace_bounds.height as i32
                    - (gap * (windows_in_workspace.len() as i32 + 1));
                let window_height = available_height / windows_in_workspace.len() as i32;

                for (i, window) in windows_in_workspace.iter().enumerate() {
                    let x = workspace_bounds.x + gap;
                    let y = workspace_bounds.y + gap + i as i32 * (window_height + gap);
                    let w = workspace_bounds.width - 2 * gap as u32;
                    let h = window_height as u32;

                    layouts.push((
                        window.window.id,
                        Rectangle::from_loc_and_size((x, y), (w, h)),
                    ));
                }
            }
            _ => {
                // Default to horizontal
                return self.calculate_layout(workspace_bounds);
            }
        }

        layouts
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

    pub fn shutdown(&mut self) -> Result<()> {
        self.windows.clear();
        Ok(())
    }
}
