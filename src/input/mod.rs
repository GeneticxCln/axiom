//! Phase 3: Enhanced input handling and key bindings
//! Manages keyboard, mouse, and gesture input with real processing
#![allow(missing_docs)]

use crate::config::{BindingsConfig, InputConfig};
use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;

/// Represents different types of input events
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// Keyboard key press/release
    Keyboard {
        key: String,
        modifiers: Vec<String>,
        pressed: bool,
    },
    /// Mouse button press/release
    MouseButton {
        button: MouseButton,
        pressed: bool,
        x: f64,
        y: f64,
    },
    /// Mouse movement
    MouseMove {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    },
    /// Scroll wheel/trackpad scrolling
    Scroll {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    },
    /// Touch/gesture events
    Gesture {
        gesture_type: GestureType,
        delta_x: f64,
        delta_y: f64,
        velocity: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GestureType {
    Swipe,
    Pinch,
    Pan,
}

/// Represents compositor actions that can be triggered by input
#[derive(Debug, Clone, PartialEq)]
pub enum CompositorAction {
    ScrollWorkspaceLeft,
    ScrollWorkspaceRight,
    MoveWindowLeft,
    MoveWindowRight,
    #[allow(dead_code)]
    CloseWindow,
    #[allow(dead_code)]
    ToggleFullscreen,
    Quit,
    #[allow(dead_code)]
    Custom(String),
}

/// Phase 3: Enhanced input manager with real processing
#[derive(Debug)]
pub struct InputManager {
    #[allow(dead_code)]
    input_config: InputConfig,
    #[allow(dead_code)]
    bindings_config: BindingsConfig,

    /// Key binding mappings
    key_bindings: HashMap<String, CompositorAction>,

    /// Mouse button bindings
    mouse_bindings: HashMap<MouseButton, CompositorAction>,

    /// Current modifier state
    active_modifiers: Vec<String>,

    /// Mouse state
    mouse_position: (f64, f64),

    /// Gesture state for momentum scrolling
    #[allow(dead_code)]
    gesture_state: Option<GestureState>,

    /// Drag mode state
    drag_mode: Option<DragMode>,
    drag_start: Option<(f64, f64)>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct GestureState {
    start_time: std::time::Instant,
    start_position: (f64, f64),
    current_velocity: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    Move,
    Resize,
}

impl InputManager {
    pub fn new(input_config: &InputConfig, bindings_config: &BindingsConfig) -> Result<Self> {
        info!("⌨️ Phase 3: Initializing enhanced input manager...");

        // Parse key bindings from config
        let mut key_bindings = HashMap::new();
        key_bindings.insert(
            bindings_config.scroll_left.clone(),
            CompositorAction::ScrollWorkspaceLeft,
        );
        key_bindings.insert(
            bindings_config.scroll_right.clone(),
            CompositorAction::ScrollWorkspaceRight,
        );
        key_bindings.insert(
            bindings_config.move_window_left.clone(),
            CompositorAction::MoveWindowLeft,
        );
        key_bindings.insert(
            bindings_config.move_window_right.clone(),
            CompositorAction::MoveWindowRight,
        );
        key_bindings.insert(bindings_config.quit.clone(), CompositorAction::Quit);
        key_bindings.insert(
            bindings_config.toggle_fullscreen.clone(),
            CompositorAction::ToggleFullscreen,
        );
        key_bindings.insert(
            bindings_config.close_window.clone(),
            CompositorAction::CloseWindow,
        );

        // Parse mouse button bindings from config (optional)
        let mut mouse_bindings: HashMap<MouseButton, CompositorAction> = HashMap::new();
        if !bindings_config.mouse_left.trim().is_empty() {
            if let Some(action) = Self::parse_action_name(&bindings_config.mouse_left) {
                mouse_bindings.insert(MouseButton::Left, action);
            }
        }
        if !bindings_config.mouse_right.trim().is_empty() {
            if let Some(action) = Self::parse_action_name(&bindings_config.mouse_right) {
                mouse_bindings.insert(MouseButton::Right, action);
            }
        }
        if !bindings_config.mouse_middle.trim().is_empty() {
            if let Some(action) = Self::parse_action_name(&bindings_config.mouse_middle) {
                mouse_bindings.insert(MouseButton::Middle, action);
            }
        }

        debug!("🔑 Loaded {} key bindings", key_bindings.len());

        Ok(Self {
            input_config: input_config.clone(),
            bindings_config: bindings_config.clone(),
            key_bindings,
            mouse_bindings,
            active_modifiers: Vec::new(),
            mouse_position: (0.0, 0.0),
            gesture_state: None,
            drag_mode: None,
            drag_start: None,
        })
    }

    /// Process an input event and return any triggered actions
    pub fn process_input_event(&mut self, event: InputEvent) -> Vec<CompositorAction> {
        match event {
            InputEvent::Keyboard {
                key,
                modifiers,
                pressed,
            } => self.process_keyboard_event(key, modifiers, pressed),
            InputEvent::MouseButton {
                button,
                pressed,
                x,
                y,
            } => self.process_mouse_button(button, pressed, x, y),
            InputEvent::MouseMove {
                x,
                y,
                delta_x: _,
                delta_y: _,
            } => {
                let mut actions = Vec::new();
                // Handle drag motion if active
                if let Some(mode) = self.drag_mode {
                    if let Some((sx, sy)) = self.drag_start {
                        let dx = x - sx;
                        let dy = y - sy;
                        if dx.abs() + dy.abs() > self.input_config.drag_threshold {
                            match mode {
                                DragMode::Move => {
                                    // Map to move window left/right based on sign; fine-grained movement is TBD
                                    if dx > 0.0 {
                                        actions.push(CompositorAction::MoveWindowRight);
                                    } else {
                                        actions.push(CompositorAction::MoveWindowLeft);
                                    }
                                }
                                DragMode::Resize => {
                                    // Reuse move actions as placeholder for resize
                                    if dx > 0.0 {
                                        actions.push(CompositorAction::MoveWindowRight);
                                    } else {
                                        actions.push(CompositorAction::MoveWindowLeft);
                                    }
                                }
                            }
                        }
                    }
                }
                self.mouse_position = (x, y);
                actions
            }
            InputEvent::Scroll {
                x,
                y,
                delta_x,
                delta_y,
            } => self.process_scroll_event(x, y, delta_x, delta_y),
            InputEvent::Gesture {
                gesture_type,
                delta_x,
                delta_y,
                velocity,
            } => self.process_gesture_event(gesture_type, delta_x, delta_y, velocity),
        }
    }

    /// Process keyboard events
    fn process_keyboard_event(
        &mut self,
        key: String,
        modifiers: Vec<String>,
        pressed: bool,
    ) -> Vec<CompositorAction> {
        if pressed {
            // Update modifier state
            for modifier in &modifiers {
                if !self.active_modifiers.contains(modifier) {
                    self.active_modifiers.push(modifier.clone());
                }
            }

            // Create key combination string
            let key_combo = if modifiers.is_empty() {
                key
            } else {
                format!("{}+{}", modifiers.join("+"), key)
            };

            debug!("⌨️ Key pressed: {}", key_combo);

            // Check for matching binding
            if let Some(action) = self.key_bindings.get(&key_combo) {
                info!("🚀 Triggered action: {:?}", action);
                return vec![action.clone()];
            }
        } else {
            // Remove modifiers when keys are released
            self.active_modifiers.retain(|m| !modifiers.contains(m));
        }

        Vec::new()
    }

    /// Process mouse button events
    fn process_mouse_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
        x: f64,
        y: f64,
    ) -> Vec<CompositorAction> {
        self.mouse_position = (x, y);

        if pressed {
            debug!(
                "🐁 Mouse button {:?} pressed at ({:.1}, {:.1})",
                button, x, y
            );
            // Detect drag chords
            let mods = self.active_modifiers.join("+");
            let drag_move_mod = self.bindings_config.drag_move_modifier.trim();
            let drag_resize_mod = self.bindings_config.drag_resize_modifier.trim();
            if !drag_move_mod.is_empty()
                && mods.contains(drag_move_mod)
                && button == MouseButton::Left
            {
                self.drag_mode = Some(DragMode::Move);
                self.drag_start = Some((x, y));
                info!("🚚 Drag move started at ({:.1},{:.1})", x, y);
            } else if !drag_resize_mod.is_empty()
                && mods.contains(drag_resize_mod)
                && button == MouseButton::Right
            {
                self.drag_mode = Some(DragMode::Resize);
                self.drag_start = Some((x, y));
                info!("📐 Drag resize started at ({:.1},{:.1})", x, y);
            }
            if let Some(action) = self.mouse_bindings.get(&button) {
                info!("🚀 Triggered action via mouse: {:?}", action);
                return vec![action.clone()];
            }
        } else {
            // Release ends any ongoing drag
            if self.drag_mode.is_some() {
                info!("🛑 Drag ended");
                self.drag_mode = None;
                self.drag_start = None;
            }
        }

        Vec::new()
    }

    /// Process scroll events (trackpad/mouse wheel)
    fn process_scroll_event(
        &mut self,
        _x: f64,
        _y: f64,
        delta_x: f64,
        delta_y: f64,
    ) -> Vec<CompositorAction> {
        // Horizontal scrolling for workspace navigation
        if delta_x.abs() > delta_y.abs() && delta_x.abs() > self.input_config.scroll_threshold {
            debug!("📜 Horizontal scroll: {:.1}", delta_x);

            if delta_x > 0.0 {
                return vec![CompositorAction::ScrollWorkspaceRight];
            } else {
                return vec![CompositorAction::ScrollWorkspaceLeft];
            }
        }

        Vec::new()
    }

    /// Process gesture events (touchpad gestures)
    fn process_gesture_event(
        &mut self,
        gesture_type: GestureType,
        delta_x: f64,
        delta_y: f64,
        velocity: f64,
    ) -> Vec<CompositorAction> {
        match gesture_type {
            GestureType::Swipe => {
                debug!(
                    "👋 Swipe gesture: delta=({:.1}, {:.1}), velocity={:.1}",
                    delta_x, delta_y, velocity
                );

                // Horizontal swipes for workspace navigation
                if delta_x.abs() > self.input_config.swipe_threshold {
                    if delta_x > 0.0 {
                        return vec![CompositorAction::ScrollWorkspaceRight];
                    } else {
                        return vec![CompositorAction::ScrollWorkspaceLeft];
                    }
                }
            }
            GestureType::Pan => {
                // Basic horizontal pan mapping to workspace navigation
                debug!("🤏 Pan gesture: ({:.1}, {:.1})", delta_x, delta_y);
                if delta_x.abs() > self.input_config.pan_threshold {
                    if delta_x > 0.0 {
                        return vec![CompositorAction::ScrollWorkspaceRight];
                    } else {
                        return vec![CompositorAction::ScrollWorkspaceLeft];
                    }
                }
            }
            GestureType::Pinch => {
                // TODO: Implement workspace overview with pinch
                debug!("🤏 Pinch gesture: {:.1}", velocity);
            }
        }

        Vec::new()
    }

    /// Get current mouse position
    #[allow(dead_code)]
    pub fn mouse_position(&self) -> (f64, f64) {
        self.mouse_position
    }

    /// Check if a modifier is currently active
    #[allow(dead_code)]
    pub fn is_modifier_active(&self, modifier: &str) -> bool {
        self.active_modifiers.contains(&modifier.to_string())
    }

    /// Simulate input for testing
    pub fn simulate_key_press(&mut self, key_combo: &str) -> Vec<CompositorAction> {
        debug!("🧪 Simulating key press: {}", key_combo);
        if let Some(action) = self.key_bindings.get(key_combo) {
            vec![action.clone()]
        } else {
            Vec::new()
        }
    }

    pub fn shutdown(&mut self) -> Result<()> {
        info!("🔌 Input manager shutting down");
        Ok(())
    }

    /// Return keyboard repeat parameters (delay in ms, rate in Hz)
    pub fn repeat_params(&self) -> (u32, u32) {
        (
            self.input_config.keyboard_repeat_delay,
            self.input_config.keyboard_repeat_rate,
        )
    }

    /// Whether natural scrolling is enabled
    pub fn natural_scrolling(&self) -> bool {
        self.input_config.natural_scrolling
    }

    pub fn update_thresholds(&mut self, pan: Option<f64>, scroll: Option<f64>, swipe: Option<f64>) {
        if let Some(p) = pan {
            self.input_config.pan_threshold = p.clamp(0.0, 1000.0);
        }
        if let Some(s) = scroll {
            self.input_config.scroll_threshold = s.clamp(0.0, 1000.0);
        }
        if let Some(sw) = swipe {
            self.input_config.swipe_threshold = sw.clamp(0.0, 5000.0);
        }
        debug!(
            "Updated thresholds: pan={:.1}, scroll={:.1}, swipe={:.1}",
            self.input_config.pan_threshold,
            self.input_config.scroll_threshold,
            self.input_config.swipe_threshold
        );
    }

    fn parse_action_name(name: &str) -> Option<CompositorAction> {
        match name.trim().to_lowercase().as_str() {
            "scroll_left" => Some(CompositorAction::ScrollWorkspaceLeft),
            "scroll_right" => Some(CompositorAction::ScrollWorkspaceRight),
            "move_window_left" => Some(CompositorAction::MoveWindowLeft),
            "move_window_right" => Some(CompositorAction::MoveWindowRight),
            "close_window" => Some(CompositorAction::CloseWindow),
            "toggle_fullscreen" => Some(CompositorAction::ToggleFullscreen),
            "quit" => Some(CompositorAction::Quit),
            s if !s.is_empty() => Some(CompositorAction::Custom(s.to_string())),
            _ => None,
        }
    }
}
