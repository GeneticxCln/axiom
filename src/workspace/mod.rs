//! Scrollable workspace management (niri-inspired)
//!
//! This module implements Axiom's core innovation: infinite scrollable
//! workspaces with smooth animations and intelligent window placement.

use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::sync::{Mutex, OnceLock};

use crate::config::WorkspaceConfig;
use crate::window::Rectangle;

// Global snapshot of workspace scroll speed for metrics
static GLOBAL_SCROLL_SPEED: OnceLock<Mutex<f64>> = OnceLock::new();

/// Update global scroll speed snapshot
pub fn set_global_scroll_speed(speed: f64) {
    let cell = GLOBAL_SCROLL_SPEED.get_or_init(|| Mutex::new(1.0));
    match cell.lock() {
        Ok(mut guard) => { *guard = speed.clamp(0.01, 10.0); }
        Err(poisoned) => { *poisoned.into_inner() = speed.clamp(0.01, 10.0); }
    }
}

/// Read global scroll speed snapshot
pub fn get_global_scroll_speed() -> f64 {
    let cell = GLOBAL_SCROLL_SPEED.get_or_init(|| Mutex::new(1.0));
    match cell.lock() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

/// Layout mode for arranging windows within a column
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// Stack windows vertically (default)
    Vertical,
    /// Stack windows horizontally
    Horizontal,
    /// Master window on left, stack on right
    MasterStack,
    /// Grid layout (auto-arranges in rows/columns)
    Grid,
    /// Spiral layout (fibonacci-style)
    Spiral,
}

impl Default for LayoutMode {
    fn default() -> Self {
        LayoutMode::Vertical
    }
}

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

    /// Layout mode for this column
    pub layout_mode: LayoutMode,

    /// Window split ratios (for resizing within tiled layouts)
    /// Maps window index to its size ratio (0.0 to 1.0)
    pub split_ratios: HashMap<usize, f64>,

    /// Currently focused window index within this column
    pub focused_window_index: Option<usize>,
}

impl WorkspaceColumn {
    pub fn new(index: i32, position: f64) -> Self {
        Self {
            index,
            position,
            windows: Vec::new(),
            active: false,
            last_accessed: Instant::now(),
            layout_mode: LayoutMode::default(),
            split_ratios: HashMap::new(),
            focused_window_index: None,
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

    /// Reserved insets for layer-shell exclusive zones (pixels)
    reserved_top: f64,
    reserved_right: f64,
    reserved_bottom: f64,
    reserved_left: f64,

    /// Animation easing parameters
    last_update: Instant,
}

impl ScrollableWorkspaces {
    /// Backwards-compatible helper used by older tests: remove_window_* returning bool
    pub fn remove_window_bool(&mut self, window_id: u64) -> bool {
        self.remove_window(window_id).is_some()
    }

    /// Update workspace scroll speed at runtime (validated/clamped)
    pub fn set_scroll_speed(&mut self, speed: f64) {
        let clamped = speed.clamp(0.01, 10.0);
        self.config.scroll_speed = clamped;
        set_global_scroll_speed(clamped);
        info!("⚙️ Updated workspace scroll_speed to {:.2}", clamped);
    }

    /// Get the current workspace scroll speed
    pub fn scroll_speed(&self) -> f64 {
        self.config.scroll_speed
    }

    /// Check if a window exists in any column
    pub fn window_exists(&self, window_id: u64) -> bool {
        self.columns
            .values()
            .any(|c| c.windows.contains(&window_id))
    }

    /// Whether infinite scroll is enabled
    pub fn is_infinite_scroll_enabled(&self) -> bool {
        self.config.infinite_scroll
    }
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
            reserved_top: 0.0,
            reserved_right: 0.0,
            reserved_bottom: 0.0,
            reserved_left: 0.0,
        };

        // Sync global scroll speed snapshot
        set_global_scroll_speed(workspace_manager.config.scroll_speed);
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

    /// Update reserved insets (top, right, bottom, left)
    pub fn set_reserved_insets(&mut self, top: f64, right: f64, bottom: f64, left: f64) {
        self.reserved_top = top.max(0.0);
        self.reserved_right = right.max(0.0);
        self.reserved_bottom = bottom.max(0.0);
        self.reserved_left = left.max(0.0);
        debug!(
            "📐 Reserved insets set: top {:.1}, right {:.1}, bottom {:.1}, left {:.1}",
            self.reserved_top, self.reserved_right, self.reserved_bottom, self.reserved_left
        );
    }

    /// Increase reserved insets to at least the provided values (component-wise max)
    pub fn update_reserved_insets_max(&mut self, top: f64, right: f64, bottom: f64, left: f64) {
        let nt = self.reserved_top.max(top.max(0.0));
        let nr = self.reserved_right.max(right.max(0.0));
        let nb = self.reserved_bottom.max(bottom.max(0.0));
        let nl = self.reserved_left.max(left.max(0.0));
        self.set_reserved_insets(nt, nr, nb, nl);
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
                // Apply reserved insets from layer-shell exclusive zones
                let usable_height =
                    (self.viewport_height - self.reserved_top - self.reserved_bottom).max(1.0);
                let usable_width =
                    (self.config.workspace_width as f64 - self.reserved_left - self.reserved_right)
                        .max(1.0);
                let column_bounds = Rectangle {
                    x: (column_left + self.reserved_left) as i32,
                    y: self.reserved_top as i32,
                    width: usable_width as u32,
                    height: usable_height as u32,
                };

                // Calculate layout based on the column's layout mode
                if !column.windows.is_empty() {
                    let window_layouts = self.calculate_column_layout(
                        column,
                        &column_bounds,
                        self.config.gaps as i32,
                    );
                    layouts.extend(window_layouts);
                }
            }
        }

