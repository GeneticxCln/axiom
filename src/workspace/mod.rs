//! Scrollable workspace management
//!
//! This module implements Axiom's core innovation: infinite scrollable
//! workspaces with smooth animations and intelligent window placement.
//!
//! Refactored for Multi-Monitor Support:
//! - [`WorkspaceTape`]: Represents a single scrollable strip (one per output).
//! - [`ScrollableWorkspaces`]: Manager that holds multiple tapes.

use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::config::WorkspaceConfig;
use crate::window::Rectangle;

/// Maximum number of workspace columns allowed per tape.
/// Prevents unbounded memory growth from a malicious or runaway client.
/// When the limit is reached, the oldest empty column is evicted to make
/// room for new columns. Non-empty columns are never evicted.
const MAX_COLUMNS: usize = 256;

/// Default viewport width (pixels) until updated by the backend.
const DEFAULT_VIEWPORT_WIDTH: f64 = 1920.0;

/// Default viewport height (pixels) until updated by the backend.
const DEFAULT_VIEWPORT_HEIGHT: f64 = 1080.0;

/// Base scroll animation duration (milliseconds).
const BASE_SCROLL_DURATION_MS: u64 = 250;

/// Distance normalization factor for scaling scroll duration.
const SCROLL_DISTANCE_NORMALIZER: f64 = 2000.0;

/// Maximum scroll animation duration (milliseconds).
const MAX_SCROLL_DURATION_MS: f64 = 800.0;

/// Minimum velocity threshold to start momentum scrolling.
const MIN_MOMENTUM_VELOCITY: f64 = 10.0;

/// Maximum delta time (seconds) to prevent huge jumps after pauses.
const MAX_DT_SECONDS: f64 = 1.0 / 30.0;

/// How long to keep empty columns before cleanup (seconds).
const COLUMN_CLEANUP_INTERVAL_SECS: u64 = 1;

/// How long empty columns survive before eviction (seconds).
const EMPTY_COLUMN_TTL_SECS: u64 = 30;

/// Velocity decay factor per frame when idle.
const IDLE_VELOCITY_DECAY: f64 = 0.9;

/// Velocity threshold below which idle velocity is zeroed.
const IDLE_VELOCITY_ZERO_THRESHOLD: f64 = 0.1;

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
    /// Create a new workspace column at the given index and position.
    pub fn new(index: i32, position: f64) -> Self {
        Self {
            index,
            position,
            windows: Vec::new(),
            active: false,
            last_accessed: Instant::now(),
        }
    }

    /// Add a window to this column if not already present.
    pub fn add_window(&mut self, window_id: u64) {
        if !self.windows.contains(&window_id) {
            self.windows.push(window_id);
            self.last_accessed = Instant::now();
        }
    }

    /// Remove a window from this column. Returns `true` if found and removed.
    pub fn remove_window(&mut self, window_id: u64) -> bool {
        if let Some(pos) = self.windows.iter().position(|&id| id == window_id) {
            self.windows.remove(pos);
            self.last_accessed = Instant::now();
            true
        } else {
            false
        }
    }

    /// Returns `true` if this column contains no windows.
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

/// A single scrollable tape of workspaces (corresponds to one output/monitor)
#[derive(Debug)]
pub struct WorkspaceTape {
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

    /// Max columns allowed (bound grows in both directions).
    /// Prevents unbounded memory growth from a malicious or runaway client.
    max_columns: usize,

    /// Animation state
    scroll_state: ScrollState,

    /// Viewport bounds (what's currently visible)
    viewport_width: f64,
    viewport_height: f64,

    /// Animation easing parameters
    last_update: Instant,

    /// Last time cleanup was performed
    last_cleanup: Instant,

    /// Output DPI scale factor (e.g. 1.0 for normal, 2.0 for HiDPI).
    /// Configured by the backend during output setup; used by
    /// `calculate_workspace_layouts` to produce logical-space window
    /// rectangles that HiDPI-aware clients can consume directly.
    scale_factor: f64,
}

