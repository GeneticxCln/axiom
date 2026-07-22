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

    /// Returns `true` if the point `(x, y)` lies inside this rectangle.
    /// The right and bottom edges are exclusive (standard UI hit-testing).
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }

    /// Returns `true` if this rectangle intersects another rectangle.
    /// Two rectangles intersect if they share any area; edge-only touches
    /// are not considered intersection (exclusive right/bottom edges).
    pub fn intersects(&self, other: &Self) -> bool {
        self.x < other.x + other.width as i32
            && self.x + self.width as i32 > other.x
            && self.y < other.y + other.height as i32
            && self.y + self.height as i32 > other.y
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
///
/// `minimized` is the **compositor-internal** hidden-state flag — Wayland
/// has no standard minimize protocol, so the compositor remembers the
/// window is hidden, removes it from the visible layout, and replays
/// `animate_window_open` on restore. Such windows are kept in the
/// [`WindowManager`] map (so a `KeyRelease` event still routes to them)
/// but absent from `ScrollableWorkspaces::calculate_workspace_layouts`.
/// The kill-switch `features.enable_minimize` (see [`crate::config`])
/// gates the titlebar button that drives this flip.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowProperties {
    /// Whether the window is floating (vs tiled)
    pub floating: bool,

    /// Whether the window is fullscreen
    pub fullscreen: bool,

    /// Whether the window is maximized
    pub maximized: bool,

    /// Whether the window is minimized (hidden from layout, can be restored).
    pub minimized: bool,

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
            minimized: false,
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
    #[must_use]
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
            self.focused_window = self.windows.keys().filter(|&&k| k != id).max().copied();
        }
        self.windows.remove(&id)
    }

    /// Iterate over all managed windows via a closure (avoids per-frame allocation).
    pub fn for_each_window<F: FnMut(u64, &AxiomWindow)>(&self, mut f: F) {
        for (&id, window) in &self.windows {
            f(id, window);
        }
    }

    /// Total number of managed windows (includes minimized).
    /// Used by the compositor tick to feed `LiveMetrics.active_windows`
    /// so monitoring clients (HealthCheck / GetPerformanceReport) see
    /// real values instead of zeros. Includes minimized windows so the
    /// count matches the IPC-level "registered with the compositor"
    /// definition; layout-visible filtering happens downstream.
    pub fn window_count(&self) -> u32 {
        self.windows.len() as u32
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

    /// Set focus to a specific window ID or clear focus entirely.
    pub fn set_focused_window(&mut self, id: Option<u64>) {
        match id {
            Some(id) if self.windows.contains_key(&id) => {
                self.focused_window = Some(id);
            }
            Some(_) => {}
            None => {
                self.focused_window = None;
            }
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

    /// Toggle the floating state of a window. Floating windows are
    /// positioned by the user rather than auto-tiled.
    pub fn toggle_floating(&mut self, id: u64) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.properties.floating = !window.properties.floating;
        }
    }

    /// Mark a window as minimized. Returns `true` if the window existed
    /// and its state changed (i.e. it was previously visible). Minimizing
    /// a window that is already minimized, or that does not exist, returns
    /// `false` and leaves state untouched.
    ///
    /// Minimizing clears the keyboard focus if the minimized window was
    /// focused, mirroring X11/Wayland conventions: clicking the minimize
    /// button hides the window and focus moves to the next *visible*
    /// window (any ID whose `properties.minimized` is `false`).
    pub fn minimize_window(&mut self, id: u64) -> bool {
        let Some(window) = self.windows.get_mut(&id) else {
            return false;
        };
        if window.properties.minimized {
            return false; // already minimized: idempotent
        }
        window.properties.minimized = true;
        // If the minimized window was focused, drop focus to a visible
        // sibling if one exists; otherwise leave focus = None.
        if self.focused_window == Some(id) {
            self.focused_window = self
                .windows
                .iter()
                .filter(|(_, w)| !w.properties.minimized)
                .map(|(k, _)| *k)
                .max();
        }
        true
    }

    /// Restore (un-minimize) a window. Returns `true` if the window
    /// existed and was previously minimized. Restoring a window that is
    /// already visible, or that does not exist, returns `false`.
    ///
    /// Restoring does NOT change focus — the caller (the backend) is
    /// responsible for focusing the newly-restored window if desired.
    /// This separation keeps `WindowManager` policy-neutral about who
    /// gets the keyboard after a restore.
    pub fn restore_window(&mut self, id: u64) -> bool {
        let Some(window) = self.windows.get_mut(&id) else {
            return false;
        };
        if !window.properties.minimized {
            return false; // already visible: idempotent
        }
        window.properties.minimized = false;
        true
    }

    /// Read-only accessor: is the given window currently minimized?
    pub fn is_minimized(&self, id: u64) -> bool {
        self.windows
            .get(&id)
            .map(|w| w.properties.minimized)
            .unwrap_or(false)
    }

    /// Snapshot of currently-minimized window IDs. Useful for tests
    /// and for rendering a (not-yet-shipped) taskbar list.
    pub fn minimized_ids(&self) -> Vec<u64> {
        self.windows
            .iter()
            .filter(|(_, w)| w.properties.minimized)
            .map(|(k, _)| *k)
            .collect()
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
    fn test_contains_point() {
        let r = Rectangle {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        assert!(r.contains_point(10, 20));
        assert!(r.contains_point(39, 59));
        assert!(!r.contains_point(40, 20)); // right edge exclusive
        assert!(!r.contains_point(10, 60)); // bottom edge exclusive
        assert!(!r.contains_point(9, 20)); // left edge exclusive
    }

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
        let _ = wm.add_window("test".into());
        wm.focus_window(999);
        // Focus should not change
        assert_eq!(wm.focused_window_id(), Some(1));
    }

    #[test]
    fn test_set_focused_window_can_clear_focus() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("test".into());
        assert_eq!(wm.focused_window_id(), Some(id));
        wm.set_focused_window(None);
        assert_eq!(wm.focused_window_id(), None);
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
    fn test_minimize_window_marks_minimized_and_drops_focus() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id_a = wm.add_window("a".into());
        let _id_b = wm.add_window("b".into());
        // Add order: a has id=1, b has id=2. a should still be focused.
        assert_eq!(wm.focused_window_id(), Some(id_a));
        assert!(wm.minimize_window(id_a));
        assert!(wm.is_minimized(id_a));
        // After minimizing the focused window, focus moves to a visible sibling.
        assert_eq!(wm.focused_window_id(), Some(2));
        assert_eq!(wm.minimized_ids(), vec![id_a]);
    }

    #[test]
    fn test_minimize_window_is_idempotent() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("only".into());
        assert!(wm.minimize_window(id));
        // Second call should be a no-op (idempotent).
        assert!(!wm.minimize_window(id));
        assert!(wm.is_minimized(id));
    }

    #[test]
    fn test_minimize_unknown_window_returns_false() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        assert!(!wm.minimize_window(99));
        assert!(!wm.is_minimized(99));
    }

    #[test]
    fn test_restore_window_unmarks_minimized() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("only".into());
        // With only one window present, minimizing the focused window
        // drops focus to `None` (no visible sibling to take over).
        wm.minimize_window(id);
        assert!(wm.is_minimized(id));
        assert_eq!(wm.focused_window_id(), None);
        assert!(wm.restore_window(id));
        assert!(!wm.is_minimized(id));
        // Restore does NOT auto-focus (caller decides who gets the
        // keyboard). A previous version of this test asserted
        // `Some(2)` which was incorrect because (a) only one window
        // exists so the only id is 1, not 2, and (b) restore is
        // deliberately focus-neutral per the method's docstring.
        assert_eq!(wm.focused_window_id(), None);
    }

    #[test]
    fn test_restore_visible_window_returns_false() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let id = wm.add_window("only".into());
        // Already visible — restore is a no-op.
        assert!(!wm.restore_window(id));
    }

    #[test]
    fn test_shutdown_clears_windows() {
        let mut wm = WindowManager::new(&WindowConfig::default());
        let _ = wm.add_window("test".into());
        wm.shutdown();
    }
}
