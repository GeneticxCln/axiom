//! Window Z-ordering stack management
//!
//! This module provides the `WindowStack` data structure for managing the Z-order
//! (stacking order) of windows in the compositor. Windows are ordered from bottom
//! to top, with the last window in the stack being the top-most visible window.

use std::collections::HashMap;

/// Manages the Z-ordering of windows in the compositor.
///
/// Windows are stored in bottom-to-top order, where index 0 represents the
/// bottom-most window and the last index represents the top-most window.
///
/// # Examples
///
/// ```
/// use axiom::renderer::window_stack::WindowStack;
///
/// let mut stack = WindowStack::new();
/// stack.push(1);
/// stack.push(2);
///
/// assert_eq!(stack.top(), Some(2));
/// ```
#[derive(Debug, Clone)]
pub struct WindowStack {
    /// Windows ordered from bottom to top
    windows: Vec<u64>,

    /// Fast lookup: window ID → position in stack
    positions: HashMap<u64, usize>,
}

impl WindowStack {
    /// Creates a new empty window stack.
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            positions: HashMap::new(),
        }
    }

    /// Creates a window stack with the specified initial capacity.
    ///
    /// This is useful if you know approximately how many windows will be managed.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            windows: Vec::with_capacity(capacity),
            positions: HashMap::with_capacity(capacity),
        }
    }

    /// Adds a window to the top of the stack.
    ///
    /// If the window is already in the stack, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `window_id` - The ID of the window to add
    ///
    /// # Returns
    ///
    /// `true` if the window was added, `false` if it was already present
    pub fn push(&mut self, window_id: u64) -> bool {
        if self.positions.contains_key(&window_id) {
            return false;
        }

        let position = self.windows.len();
        self.windows.push(window_id);
        self.positions.insert(window_id, position);
        true
    }

    /// Removes a window from the stack.
    ///
    /// # Arguments
    ///
    /// * `window_id` - The ID of the window to remove
    ///
    /// # Returns
    ///
    /// `Some(position)` with the window's previous position if found, `None` otherwise
    pub fn remove(&mut self, window_id: u64) -> Option<usize> {
        let pos = self.positions.remove(&window_id)?;
        self.windows.remove(pos);
        self.rebuild_positions();
        Some(pos)
    }

    /// Raises a window to the top of the stack.
    ///
    /// If the window is not in the stack, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `window_id` - The ID of the window to raise
    ///
    /// # Returns
    ///
    /// `true` if the window was raised, `false` if it wasn't in the stack
    pub fn raise_to_top(&mut self, window_id: u64) -> bool {
        if self.remove(window_id).is_some() {
            self.push(window_id);
            true
        } else {
            false
        }
    }

    /// Lowers a window to the bottom of the stack.
    ///
    /// If the window is not in the stack, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `window_id` - The ID of the window to lower
    ///
    /// # Returns
    ///
    /// `true` if the window was lowered, `false` if it wasn't in the stack
    pub fn lower_to_bottom(&mut self, window_id: u64) -> bool {
        if self.remove(window_id).is_some() {
            self.windows.insert(0, window_id);
            self.rebuild_positions();
            true
        } else {
            false
        }
    }

    /// Raises a window above another window in the stack.
    ///
    /// The window will be placed immediately above the target window.
    /// If either window is not in the stack, this is a no-op.
    ///
    /// # Arguments
    ///
    /// * `window_id` - The window to raise
    /// * `above` - The window to place it above
    ///
    /// # Returns
    ///
    /// `true` if successful, `false` if either window wasn't in the stack
    pub fn raise_above(&mut self, window_id: u64, above: u64) -> bool {
        // Check that target window exists
        if !self.positions.contains_key(&above) {
            return false;
        }

        // Remove window from current position
        if self.remove(window_id).is_none() {
            return false;
        }

        // Insert at position above target (which may have shifted after removal)
        let new_pos = self.positions.get(&above).map(|&p| p + 1).unwrap_or(0);
        self.windows.insert(new_pos, window_id);
        self.rebuild_positions();
        true
    }

    /// Returns the windows in bottom-to-top rendering order.
    ///
    /// This is the order in which windows should be rendered, with the first
    /// window being drawn first (bottom) and the last window being drawn last (top).
    pub fn render_order(&self) -> &[u64] {
        &self.windows
    }

    /// Returns an iterator over windows in bottom-to-top order.
    pub fn iter(&self) -> impl Iterator<Item = &u64> {
        self.windows.iter()
    }

    /// Returns the top-most window in the stack.
    ///
    /// This is the window that should receive input events.
    pub fn top(&self) -> Option<u64> {
        self.windows.last().copied()
    }

    /// Returns the bottom-most window in the stack.
    pub fn bottom(&self) -> Option<u64> {
        self.windows.first().copied()
    }

    /// Returns the number of windows in the stack.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Returns `true` if the stack contains no windows.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Checks if a window is in the stack.
    pub fn contains(&self, window_id: u64) -> bool {
        self.positions.contains_key(&window_id)
    }

    /// Returns the position of a window in the stack.
    ///
    /// Position 0 is the bottom-most window, and `len() - 1` is the top-most.
    pub fn position(&self, window_id: u64) -> Option<usize> {
        self.positions.get(&window_id).copied()
    }

    /// Returns all windows above a given window in the stack.
    ///
    /// Useful for occlusion detection and rendering optimizations.
    pub fn windows_above(&self, window_id: u64) -> &[u64] {
        match self.positions.get(&window_id) {
            Some(&pos) => &self.windows[pos + 1..],
            None => &[],
        }
    }

    /// Returns all windows below a given window in the stack.
    pub fn windows_below(&self, window_id: u64) -> &[u64] {
        match self.positions.get(&window_id) {
            Some(&pos) => &self.windows[..pos],
            None => &[],
        }
    }

    /// Clears all windows from the stack.
    pub fn clear(&mut self) {
        self.windows.clear();
        self.positions.clear();
    }

    /// Rebuilds the position lookup map.
    ///
    /// This is called internally after operations that change window positions.
    fn rebuild_positions(&mut self) {
        self.positions.clear();
        for (i, &window_id) in self.windows.iter().enumerate() {
            self.positions.insert(window_id, i);
        }
    }
}

