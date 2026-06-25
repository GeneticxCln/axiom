//! Window management system
//!
//! Handles window placement, focusing, and layout algorithms.
//! Manages the lifecycle of AxiomWindow instances and provides
//! iteration and query interfaces for the compositor.

use crate::config::WindowConfig;
use std::collections::HashMap;

/// Backend-agnostic window record that the Smithay backend populates with the
/// raw geometry it receives from Wayland. Axiom-specific behaviour lives in
/// [`AxiomWindow`] (subscriber of a `BackendWindow`).
#[derive(Debug, Clone, PartialEq)]
pub struct BackendWindow {
    /// Stable window ID assigned by [`WindowManager`].
    pub id: u64,
    /// Window title (updated by the backend on every `set_title`).
    pub title: String,
    /// Top-left position in compositor logical pixels.
    pub position: (i32, i32),
    /// Width and height in compositor logical pixels.
    pub size: (u32, u32),
}

impl BackendWindow {
    /// Create a new `BackendWindow` with default 800×600 geometry.
    pub fn new(id: u64, title: String) -> Self {
        Self {
            id,
            title,
            position: (0, 0),
            size: (800, 600),
        }
    }
    /// Update the window's top-left position.
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.position = (x, y);
    }
    /// Update the window's size in pixels.
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }
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
    /// Build a [`Rectangle`] from a `(x, y)` top-left coordinate tuple and a
    /// `(width, height)` size tuple.
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

/// Per-window properties that the compositor reads when applying layout or
/// effects (floating vs tiled, fullscreen / maximized, opacity for fade
/// animations, border radius for decorations).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowProperties {
    /// Whether the window is floating (vs tiled)
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

/// Central store of all managed windows. Owns every [`AxiomWindow`] keyed by
/// stable monotonic IDs and tracks which window currently has keyboard
/// focus. Locked behind an `Arc<RwLock<…>>` in [`crate::compositor::AxiomCompositor`].
#[derive(Debug)]
pub struct WindowManager {
    /// Window tracking
    windows: HashMap<u64, AxiomWindow>,

    /// Next window ID
    next_window_id: u64,

    /// Currently focused window
    focused_window: Option<u64>,
}

impl WindowManager {
    /// Create an empty `WindowManager`. The `_config` argument is retained
    /// for future config-driven defaults.
    pub fn new(_config: &WindowConfig) -> Self {
        Self {
            windows: HashMap::new(),
            next_window_id: 1,
            focused_window: None,
        }
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

    /// Remove a window from management.
    ///
    /// If the removed window was focused, focus moves to the next available
    /// window (any remaining ID), so the compositor never loses track of the
    /// active window. Returns `None` when the window doesn't exist.
    pub fn remove_window(&mut self, id: u64) -> Option<AxiomWindow> {
        if self.focused_window == Some(id) {
            // Re-focus a sibling before clearing focus.
            self.focused_window = self.windows.keys().find(|&&k| k != id).copied();
        }
        self.windows.remove(&id)
    }

    /// Iterate over all managed windows via a closure (avoids per-frame allocation).
    pub fn for_each_window<F: FnMut(u64, &AxiomWindow)>(&self, mut f: F) {
        for (&id, window) in &self.windows {
            f(id, window);
        }
    }

    /// Borrow a window by ID. Returns `None` if no window with that ID exists.
    pub fn get_window(&self, id: u64) -> Option<&AxiomWindow> {
        self.windows.get(&id)
    }

    /// Get a mutable handle on a window by ID. Returns `None` if no window with that ID exists.
    pub fn get_window_mut(&mut self, id: u64) -> Option<&mut AxiomWindow> {
        self.windows.get_mut(&id)
    }

    /// Focus a window
    pub fn focus_window(&mut self, id: u64) {
        if self.windows.contains_key(&id) {
            self.focused_window = Some(id);
        }
    }

    /// Get the currently focused window id
    pub fn focused_window_id(&self) -> Option<u64> {
        self.focused_window
    }

    /// Toggle fullscreen for a window
    pub fn toggle_fullscreen(&mut self, id: u64) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.properties.fullscreen = !window.properties.fullscreen;
        }
    }

    /// Drop every managed window. The `WindowManager` itself stays usable;
    /// subsequent calls to [`add_window`](Self::add_window) start mapping
    /// from ID 1 again.
    pub fn shutdown(&mut self) {
        self.windows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_manager_initialization() {
        let wm = WindowManager::new(&WindowConfig::default());
        assert_eq!(wm.focused_window_id(), None);
    }

    #[test]
    fn test_add_window() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("test".into());
        assert_eq!(id, 1);
        // First window should be auto-focused
        assert_eq!(wm.focused_window_id(), Some(1));
        assert!(wm.get_window(1).is_some());
    }

    #[test]
    fn test_add_multiple_windows() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id1 = wm.add_window("first".into());
        let id2 = wm.add_window("second".into());
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        // Focus should stay on first window
        assert_eq!(wm.focused_window_id(), Some(1));
    }

    #[test]
    fn test_remove_window() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("test".into());
        assert!(wm.get_window(id).is_some());
        let removed = wm.remove_window(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().window.id, id);
        assert!(wm.get_window(id).is_none());
    }

    #[test]
    fn test_remove_focused_window_clears_focus() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("test".into());
        assert_eq!(wm.focused_window_id(), Some(id));
        wm.remove_window(id);
        assert_eq!(wm.focused_window_id(), None);
    }

    #[test]
    fn test_focus_window() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let _id1 = wm.add_window("first".into());
        let id2 = wm.add_window("second".into());
        assert_eq!(wm.focused_window_id(), Some(1));
        wm.focus_window(id2);
        assert_eq!(wm.focused_window_id(), Some(2));
    }

    #[test]
    fn test_focus_nonexistent_window() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        wm.add_window("test".into());
        wm.focus_window(999);
        // Focus should not change
        assert_eq!(wm.focused_window_id(), Some(1));
    }

    #[test]
    fn test_toggle_fullscreen() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("test".into());
        let win = wm.get_window(id).unwrap();
        assert!(!win.properties.fullscreen);
        wm.toggle_fullscreen(id);
        let win = wm.get_window(id).unwrap();
        assert!(win.properties.fullscreen);
    }

    #[test]
    fn test_shutdown_clears_windows() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        wm.add_window("test".into());
        wm.shutdown();
    }
}