impl WorkspaceTape {
    pub fn new(config: &WorkspaceConfig) -> Self {
        let mut tape = Self {
            config: config.clone(),
            current_position: 0.0,
            target_position: 0.0,
            scroll_velocity: 0.0,
            columns: HashMap::new(),
            focused_column: 0,
            max_columns: MAX_COLUMNS,
            scroll_state: ScrollState::Idle,
            viewport_width: DEFAULT_VIEWPORT_WIDTH,
            viewport_height: DEFAULT_VIEWPORT_HEIGHT,
            last_update: Instant::now(),
            last_cleanup: Instant::now(),
            scale_factor: 1.0,
        };

        // Create the initial workspace column
        tape.ensure_column(0);
        tape
    }

    /// Update configuration
    pub fn update_config(&mut self, config: WorkspaceConfig) {
        self.config = config;
    }

    /// Check if a window exists in any column
    pub fn window_exists(&self, window_id: u64) -> bool {
        self.columns
            .values()
            .any(|c| c.windows.contains(&window_id))
    }

    /// Update the viewport size (called when window/display size changes)
    pub fn set_viewport_size(&mut self, width: f64, height: f64) {
        self.viewport_width = width;
        self.viewport_height = height;
        debug!("📐 Viewport size updated to {}x{}", width, height);
    }