impl Default for WindowStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_stack_is_empty() {
        let stack = WindowStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        assert_eq!(stack.top(), None);
        assert_eq!(stack.bottom(), None);
    }

    #[test]
    fn test_push_adds_to_top() {
        let mut stack = WindowStack::new();
        assert!(stack.push(1));
        assert!(stack.push(2));
        assert!(stack.push(3));

        assert_eq!(stack.len(), 3);
        assert_eq!(stack.top(), Some(3));
        assert_eq!(stack.bottom(), Some(1));
        assert_eq!(stack.render_order(), &[1, 2, 3]);
    }

    #[test]
    fn test_push_duplicate_is_noop() {
        let mut stack = WindowStack::new();
        assert!(stack.push(1));
        assert!(!stack.push(1)); // Should return false

        assert_eq!(stack.len(), 1);
        assert_eq!(stack.render_order(), &[1]);
    }

    #[test]
    fn test_remove_window() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.remove(2), Some(1));
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.render_order(), &[1, 3]);
        assert!(!stack.contains(2));
    }

    #[test]
    fn test_remove_nonexistent_window() {
        let mut stack = WindowStack::new();
        stack.push(1);

        assert_eq!(stack.remove(99), None);
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn test_raise_to_top() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert!(stack.raise_to_top(1));

        assert_eq!(stack.render_order(), &[2, 3, 1]);
        assert_eq!(stack.top(), Some(1));
        assert_eq!(stack.position(1), Some(2));
    }

    #[test]
    fn test_raise_to_top_nonexistent() {
        let mut stack = WindowStack::new();
        stack.push(1);

        assert!(!stack.raise_to_top(99));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn test_lower_to_bottom() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert!(stack.lower_to_bottom(3));

        assert_eq!(stack.render_order(), &[3, 1, 2]);
        assert_eq!(stack.bottom(), Some(3));
        assert_eq!(stack.position(3), Some(0));
    }

    #[test]
    fn test_raise_above() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);
        stack.push(4);

        assert!(stack.raise_above(1, 2));

        // Window 1 should now be above window 2
        assert_eq!(stack.render_order(), &[2, 1, 3, 4]);
    }

    #[test]
    fn test_contains() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);

        assert!(stack.contains(1));
        assert!(stack.contains(2));
        assert!(!stack.contains(3));
    }

    #[test]
    fn test_position() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.position(1), Some(0));
        assert_eq!(stack.position(2), Some(1));
        assert_eq!(stack.position(3), Some(2));
        assert_eq!(stack.position(99), None);
    }

    #[test]
    fn test_windows_above() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);
        stack.push(4);

        assert_eq!(stack.windows_above(2), &[3, 4]);
        assert_eq!(stack.windows_above(4), &[] as &[u64]);
        assert_eq!(stack.windows_above(99), &[] as &[u64]);
    }

    #[test]
    fn test_windows_below() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);
        stack.push(4);

        assert_eq!(stack.windows_below(3), &[1, 2]);
        assert_eq!(stack.windows_below(1), &[] as &[u64]);
        assert_eq!(stack.windows_below(99), &[] as &[u64]);
    }

    #[test]
    fn test_clear() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        stack.clear();

        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        assert_eq!(stack.top(), None);
    }

    #[test]
    fn test_iter() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        let collected: Vec<_> = stack.iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    #[test]
    fn test_multiple_operations() {
        let mut stack = WindowStack::new();

        // Add several windows
        stack.push(1);
        stack.push(2);
        stack.push(3);
        stack.push(4);
        stack.push(5);

        // Raise middle window to top
        stack.raise_to_top(3);
        assert_eq!(stack.top(), Some(3));

        // Remove another window
        stack.remove(2);
        assert_eq!(stack.len(), 4);

        // Lower top window to bottom
        stack.lower_to_bottom(3);
        assert_eq!(stack.bottom(), Some(3));
        assert_eq!(stack.top(), Some(5));

        // Final order should be: [3, 1, 4, 5]
        assert_eq!(stack.render_order(), &[3, 1, 4, 5]);
    }

    #[test]
    fn test_with_capacity() {
        let stack = WindowStack::with_capacity(10);
        assert!(stack.is_empty());
        assert!(stack.windows.capacity() >= 10);
    }

    #[test]
    fn test_position_consistency_after_operations() {
        let mut stack = WindowStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        // Remove middle window
        stack.remove(2);

        // Positions should be consistent
        assert_eq!(stack.position(1), Some(0));
        assert_eq!(stack.position(3), Some(1));
        assert_eq!(stack.position(2), None);

        // Verify render order matches positions
        for (i, &window_id) in stack.render_order().iter().enumerate() {
            assert_eq!(stack.position(window_id), Some(i));
        }
    }
}
