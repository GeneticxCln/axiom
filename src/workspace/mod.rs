//! Scrollable workspace management (niri-inspired)
//!
//! This module implements Axiom's core innovation: infinite scrollable
//! workspaces with smooth animations and intelligent window placement.

use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::config::WorkspaceConfig;
use crate::window::Rectangle;

/// Represents a workspace column in the scrollable view
#[derive(Debug, Clone)]
pub struct WorkspaceColumn {
    /// Column index (can be negative for infinite scroll)
    pub index: i32,

    /// X position of this column
    pub position: f64,

    /// Windows in this column
    pub windows: Vec<u64>, // Window IDs

    /// Whether this column is active/visible
    pub active: bool,

    /// Last time this column was accessed
    pub last_accessed: Instant,
}

impl WorkspaceColumn {
    pub fn new(index: i32, position: f64) -> Self {
        Self {
            index,
            position,
            windows: Vec::new(),
            active: false,
            last_accessed: Instant::now(),
        }
    }

    pub fn add_window(&mut self, window_id: u64) {
        if !self.windows.contains(&window_id) {
            self.windows.push(window_id);
            self.last_accessed = Instant::now();
        }
    }

    pub fn remove_window(&mut self, window_id: u64) -> bool {
        if let Some(pos) = self.windows.iter().position(|&id| id == window_id) {
            self.windows.remove(pos);
            self.last_accessed = Instant::now();
            true
        } else {
            false
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// Scroll animation state
#[derive(Debug, Clone, Copy)]
pub enum ScrollState {
    Idle,
    Scrolling {
        start_time: Instant,
        start_position: f64,
        target_position: f64,
        duration: Duration,
    },
    Momentum {
        start_time: Instant,
        start_position: f64,
        velocity: f64,
    },
}

/// Scrollable workspace manager
#[derive(Debug)]
pub struct ScrollableWorkspaces {
    config: WorkspaceConfig,

    /// Current scroll position (in workspace units)
    current_position: f64,

    /// Target scroll position for animations
    target_position: f64,

    /// Current scroll velocity
    scroll_velocity: f64,

    /// Map of column index to workspace column
    columns: HashMap<i32, WorkspaceColumn>,

    /// Currently focused column index
    focused_column: i32,

    /// Animation state
    scroll_state: ScrollState,

    /// Viewport bounds (what's currently visible)
    viewport_width: f64,
    viewport_height: f64,

    /// Animation easing parameters
    last_update: Instant,
}

impl ScrollableWorkspaces {
    pub fn new(config: &WorkspaceConfig) -> Result<Self> {
        let mut workspace_manager = Self {
            config: config.clone(),
            current_position: 0.0,
            target_position: 0.0,
            scroll_velocity: 0.0,
            columns: HashMap::new(),
            focused_column: 0,
            scroll_state: ScrollState::Idle,
            viewport_width: 1920.0,  // Default, will be updated
            viewport_height: 1080.0, // Default, will be updated
            last_update: Instant::now(),
        };

        // Create the initial workspace column
        workspace_manager.ensure_column(0);

        info!("🔄 Scrollable workspaces initialized with infinite scrolling");
        debug!(
            "📏 Workspace width: {} px, gaps: {} px",
            config.workspace_width, config.gaps
        );

        Ok(workspace_manager)
    }

    /// Update the viewport size (called when window/display size changes)
    pub fn set_viewport_size(&mut self, width: f64, height: f64) {
        self.viewport_width = width;
        self.viewport_height = height;
        debug!("📐 Viewport size updated to {}x{}", width, height);
    }

    /// Ensure a column exists at the given index
    pub fn ensure_column(&mut self, index: i32) -> &mut WorkspaceColumn {
        if !self.columns.contains_key(&index) {
            let position = index as f64 * self.config.workspace_width as f64;
            let column = WorkspaceColumn::new(index, position);
            debug!(
                "📄 Created new workspace column {} at position {}",
                index, position
            );
            self.columns.insert(index, column);
        }
        self.columns.get_mut(&index).unwrap()
    }

    /// Get the current focused column
    pub fn get_focused_column(&self) -> Option<&WorkspaceColumn> {
        self.columns.get(&self.focused_column)
    }

    /// Get the current focused column mutably
    pub fn get_focused_column_mut(&mut self) -> &mut WorkspaceColumn {
        self.ensure_column(self.focused_column)
    }

    /// Scroll to a specific column (animated)
    pub fn scroll_to_column(&mut self, column_index: i32) {
        self.ensure_column(column_index);

        let target_pos = column_index as f64 * self.config.workspace_width as f64;
        let current_time = Instant::now();

        // Calculate animation duration based on distance
        let distance = (target_pos - self.current_position).abs();
        let base_duration = Duration::from_millis(250); // Base animation duration
        let duration = Duration::from_millis(
            (base_duration.as_millis() as f64 * (1.0 + distance / 2000.0)).min(800.0) as u64,
        );

        self.scroll_state = ScrollState::Scrolling {
            start_time: current_time,
            start_position: self.current_position,
            target_position: target_pos,
            duration,
        };

        self.focused_column = column_index;
        self.target_position = target_pos;

        info!(
            "📱 Scrolling to column {} (position: {:.1})",
            column_index, target_pos
        );
    }

    /// Scroll left by one workspace
    pub fn scroll_left(&mut self) {
        let new_column = self.focused_column - 1;
        self.scroll_to_column(new_column);
    }

    /// Scroll right by one workspace
    pub fn scroll_right(&mut self) {
        let new_column = self.focused_column + 1;
        self.scroll_to_column(new_column);
    }

    /// Start momentum scrolling (for gesture input)
    pub fn start_momentum_scroll(&mut self, velocity: f64) {
        if velocity.abs() > 10.0 {
            // Minimum velocity threshold
            self.scroll_state = ScrollState::Momentum {
                start_time: Instant::now(),
                start_position: self.current_position,
                velocity: velocity * self.config.scroll_speed,
            };
            debug!("🏃 Started momentum scroll with velocity: {:.1}", velocity);
        }
    }

    /// Add a window to a specific column
    pub fn add_window_to_column(&mut self, window_id: u64, column_index: i32) {
        let column = self.ensure_column(column_index);
        column.add_window(window_id);
        debug!("🪟 Added window {} to column {}", window_id, column_index);
    }

    /// Add a window to the current focused column
    pub fn add_window(&mut self, window_id: u64) {
        let focused_column = self.focused_column;
        self.add_window_to_column(window_id, focused_column);
    }

    /// Remove a window from all columns (public interface)
    pub fn remove_window(&mut self, window_id: u64) -> Option<i32> {
        self.remove_window_internal(window_id)
    }

    /// Remove a window from all columns
    fn remove_window_internal(&mut self, window_id: u64) -> Option<i32> {
        for (column_index, column) in self.columns.iter_mut() {
            if column.remove_window(window_id) {
                debug!(
                    "🗑️ Removed window {} from column {}",
                    window_id, column_index
                );
                return Some(*column_index);
            }
        }
        None
    }

    /// Move a window to a different column
    pub fn move_window_to_column(&mut self, window_id: u64, target_column: i32) -> bool {
        // First remove from current column
        if let Some(_current_column) = self.remove_window_internal(window_id) {
            // Then add to target column
            self.add_window_to_column(window_id, target_column);
            info!("🔀 Moved window {} to column {}", window_id, target_column);
            return true;
        }
        false
    }

    /// Move the focused window to the left column
    pub fn move_window_left(&mut self, window_id: u64) -> bool {
        let target_column = self.focused_column - 1;
        self.move_window_to_column(window_id, target_column)
    }

    /// Move the focused window to the right column
    pub fn move_window_right(&mut self, window_id: u64) -> bool {
        let target_column = self.focused_column + 1;
        self.move_window_to_column(window_id, target_column)
    }

    /// Get all windows in the currently focused column
    pub fn get_focused_column_windows(&self) -> Vec<u64> {
        self.get_focused_column()
            .map(|column| column.windows.clone())
            .unwrap_or_default()
    }

    /// Get the currently focused column (optional version)
    pub fn get_focused_column_opt(&self) -> Option<&WorkspaceColumn> {
        self.get_focused_column()
    }

    /// Get all visible columns based on current viewport
    pub fn get_visible_columns(&self) -> Vec<&WorkspaceColumn> {
        let left_bound = self.current_position - self.viewport_width / 2.0;
        let right_bound = self.current_position + self.viewport_width / 2.0;

        self.columns
            .values()
            .filter(|column| {
                column.position >= left_bound - self.config.workspace_width as f64
                    && column.position <= right_bound + self.config.workspace_width as f64
            })
            .collect()
    }

    /// Calculate the layout for windows in the visible columns
    pub fn calculate_workspace_layouts(&self) -> HashMap<u64, Rectangle> {
        let mut layouts = HashMap::new();
        let visible_columns = self.get_visible_columns();

        for column in visible_columns {
            // Calculate column bounds relative to viewport
            let column_offset = column.position - self.current_position;
            let column_left = (self.viewport_width / 2.0) + column_offset;

            // Only layout windows for columns that are actually visible
            if column_left + self.config.workspace_width as f64 >= 0.0
                && column_left <= self.viewport_width
            {
                let column_bounds = Rectangle {
                    x: column_left as i32,
                    y: 0,
                    width: self.config.workspace_width,
                    height: self.viewport_height as u32,
                };

                // Calculate layout for windows in this column
                if !column.windows.is_empty() {
                    let gap = self.config.gaps as i32;
                    let window_height = (column_bounds.height as i32
                        - (gap * (column.windows.len() as i32 + 1)))
                        / column.windows.len() as i32;

                    for (i, &window_id) in column.windows.iter().enumerate() {
                        let y = gap + i as i32 * (window_height + gap);
                        let window_rect = Rectangle {
                            x: column_bounds.x + gap,
                            y,
                            width: column_bounds.width - 2 * gap as u32,
                            height: window_height as u32,
                        };
                        layouts.insert(window_id, window_rect);
                    }
                }
            }
        }

        layouts
    }

    /// Update animations and smooth scrolling
    pub fn update_animations(&mut self) -> Result<()> {
        let now = Instant::now();
        let _delta_time = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;

        match self.scroll_state {
            ScrollState::Scrolling {
                start_time,
                start_position,
                target_position,
                duration,
            } => {
                let elapsed = now.duration_since(start_time);

                if elapsed >= duration {
                    // Animation complete
                    self.current_position = target_position;
                    self.scroll_velocity = 0.0;
                    self.scroll_state = ScrollState::Idle;
                    debug!(
                        "✅ Scroll animation completed at position {:.1}",
                        target_position
                    );
                } else {
                    // Calculate eased position
                    let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
                    let eased_progress = self.ease_out_cubic(progress);

                    self.current_position =
                        start_position + (target_position - start_position) * eased_progress;

                    // Calculate velocity for smooth transitions
                    self.scroll_velocity = (target_position - start_position)
                        * self.ease_out_cubic_derivative(progress)
                        / duration.as_secs_f64();
                }
            }

            ScrollState::Momentum {
                start_time,
                start_position,
                velocity,
            } => {
                let elapsed = now.duration_since(start_time).as_secs_f64();
                let friction: f64 = 0.95; // Friction coefficient

                // Apply friction to velocity
                let current_velocity = velocity * friction.powf(elapsed * 60.0);

                if current_velocity.abs() < 1.0 {
                    // Momentum has died down, snap to nearest column
                    let nearest_column =
                        (self.current_position / self.config.workspace_width as f64).round() as i32;
                    self.scroll_to_column(nearest_column);
                } else {
                    // Update position based on momentum
                    self.current_position = start_position + velocity * elapsed;
                    self.scroll_velocity = current_velocity;
                }
            }

            ScrollState::Idle => {
                // Gradually reduce any remaining velocity
                self.scroll_velocity *= 0.9;
                if self.scroll_velocity.abs() < 0.1 {
                    self.scroll_velocity = 0.0;
                }
            }
        }

        // Cleanup empty columns that haven't been accessed in a while
        if now.duration_since(self.last_update) > Duration::from_secs(1) {
            self.cleanup_empty_columns();
        }

        Ok(())
    }

    /// Ease-out cubic function for smooth animations
    fn ease_out_cubic(&self, t: f64) -> f64 {
        let t = t - 1.0;
        t * t * t + 1.0
    }

    /// Derivative of ease-out cubic for velocity calculation
    fn ease_out_cubic_derivative(&self, t: f64) -> f64 {
        let t = t - 1.0;
        3.0 * t * t
    }

    /// Clean up empty columns that haven't been used recently
    fn cleanup_empty_columns(&mut self) {
        let now = Instant::now();
        let cleanup_threshold = Duration::from_secs(30); // Keep empty columns for 30 seconds

        let columns_to_remove: Vec<i32> = self
            .columns
            .iter()
            .filter(|(index, column)| {
                **index != self.focused_column && // Never remove focused column
                column.is_empty() &&
                now.duration_since(column.last_accessed) > cleanup_threshold
            })
            .map(|(index, _)| *index)
            .collect();

        for index in columns_to_remove {
            self.columns.remove(&index);
            debug!("🧹 Cleaned up empty workspace column {}", index);
        }
    }

    /// Get current scroll position
    pub fn current_position(&self) -> f64 {
        self.current_position
    }

    /// Get current focused column index
    pub fn focused_column_index(&self) -> i32 {
        self.focused_column
    }

    /// Get total number of active columns
    pub fn active_column_count(&self) -> usize {
        self.columns.len()
    }

    /// Check if currently scrolling
    pub fn is_scrolling(&self) -> bool {
        !matches!(self.scroll_state, ScrollState::Idle)
    }

    /// Get scroll progress for animations (0.0 to 1.0)
    pub fn scroll_progress(&self) -> f64 {
        match self.scroll_state {
            ScrollState::Scrolling {
                start_time,
                duration,
                ..
            } => {
                let elapsed = Instant::now().duration_since(start_time);
                (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    pub fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down scrollable workspaces...");
        self.columns.clear();
        self.scroll_state = ScrollState::Idle;
        debug!("✅ Workspace cleanup complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