    /// Ensure a column exists at the given index.
    ///
    /// When the column map already has `max_columns` entries and the
    /// requested index is absent, the oldest empty column (by
    /// `last_accessed`) is evicted to stay within the bound. If no
    /// column is eligible for eviction the request is silently dropped
    /// and a temporary default column is returned (the caller should
    /// not hold the reference across an await point).
    pub fn ensure_column(&mut self, index: i32) -> &mut WorkspaceColumn {
        if !self.columns.contains_key(&index) {
            if self.columns.len() >= self.max_columns {
                // Try to evict the oldest empty column that isn't focused
                let to_evict = self
                    .columns
                    .iter()
                    .filter(|(i, c)| **i != self.focused_column && c.is_empty())
                    .min_by_key(|(_, c)| c.last_accessed)
                    .map(|(i, _)| *i);
                if let Some(evict_idx) = to_evict {
                    self.columns.remove(&evict_idx);
                    debug!(
                        "📦 Evicted column {} to stay under max_columns ({})",
                        evict_idx, self.max_columns
                    );
                }
                // If still at capacity (no empty column to evict), refuse
                // and return the focused column so callers don't crash.
                if self.columns.len() >= self.max_columns {
                    warn!(
                        "🚫 Column map at capacity ({}) — refusing to create column {}",
                        self.max_columns, index
                    );
                    return self
                        .columns
                        .get_mut(&self.focused_column)
                        .expect("focused column exists");
                }
            }
            let position = index as f64 * self.config.workspace_width as f64;
            let column = WorkspaceColumn::new(index, position);
            debug!(
                "📄 Created new workspace column {} at position {}",
                index, position
            );
            self.columns.insert(index, column);
        }
        self.columns
            .get_mut(&index)
            .expect("column was just inserted")
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
        let base_duration = Duration::from_millis(BASE_SCROLL_DURATION_MS);
        let duration = Duration::from_millis(
            (base_duration.as_millis() as f64 * (1.0 + distance / SCROLL_DISTANCE_NORMALIZER))
                .min(MAX_SCROLL_DURATION_MS) as u64,
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
        if velocity.abs() > MIN_MOMENTUM_VELOCITY {
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
        for (column_index, column) in &mut self.columns {
            if column.remove_window(window_id) {
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

    /// Get all windows in the currently focused column. This is the
    /// **logical-target** column (the column the user's last input
    /// commanded), updated instantly in `scroll_to_column()`,
    /// independent of any in-flight scroll or momentum animation.
    /// Use this for input routing, focus-driven engine passes, and
    /// any query where "what did the user just ask for?" matters
    /// more than "what is on screen right now?".
    ///
    /// For the column the user is *visually* looking at during a
    /// smooth-scroll animation (which lags behind `focused_column`
    /// by up to `MAX_SCROLL_DURATION_MS`), use
    /// [`Self::visual_focused_column_index`] directly. The pre-fix
    /// audit wired `get_focused_column_windows()` to that visual
    /// path, which produced a real correctness bug under rapid
    /// back-and-forth scroll: rounded visual position often pointed
    /// *backward* relative to the user's most recent input (e.g.
    /// scroll-left then scroll-right returned the left column's
    /// windows while the right animation was still mid-flight),
    /// causing input and effect-engine callers to target the wrong
    /// column.
    pub fn get_focused_column_windows(&self) -> Vec<u64> {
        self.columns
            .get(&self.focused_column)
            .map(|column| column.windows.clone())
            .unwrap_or_default()
    }

    /// The column index that the user is currently looking at.
    /// Returns `focused_column` once scrolling has settled, and the
    /// column whose center contains `current_position` while a
    /// scroll or momentum animation is in flight. Safe against a
    /// zero workspace-width (returns `focused_column`); safe against
    /// a mid-animation column map that has not yet created the
    /// target column (falls back to the closest populated column
    /// via `get_focused_column()` semantics upstream).
    ///
    /// Public because it serves a different semantic than
    /// [`Self::focused_column`]: a rendering/framebuffer-positioning
    /// caller wants the on-screen column (this method), while a
    /// logical-target / input-routing caller wants the user's last
    /// commanded destination ([`Self::focused_column`], exposed via
    /// [`Self::get_focused_column_windows`]). Keep both APIs
    /// available so callers can pick the one that matches their
    /// semantic.
    pub fn visual_focused_column_index(&self) -> i32 {
        if matches!(
            self.scroll_state,
            ScrollState::Scrolling { .. } | ScrollState::Momentum { .. }
        ) {
            let width = self.config.workspace_width as f64;
            if width > 0.0 {
                let raw = (self.current_position / width).round() as i32;
                // If the in-flight column hasn't been instantiated
                // yet (rare — happens during very fast multi-column
                // scrolls), prefer `focused_column` so we don't
                // return an empty window list.
                if self.columns.contains_key(&raw) {
                    raw
                } else {
                    self.focused_column
                }
            } else {
                self.focused_column
            }
        } else {
            self.focused_column
        }
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

    /// Update animations and smooth scrolling
    pub fn update_animations(&mut self) {
        let now = Instant::now();
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
                // Clamp dt to prevent huge jumps after GC pauses, debug
                // breakpoints, or system suspend (max ~33 ms = 30 FPS).
                let dt = elapsed.min(MAX_DT_SECONDS);
                let friction: f64 = self.config.momentum_friction.clamp(0.0, 0.9999);

                // Apply friction to velocity using clamped dt
                let current_velocity = velocity * friction.powf(dt * 60.0);

                if current_velocity.abs() < self.config.momentum_min_velocity {
                    // Momentum has died down, snap to nearest column if close enough
                    let nearest_column =
                        (self.current_position / self.config.workspace_width as f64).round() as i32;
                    let target_pos = nearest_column as f64 * self.config.workspace_width as f64;
                    if (self.current_position - target_pos).abs() <= self.config.snap_threshold_px {
                        self.scroll_to_column(nearest_column);
                    } else {
                        // If not close, continue deceleration towards target.
                        // Clamp to prevent overshoot oscillation.
                        let raw_pos = start_position + velocity * elapsed;
                        self.current_position = if velocity > 0.0 {
                            raw_pos.min(target_pos + self.config.snap_threshold_px)
                        } else {
                            raw_pos.max(target_pos - self.config.snap_threshold_px)
                        };
                        self.scroll_velocity = current_velocity;
                    }
                } else {
                    // Update position based on momentum, clamped to avoid
                    // overshoot oscillation when near the target.
                    self.current_position = start_position + velocity * dt;
                    self.scroll_velocity = current_velocity;
                }
            }

            ScrollState::Idle => {
                // Gradually reduce any remaining velocity
                self.scroll_velocity *= IDLE_VELOCITY_DECAY;
                if self.scroll_velocity.abs() < IDLE_VELOCITY_ZERO_THRESHOLD {
                    self.scroll_velocity = 0.0;
                }
            }
        }

        // Cleanup empty columns that haven't been accessed in a while
        if now.duration_since(self.last_cleanup) > Duration::from_secs(COLUMN_CLEANUP_INTERVAL_SECS)
        {
            self.cleanup_empty_columns();
            self.last_cleanup = now;
        }
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
        let cleanup_threshold = Duration::from_secs(EMPTY_COLUMN_TTL_SECS);

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

    /// Get total number of active columns
    pub fn active_column_count(&self) -> usize {
        self.columns.len()
    }

    /// Get current scroll position
    pub fn current_position(&self) -> f64 {
        self.current_position
    }

    /// Set the output DPI scale factor for this tape.
    /// Clamped to [1.0, 4.0] since fractional scales between 1x and 4x
    /// cover typical HiDPI hardware; extreme values are clamped.
    pub fn set_scale_factor(&mut self, factor: f64) {
        self.scale_factor = factor.clamp(1.0, 4.0);
        debug!("Tape scale factor set to {:.1}x", self.scale_factor);
    }

    /// Get the output DPI scale factor for this tape.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }
}

/// Scrollable workspace manager (Top-level Multi-Monitor)
#[derive(Debug)]
pub struct ScrollableWorkspaces {
    config: WorkspaceConfig,

    /// Tapes for each output (Output ID -> Tape)
    tapes: HashMap<String, WorkspaceTape>,

    /// The currently focused output ID
    pub focused_output: String,

    /// Cached layout map from `calculate_workspace_layouts`, invalidated
    /// whenever the scroll position changes. This avoids recomputing all
    /// window rectangles on every pointer motion (the hot path for
    /// `element_under`).
    ///
    /// Uses `parking_lot::Mutex` rather than `RefCell` because the
    /// outer `Arc<parking_lot::RwLock<ScrollableWorkspaces>>` is held
    /// under a *read* guard by multiple backend/pointer paths in
    /// parallel (e.g. `backend::render` and `process_events` both
    /// acquire the workspace_manager read lock). With `RefCell`,
    /// `borrow_mut()` would panic when two readers competed. The
    /// mutex serialises correctly — and since both call sites
    /// (`calculate_workspace_layouts` and `update_animations`) do
    /// not recurse, there is no risk of self-deadlock.
    cached_layouts: parking_lot::Mutex<Option<(f64, HashMap<u64, Rectangle>)>>,

    /// Set of window IDs that are currently minimized (hidden). Such
    /// windows are still owned (the compositor can restore them) but
    /// `calculate_workspace_layouts` skips them so the renderer
    /// never allocates a rectangle for them. Mirrors
    /// [`crate::window::WindowManager`]'s `minimized` flag at the
    /// workspace layer — both must agree on every minimize/restore call.
    /// See `minimize_window` / `restore_window` below.
    minimized_windows: HashSet<u64>,
    /// Per-window originating column captured at minimize time. When a
    /// window is restored, that column is preferred over the focused
    /// column so the window "comes back" to where it was instead of
    /// quietly sliding to whichever workspace the user happens to be
    /// viewing. Entries are inserted on `minimize_window` and removed
    /// on `restore_window`; `restore_window` falls back to focused
    /// when this map has no entry (e.g. window was empty before
    /// minimize or a hot-reload cleared the in-memory map).
    pub originating_column: HashMap<u64, i32>,

    /// Set of window IDs currently in floating mode. `calculate_workspace_layouts`
    /// skips these windows so they are not auto-tiled. Must be kept in sync
    /// with `WindowManager`'s `properties.floating` by the caller.
    floating_windows: HashSet<u64>,
}

impl ScrollableWorkspaces {
    /// Backwards-compatible helper used by older tests
    pub fn remove_window_bool(&mut self, window_id: u64) -> bool {
        self.remove_window(window_id).is_some()
    }

    /// Create a new scrollable workspace manager with a default tape.
    pub fn new(config: &WorkspaceConfig) -> Self {
        let mut manager = Self {
            config: config.clone(),
            tapes: HashMap::new(),
            focused_output: "default".to_string(),
            cached_layouts: parking_lot::Mutex::new(None),
            minimized_windows: HashSet::new(),
            originating_column: HashMap::new(),
            floating_windows: HashSet::new(),
        };

        // Create default tape
        manager.ensure_tape("default");

        info!("🔄 Scrollable workspaces initialized with multi-monitor support");
        manager
    }

    /// Mark a window as minimized on the workspace layer.
    ///
    /// Removes the window from whatever column it currently occupies on
    /// every tape (so `calculate_workspace_layouts` no longer emits a
    /// rectangle for it) and adds the ID to [`minimized_windows`]. The
    /// companion `WindowManager::minimize_window` must be called by the
    /// caller to keep both layers in sync — this method does not touch
    /// it. Returns `true` when the window was actually located on a
    /// tape (and therefore hidden); `false` is treated as a no-op.
    ///
    /// Invalidation of `cached_layouts` is implied: since the window is
    /// removed from a column, the cached map may already include it
    /// and must be re-computed on the next layout query.
    pub fn minimize_window(&mut self, window_id: u64) -> bool {
        // Idempotent: minimize of an already-minimized window is a no-op.
        if self.minimized_windows.contains(&window_id) {
            return false;
        }
        let mut removed_anywhere = false;
        let mut last_column: Option<i32> = None;
        for tape in self.tapes.values_mut() {
            if let Some(col) = tape.remove_window_internal(window_id) {
                removed_anywhere = true;
                last_column = Some(col);
            }
        }
        // Remember the originating column so restore_window can put the
        // window back where it came from. If the window is on more than
        // one tape (defense-in-depth — a window is unique per workspace
        // manager today but multi-monitor could change that), keep the
        // last-seen column. The restoring path falls back to focused
        // when this map has no entry.
        if let Some(col) = last_column {
            self.originating_column.insert(window_id, col);
        }
        // Even when the window is not in any column yet (e.g. layout is
        // built lazily), still mark it minimized so a future layout
        // query that DOES see it will filter accordingly.
        self.minimized_windows.insert(window_id);
        *self.cached_layouts.lock() = None;
        debug!(
            "📦 Workspace: minimized window {} (removed_from_column={}, origin={:?})",
            window_id, removed_anywhere, last_column
        );
        true
    }

    /// Restore a minimized window on the workspace layer.
    ///
    /// Re-adds the window to the focused column of the focused tape,
    /// removes its ID from [`minimized_windows`], and invalidates the
    /// layout cache. Returns `true` when the window was actually
    /// minimized and has now been added back to a column.
    pub fn restore_window(&mut self, window_id: u64) -> bool {
        if !self.minimized_windows.remove(&window_id) {
            return false;
        }
        // Prefer the originating column when present; otherwise the
        // focused column (preserves the pre-minimize workspace choice).
        let target_column = self
            .originating_column
            .remove(&window_id)
            .unwrap_or_else(|| self.active_tape().focused_column);
        // Capture the focused column BEFORE the mutable borrow so we
        // can drop the tape borrow before asking RefCell for another
        // mutable borrow on `cached_layouts`. Two simultaneous
        // `&mut self.<field>` borrows across the same scope trigger
        // E0502; the explicit inner block lets NLL release the
        // `tape` borrow before `cached_layouts` takes its own.
        let focused_column = self.active_tape().focused_column;
        {
            let tape = self.active_tape_mut();
            tape.add_window_to_column(window_id, target_column);
        }
        *self.cached_layouts.lock() = None;
        debug!(
            "📦 Workspace: restored window {} to column {} (focused = {})",
            window_id, target_column, focused_column
        );
        true
    }

    /// Is the given window currently minimized at the workspace layer?
    /// Reads the [`minimized_windows`] set synchronously; does not
    /// consult the column map (a minimized window is, by definition,
    /// absent from any column).
    pub fn is_window_minimized(&self, window_id: u64) -> bool {
        self.minimized_windows.contains(&window_id)
    }

    /// Number of currently-minimized windows across all tapes.
    pub fn minimized_window_count(&self) -> usize {
        self.minimized_windows.len()
    }

    /// Ensure a tape exists for the given output
    pub fn ensure_tape(&mut self, output_id: &str) -> &mut WorkspaceTape {
        self.tapes.entry(output_id.to_string()).or_insert_with(|| {
            info!("Creating workspace tape for output: {}", output_id);
            WorkspaceTape::new(&self.config)
        })
    }

    /// Get the active tape (read-only reference).
    pub fn active_tape(&self) -> &WorkspaceTape {
        // Fallback: return the first tape if focused_output is stale.
        // `new()` always creates the "default" tape, and `ensure_tape`
        // guarantees at least one tape exists after any call path.
        self.tapes.get(&self.focused_output).unwrap_or_else(|| {
            self.tapes
                .values()
                .next()
                .expect("at least one tape exists — new() creates 'default'")
        })
    }

    /// Return the active tape, or `None` when the tape map is empty
    /// (defense-in-depth against hypothetical empty-state bugs).
    pub fn active_tape_opt(&self) -> Option<&WorkspaceTape> {
        self.tapes
            .get(&self.focused_output)
            .or_else(|| self.tapes.values().next())
    }

    /// Update configuration
    pub fn update_config(&mut self, config: WorkspaceConfig) {
        self.config = config.clone();
        for tape in self.tapes.values_mut() {
            tape.update_config(config.clone());
        }
        info!("🔄 Updated workspace configuration");
    }

    /// Get the active tape mutably
    pub fn active_tape_mut(&mut self) -> &mut WorkspaceTape {
        // Guarantee the focused tape exists before returning it.
        self.ensure_tape(&self.focused_output.clone());
        self.tapes
            .get_mut(&self.focused_output)
            .expect("tape was just ensured")
    }

    // --- Delegation methods to active tape (to maintain API compatibility) ---

    /// Check if a window exists in any tape.
    pub fn window_exists(&self, window_id: u64) -> bool {
        self.tapes.values().any(|t| t.window_exists(window_id))
    }

    /// Check if infinite scrolling is enabled.
    pub fn is_infinite_scroll_enabled(&self) -> bool {
        self.config.infinite_scroll
    }

    /// Set the viewport size on the active tape.
    pub fn set_viewport_size(&mut self, width: f64, height: f64) {
        self.active_tape_mut().set_viewport_size(width, height);
        // Viewport resize invalidates every cached layout — the new
        // dimensions change column widths and window tiling.
        *self.cached_layouts.lock() = None;
    }

    /// Add a window to a specific column on the active tape.
    pub fn add_window_to_column(&mut self, window_id: u64, column_index: i32) {
        self.active_tape_mut()
            .add_window_to_column(window_id, column_index);
    }

    // Missing methods from original impl that are likely used
    /// Get the focused column mutably from the active tape.
    pub fn get_focused_column_mut(&mut self) -> &mut WorkspaceColumn {
        self.active_tape_mut().get_focused_column_mut()
    }

    /// Get the focused column from the active tape, if any.
    pub fn get_focused_column_opt(&self) -> Option<&WorkspaceColumn> {
        self.active_tape().get_focused_column()
    }

    /// Start momentum scrolling on the active tape.
    pub fn start_momentum_scroll(&mut self, velocity: f64) {
        self.active_tape_mut().start_momentum_scroll(velocity);
    }

    /// Scroll the active tape left by one workspace.
    pub fn scroll_left(&mut self) {
        self.active_tape_mut().scroll_left();
    }

    /// Scroll the active tape right by one workspace.
    pub fn scroll_right(&mut self) {
        self.active_tape_mut().scroll_right();
    }

    /// Add a window to the active tape's focused column.
    pub fn add_window(&mut self, window_id: u64) {
        self.active_tape_mut().add_window(window_id);
    }

    /// Remove a window from all tapes. Returns the column index if found.
    pub fn remove_window(&mut self, window_id: u64) -> Option<i32> {
        // Search all tapes (a window is unique across all workspaces)
        for tape in self.tapes.values_mut() {
            if let Some(col) = tape.remove_window(window_id) {
                return Some(col);
            }
        }
        None
    }

    /// Move a window left on the active tape.
    pub fn move_window_left(&mut self, window_id: u64) -> bool {
        self.active_tape_mut().move_window_left(window_id)
    }

    /// Move a window right on the active tape.
    pub fn move_window_right(&mut self, window_id: u64) -> bool {
        self.active_tape_mut().move_window_right(window_id)
    }

    /// Get windows in the focused column of the active tape.
    pub fn get_focused_column_windows(&self) -> Vec<u64> {
        self.active_tape().get_focused_column_windows()
    }

    /// Update animations on all tapes.
    pub fn update_animations(&mut self) {
        for tape in self.tapes.values_mut() {
            tape.update_animations();
        }
        // Invalidate layout cache since scroll positions may have changed
        *self.cached_layouts.lock() = None;
    }

    /// Calculate layout rectangles for all visible windows across all tapes.
    pub fn calculate_workspace_layouts(&self) -> HashMap<u64, Rectangle> {
        // Return cached layouts if still valid. `parking_lot::Mutex::lock`
        // returns a guard that derefs to the inner value via the `Deref`
        // target — couple it with the deref-coerce pattern to keep the
        // single-read cost down.
        let tape = self.active_tape();
        if let Some((cached_pos, ref cached)) = *self.cached_layouts.lock() {
            if (cached_pos - tape.current_position).abs() < f64::EPSILON {
                return cached.clone();
            }
        }

        // Collect layouts from ALL tapes (outputs)
        // Note: For now, we assume a single viewport for backwards compatibility or just return relative coords.
        // In real multi-monitor, this would offset by output position.

        let tape = self.active_tape();
        let mut layouts = HashMap::new();
        let visible_columns = tape.get_visible_columns();

        for column in visible_columns {
            let column_offset = column.position - tape.current_position;
            let column_left = (tape.viewport_width / 2.0) + column_offset;

            if column_left + tape.config.workspace_width as f64 >= 0.0
                && column_left <= tape.viewport_width
            {
                let column_bounds = Rectangle {
                    x: column_left as i32,
                    y: 0,
                    width: tape.config.workspace_width,
                    height: tape.viewport_height as u32,
                };

                if !column.windows.is_empty() {
                    let gap = tape.config.gaps as i32;
                    let total_gap_space = gap * (column.windows.len() as i32 + 1);
                    let available = (column_bounds.height as i32).saturating_sub(total_gap_space);
                    let window_count = column.windows.len() as i32;
                    let window_height = if window_count > 0 && available > 0 {
                        available / window_count
                    } else {
                        1 // minimum 1-pixel window so it's at least visible
                    };

                    for (i, &window_id) in column.windows.iter().enumerate() {
                        // Skip minimized windows — the renderer should not
                        // allocate a rectangle for them because they are
                        // hidden behind the compositor and excluded from
                        // pointer hit-testing by `element_under` (which
                        // reads the same returned map). The hidden window
                        // still lives in `minimized_windows` for the
                        // restore path.
                        if self.minimized_windows.contains(&window_id) {
                            continue;
                        }
                        // Floating windows are manually positioned and must
                        // not receive a tiled layout rect.
                        if self.floating_windows.contains(&window_id) {
                            continue;
                        }
                        let y = gap + i as i32 * (window_height + gap);
                        let width = column_bounds.width.saturating_sub(2 * gap as u32).max(1);
                        let height = (window_height as u32).max(1);
                        let window_rect = Rectangle {
                            x: column_bounds.x + gap,
                            y,
                            width,
                            height,
                        };
                        layouts.insert(window_id, window_rect);
                    }
                }
            }
        }

        *self.cached_layouts.lock() = Some((tape.current_position, layouts.clone()));
        layouts
    }

    /// Find the window under a given point (x, y) in viewport coordinates.
    /// Checks both tiled layouts and any extra window rects provided via
    /// `floating_rects` (floating / manually-positioned windows whose
    /// positions are stored in the window manager, not in workspace layouts).
    /// Returns the window ID and the relative coordinates within the window.
    pub fn element_under(
        &self,
        x: f64,
        y: f64,
        floating_rects: &[(u64, i32, i32, u32, u32)],
    ) -> Option<(u64, (f64, f64))> {
        // Check floating windows first (they render on top).
        for &(window_id, fx, fy, fw, fh) in floating_rects {
            if x >= fx as f64
                && x < (fx + fw as i32) as f64
                && y >= fy as f64
                && y < (fy + fh as i32) as f64
            {
                let relative_x = x - fx as f64;
                let relative_y = y - fy as f64;
                return Some((window_id, (relative_x, relative_y)));
            }
        }
        // Then check tiled layouts.
        let layouts = self.calculate_workspace_layouts();
        for (window_id, rect) in layouts {
            if x >= rect.x as f64
                && x < (rect.x + rect.width as i32) as f64
                && y >= rect.y as f64
                && y < (rect.y + rect.height as i32) as f64
            {
                let relative_x = x - rect.x as f64;
                let relative_y = y - rect.y as f64;
                return Some((window_id, (relative_x, relative_y)));
            }
        }
        None
    }

    // --- State Getters (Backwards Compatibility) ---

    /// Get the focused column index on the active tape.
    pub fn focused_column_index(&self) -> i32 {
        self.active_tape().focused_column
    }

    /// Get the current scroll position on the active tape.
    pub fn current_position(&self) -> f64 {
        self.active_tape().current_position
    }

    /// Get the total number of columns across all tapes.
    pub fn active_column_count(&self) -> usize {
        self.active_tape().columns.len()
    }

    /// Check if the active tape is currently scrolling or has momentum.
    pub fn is_scrolling(&self) -> bool {
        matches!(
            self.active_tape().scroll_state,
            ScrollState::Scrolling { .. } | ScrollState::Momentum { .. }
        )
    }

    /// Get the DPI scale factor of the active tape.
    pub fn scale_factor(&self) -> f64 {
        self.active_tape().scale_factor()
    }

    /// Get the scroll progress (0.0 to 1.0) of the active tape.
    pub fn scroll_progress(&self) -> f64 {
        match self.active_tape().scroll_state {
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

    /// Set the floating state for a window. Floating windows are exempt
    /// from auto-tiling in `calculate_workspace_layouts` — they must be
    /// positioned and rendered by the caller (typically the backend during
    /// an interactive move).
    pub fn set_window_floating(&mut self, window_id: u64, floating: bool) {
        if floating {
            self.floating_windows.insert(window_id);
        } else {
            self.floating_windows.remove(&window_id);
            // Invalidate cached layouts so the window re-enters tiling.
            *self.cached_layouts.lock() = None;
        }
    }

    /// Toggle the floating state for a window.
    pub fn toggle_window_floating(&mut self, window_id: u64) -> bool {
        let is_floating = self.floating_windows.contains(&window_id);
        self.set_window_floating(window_id, !is_floating);
        !is_floating
    }

    /// Check whether a window is in floating mode.
    pub fn is_window_floating(&self, window_id: u64) -> bool {
        self.floating_windows.contains(&window_id)
    }

    /// Return a snapshot of all floating window IDs.
    pub fn floating_window_ids(&self) -> Vec<u64> {
        self.floating_windows.iter().copied().collect()
    }

    /// Shut down all tapes and clear state.
    pub fn shutdown(&mut self) {
        info!("🔽 Shutting down scrollable workspaces...");
        self.tapes.clear();
        self.minimized_windows.clear();
        self.originating_column.clear();
        *self.cached_layouts.lock() = None;
    }
}

#[cfg(test)]
mod tests;
