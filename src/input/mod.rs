//! Phase 3: Enhanced input handling and key bindings
//! Manages keyboard, mouse, and gesture input with real processing

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

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
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
    input_config: InputConfig,
    bindings_config: BindingsConfig,

    /// Key binding mappings
    key_bindings: HashMap<String, CompositorAction>,

    /// Current modifier state
    active_modifiers: Vec<String>,

    /// Mouse state
    mouse_position: (f64, f64),

    /// Gesture state for momentum scrolling
    gesture_state: Option<GestureState>,
}

#[derive(Debug, Clone)]
struct GestureState {
    start_time: std::time::Instant,
    start_position: (f64, f64),
    current_velocity: (f64, f64),
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

        debug!("🔑 Loaded {} key bindings", key_bindings.len());

        Ok(Self {
            input_config: input_config.clone(),
            bindings_config: bindings_config.clone(),
            key_bindings,
            active_modifiers: Vec::new(),
            mouse_position: (0.0, 0.0),
            gesture_state: None,
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
                self.mouse_position = (x, y);
                Vec::new() // No actions for simple mouse movement
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
            // TODO: Add mouse button bindings
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
        if delta_x.abs() > delta_y.abs() && delta_x.abs() > 5.0 {
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
                if delta_x.abs() > 20.0 {
                    if delta_x > 0.0 {
                        return vec![CompositorAction::ScrollWorkspaceRight];
                    } else {
                        return vec![CompositorAction::ScrollWorkspaceLeft];
                    }
                }
            }
            GestureType::Pan => {
                // TODO: Implement smooth scrolling with pan gestures
                debug!("🤏 Pan gesture: ({:.1}, {:.1})", delta_x, delta_y);
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
}