        layouts
    }

    /// Calculate layout for windows within a single column based on its layout mode
    fn calculate_column_layout(
        &self,
        column: &WorkspaceColumn,
        bounds: &Rectangle,
        gap: i32,
    ) -> HashMap<u64, Rectangle> {
        match column.layout_mode {
            LayoutMode::Vertical => self.layout_vertical(column, bounds, gap),
            LayoutMode::Horizontal => self.layout_horizontal(column, bounds, gap),
            LayoutMode::MasterStack => self.layout_master_stack(column, bounds, gap),
            LayoutMode::Grid => self.layout_grid(column, bounds, gap),
            LayoutMode::Spiral => self.layout_spiral(column, bounds, gap),
        }
    }

    /// Vertical stacking layout (default)
    fn layout_vertical(
        &self,
        column: &WorkspaceColumn,
        bounds: &Rectangle,
        gap: i32,
    ) -> HashMap<u64, Rectangle> {
        let mut layouts = HashMap::new();
        let window_count = column.windows.len();
        let total_gap_height = gap * (window_count as i32 + 1);
        let available_height = (bounds.height as i32 - total_gap_height).max(1);
        let window_height = available_height / window_count as i32;

        for (i, &window_id) in column.windows.iter().enumerate() {
            let y = bounds.y + gap + i as i32 * (window_height + gap);
            let window_rect = Rectangle {
                x: bounds.x + gap,
                y,
                width: (bounds.width as i32 - 2 * gap).max(1) as u32,
                height: window_height.max(1) as u32,
            };
            layouts.insert(window_id, window_rect);
        }

        layouts
    }

    /// Horizontal stacking layout
    fn layout_horizontal(
        &self,
        column: &WorkspaceColumn,
        bounds: &Rectangle,
        gap: i32,
    ) -> HashMap<u64, Rectangle> {
        let mut layouts = HashMap::new();
        let window_count = column.windows.len();
        let total_gap_width = gap * (window_count as i32 + 1);
        let available_width = (bounds.width as i32 - total_gap_width).max(1);
        let window_width = available_width / window_count as i32;

        for (i, &window_id) in column.windows.iter().enumerate() {
            let x = bounds.x + gap + i as i32 * (window_width + gap);
            let window_rect = Rectangle {
                x,
                y: bounds.y + gap,
                width: window_width.max(1) as u32,
                height: (bounds.height as i32 - 2 * gap).max(1) as u32,
            };
            layouts.insert(window_id, window_rect);
        }

        layouts
    }

    /// Master-stack layout (one large master on left, stack on right)
    fn layout_master_stack(
        &self,
        column: &WorkspaceColumn,
        bounds: &Rectangle,
        gap: i32,
    ) -> HashMap<u64, Rectangle> {
        let mut layouts = HashMap::new();
        let window_count = column.windows.len();

        if window_count == 0 {
            return layouts;
        }

        if window_count == 1 {
            // Single window fills entire space
            let window_rect = Rectangle {
                x: bounds.x + gap,
                y: bounds.y + gap,
                width: (bounds.width as i32 - 2 * gap).max(1) as u32,
                height: (bounds.height as i32 - 2 * gap).max(1) as u32,
            };
            layouts.insert(column.windows[0], window_rect);
            return layouts;
        }

        // Master window takes 50% of width, stack takes the rest
        let master_width = (bounds.width as i32 - 3 * gap) / 2;
        let stack_width = bounds.width as i32 - master_width - 3 * gap;

        // Master window
        let master_rect = Rectangle {
            x: bounds.x + gap,
            y: bounds.y + gap,
            width: master_width.max(1) as u32,
            height: (bounds.height as i32 - 2 * gap).max(1) as u32,
        };
        layouts.insert(column.windows[0], master_rect);

        // Stack windows vertically on the right
        let stack_window_count = window_count - 1;
        let stack_height = (bounds.height as i32 - gap * (stack_window_count as i32 + 1))
            / stack_window_count as i32;

        for (i, &window_id) in column.windows[1..].iter().enumerate() {
            let y = bounds.y + gap + i as i32 * (stack_height + gap);
            let window_rect = Rectangle {
                x: bounds.x + master_width + 2 * gap,
                y,
                width: stack_width.max(1) as u32,
                height: stack_height.max(1) as u32,
            };
            layouts.insert(window_id, window_rect);
        }

        layouts
    }

    /// Grid layout (auto-arranges windows in optimal grid)
    fn layout_grid(
        &self,
        column: &WorkspaceColumn,
        bounds: &Rectangle,
        gap: i32,
    ) -> HashMap<u64, Rectangle> {
        let mut layouts = HashMap::new();
        let window_count = column.windows.len();

        if window_count == 0 {
            return layouts;
        }

        // Calculate optimal grid dimensions
        let cols = (window_count as f64).sqrt().ceil() as usize;
        let rows = (window_count as f64 / cols as f64).ceil() as usize;

        let cell_width = (bounds.width as i32 - gap * (cols as i32 + 1)) / cols as i32;
        let cell_height = (bounds.height as i32 - gap * (rows as i32 + 1)) / rows as i32;

        for (idx, &window_id) in column.windows.iter().enumerate() {
            let row = idx / cols;
            let col = idx % cols;

            let x = bounds.x + gap + col as i32 * (cell_width + gap);
            let y = bounds.y + gap + row as i32 * (cell_height + gap);

            let window_rect = Rectangle {
                x,
                y,
                width: cell_width.max(1) as u32,
                height: cell_height.max(1) as u32,
            };
            layouts.insert(window_id, window_rect);
        }

        layouts
    }

    /// Spiral layout (fibonacci-style tiling)
    fn layout_spiral(
        &self,
        column: &WorkspaceColumn,
        bounds: &Rectangle,
        gap: i32,
    ) -> HashMap<u64, Rectangle> {
        let mut layouts = HashMap::new();
        let window_count = column.windows.len();

        if window_count == 0 {
            return layouts;
        }

        if window_count == 1 {
            let window_rect = Rectangle {
                x: bounds.x + gap,
                y: bounds.y + gap,
                width: (bounds.width as i32 - 2 * gap).max(1) as u32,
                height: (bounds.height as i32 - 2 * gap).max(1) as u32,
            };
            layouts.insert(column.windows[0], window_rect);
            return layouts;
        }

        // For simplicity, spiral layout alternates between horizontal and vertical splits
        let mut rects = vec![bounds.clone()];
        let mut horizontal = true;

        for i in 0..window_count {
            if i >= rects.len() {
                break;
            }

            let current = rects[i].clone();
            let window_id = column.windows[i];

            if i == window_count - 1 {
                // Last window, use remaining space
                let window_rect = Rectangle {
                    x: current.x + gap,
                    y: current.y + gap,
                    width: (current.width as i32 - 2 * gap).max(1) as u32,
                    height: (current.height as i32 - 2 * gap).max(1) as u32,
                };
                layouts.insert(window_id, window_rect);
            } else {
                // Split current rectangle
                if horizontal {
                    let half_height = current.height as i32 / 2;
                    let window_rect = Rectangle {
                        x: current.x + gap,
                        y: current.y + gap,
                        width: (current.width as i32 - 2 * gap).max(1) as u32,
                        height: (half_height - gap).max(1) as u32,
                    };
                    layouts.insert(window_id, window_rect);

                    // Remaining space for next windows
                    rects.push(Rectangle {
                        x: current.x,
                        y: current.y + half_height,
                        width: current.width,
                        height: (current.height as i32 - half_height) as u32,
                    });
                } else {
                    let half_width = current.width as i32 / 2;
                    let window_rect = Rectangle {
                        x: current.x + gap,
                        y: current.y + gap,
                        width: (half_width - gap).max(1) as u32,
                        height: (current.height as i32 - 2 * gap).max(1) as u32,
                    };
                    layouts.insert(window_id, window_rect);

                    // Remaining space for next windows
                    rects.push(Rectangle {
                        x: current.x + half_width,
                        y: current.y,
                        width: (current.width as i32 - half_width) as u32,
                        height: current.height,
                    });
                }
                horizontal = !horizontal;
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
                let friction: f64 = self.config.momentum_friction.clamp(0.0, 0.9999);

                // Apply friction to velocity
                let current_velocity = velocity * friction.powf(elapsed * 60.0);

                if current_velocity.abs() < self.config.momentum_min_velocity {
                    // Momentum has died down, snap to nearest column if close enough
                    let nearest_column =
                        (self.current_position / self.config.workspace_width as f64).round() as i32;
                    let target_pos = nearest_column as f64 * self.config.workspace_width as f64;
                    if (self.current_position - target_pos).abs() <= self.config.snap_threshold_px {
                        self.scroll_to_column(nearest_column);
                    } else {
                        // If not close, continue gentle deceleration towards target
                        self.current_position = start_position + velocity * elapsed;
                        self.scroll_velocity = current_velocity;
                    }
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

    /// Get total number of active windows across all columns
    pub fn active_window_count(&self) -> usize {
        self.columns.values().map(|c| c.windows.len()).sum()
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

    /// Cycle to the next layout mode for the focused column
    pub fn cycle_layout_mode(&mut self) {
        let focused_col_idx = self.focused_column;
        let column = self.get_focused_column_mut();
        column.layout_mode = match column.layout_mode {
            LayoutMode::Vertical => LayoutMode::Horizontal,
            LayoutMode::Horizontal => LayoutMode::MasterStack,
            LayoutMode::MasterStack => LayoutMode::Grid,
            LayoutMode::Grid => LayoutMode::Spiral,
            LayoutMode::Spiral => LayoutMode::Vertical,
        };
        let new_mode = column.layout_mode;
        info!(
            "🔄 Cycled layout mode to {:?} for column {}",
            new_mode, focused_col_idx
        );
    }

    /// Set the layout mode for the focused column
    pub fn set_layout_mode(&mut self, mode: LayoutMode) {
        let focused_col_idx = self.focused_column;
        let column = self.get_focused_column_mut();
        column.layout_mode = mode;
        info!(
            "🎨 Set layout mode to {:?} for column {}",
            mode, focused_col_idx
        );
    }

    /// Get the current layout mode of the focused column
    pub fn get_layout_mode(&self) -> LayoutMode {
        self.get_focused_column()
            .map(|c| c.layout_mode)
            .unwrap_or(LayoutMode::Vertical)
    }

    /// Swap two windows within the focused column
    pub fn swap_windows_in_column(&mut self, index_a: usize, index_b: usize) -> Result<()> {
        let column = self.get_focused_column_mut();
        
        if index_a >= column.windows.len() || index_b >= column.windows.len() {
            return Err(anyhow::anyhow!("Window indices out of bounds"));
        }

        column.windows.swap(index_a, index_b);
        debug!(
            "🔀 Swapped windows at indices {} and {} in column {}",
            index_a, index_b, self.focused_column
        );
        Ok(())
    }

    /// Move a window within the focused column (change position)
    pub fn move_window_in_column(&mut self, window_id: u64, new_index: usize) -> Result<()> {
        let column = self.get_focused_column_mut();
        
        if let Some(old_index) = column.windows.iter().position(|&id| id == window_id) {
            if new_index >= column.windows.len() {
                return Err(anyhow::anyhow!("Target index out of bounds"));
            }
            
            let window = column.windows.remove(old_index);
            column.windows.insert(new_index, window);
            
            debug!(
                "📦 Moved window {} from index {} to {} in column {}",
                window_id, old_index, new_index, self.focused_column
            );
            Ok(())
        } else {
            Err(anyhow::anyhow!("Window {} not found in focused column", window_id))
        }
    }

    /// Focus the next window in the focused column
    pub fn focus_next_window_in_column(&mut self) -> Option<u64> {
        let column = self.get_focused_column_mut();
        
        if column.windows.is_empty() {
            return None;
        }
        
        let next_index = match column.focused_window_index {
            Some(idx) => (idx + 1) % column.windows.len(),
            None => 0,
        };
        
        column.focused_window_index = Some(next_index);
        let window_id = column.windows[next_index];
        
        debug!(
            "👆 Focused next window {} (index {}) in column {}",
            window_id, next_index, self.focused_column
        );
        
        Some(window_id)
    }

    /// Focus the previous window in the focused column
    pub fn focus_previous_window_in_column(&mut self) -> Option<u64> {
        let column = self.get_focused_column_mut();
        
        if column.windows.is_empty() {
            return None;
        }
        
        let prev_index = match column.focused_window_index {
            Some(idx) if idx > 0 => idx - 1,
            _ => column.windows.len() - 1,
        };
        
        column.focused_window_index = Some(prev_index);
        let window_id = column.windows[prev_index];
        
        debug!(
            "👆 Focused previous window {} (index {}) in column {}",
            window_id, prev_index, self.focused_column
        );
        
        Some(window_id)
    }

    /// Get the currently focused window ID in the focused column
    pub fn get_focused_window_in_column(&self) -> Option<u64> {
        self.get_focused_column().and_then(|column| {
            column.focused_window_index
                .and_then(|idx| column.windows.get(idx).copied())
        })
    }

    /// Move the focused window up in the stack (swap with previous)
    pub fn move_focused_window_up(&mut self) -> Result<()> {
        let column = self.get_focused_column_mut();
        
        if let Some(focused_idx) = column.focused_window_index {
            if focused_idx > 0 {
                column.windows.swap(focused_idx, focused_idx - 1);
                column.focused_window_index = Some(focused_idx - 1);
                debug!("⬆️ Moved focused window up in column {}", self.focused_column);
                return Ok(());
            }
        }
        
        Err(anyhow::anyhow!("Cannot move window up"))
    }

    /// Move the focused window down in the stack (swap with next)
    pub fn move_focused_window_down(&mut self) -> Result<()> {
        let column = self.get_focused_column_mut();
        let window_count = column.windows.len();
        
        if let Some(focused_idx) = column.focused_window_index {
            if focused_idx < window_count - 1 {
                column.windows.swap(focused_idx, focused_idx + 1);
                column.focused_window_index = Some(focused_idx + 1);
                debug!("⬇️ Moved focused window down in column {}", self.focused_column);
                return Ok(());
            }
        }
        
        Err(anyhow::anyhow!("Cannot move window down"))
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
