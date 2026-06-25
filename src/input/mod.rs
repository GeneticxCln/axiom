//! Input handling and key bindings
//!
//! Manages keyboard, mouse, and gesture input with real processing.
//! Translates raw input events into compositor actions via configurable
//! key binding mappings.

use crate::config::{BindingsConfig, InputConfig};
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

/// Mouse button identifiers
#[derive(Debug, Clone, PartialEq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u8),
}

/// Touch gesture types
#[derive(Debug, Clone, PartialEq)]
pub enum GestureType {
    Swipe,
    Pinch,
    Pan,
}

/// Represents compositor actions that can be triggered by input
/// Actions triggered by input events
#[derive(Debug, Clone, PartialEq)]
pub enum CompositorAction {
    ScrollWorkspaceLeft,
    ScrollWorkspaceRight,
    MoveWindowLeft,
    MoveWindowRight,
    CloseWindow,
    ToggleFullscreen,
    Quit,
    Custom(String),
}

/// Processes input events and maps them to compositor actions
#[derive(Debug)]
pub struct InputManager {
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
    #[allow(dead_code)]
    start_time: std::time::Instant,
    #[allow(dead_code)]
    start_position: (f64, f64),
    #[allow(dead_code)]
    current_velocity: (f64, f64),
}

impl InputManager {
    pub fn new(_input_config: &InputConfig, bindings_config: &BindingsConfig) -> Self {
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

        Self {
            key_bindings,
            active_modifiers: Vec::new(),
            mouse_position: (0.0, 0.0),
            gesture_state: None,
        }
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
            // TODO: Add mouse button bindings from config
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
            }
            return vec![CompositorAction::ScrollWorkspaceLeft];
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
                    }
                    return vec![CompositorAction::ScrollWorkspaceLeft];
                }
            }
            GestureType::Pan => {
                // Smooth scrolling with pan gestures
                debug!("🤏 Pan gesture: ({:.1}, {:.1})", delta_x, delta_y);

                // Track gesture state for momentum
                let now = std::time::Instant::now();
                self.gesture_state = Some(GestureState {
                    start_time: now,
                    start_position: self.mouse_position,
                    current_velocity: (delta_x, delta_y),
                });

                // Horizontal pan for workspace navigation
                if delta_x.abs() > 10.0 {
                    if delta_x > 0.0 {
                        return vec![CompositorAction::ScrollWorkspaceRight];
                    }
                    return vec![CompositorAction::ScrollWorkspaceLeft];
                }
            }
            GestureType::Pinch => {
                // Workspace overview with pinch gesture
                debug!("🤏 Pinch gesture: velocity={:.1}", velocity);

                // Pinch-in (negative velocity) could trigger workspace overview
                // Pinch-out (positive zoom) could reset view
                // For now, log the gesture for future implementation
            }
        }

        Vec::new()
    }

    /// Get current mouse position
    pub fn mouse_position(&self) -> (f64, f64) {
        self.mouse_position
    }

    /// Check if a modifier is currently active
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

    pub fn shutdown(&mut self) {
        info!("🔌 Input manager shutting down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BindingsConfig, InputConfig};

    fn make_configs() -> (InputConfig, BindingsConfig) {
        let input = InputConfig::default();
        let bindings = BindingsConfig::default();
        (input, bindings)
    }

    #[test]
    fn test_input_manager_initialization() {
        let (input_cfg, bindings_cfg) = make_configs();
        let manager = InputManager::new(&input_cfg, &bindings_cfg);
        assert_eq!(manager.mouse_position(), (0.0, 0.0));
    }

    #[test]
    fn test_simulate_key_press_known_binding() {
        let (input_cfg, bindings_cfg) = make_configs();
        let mut manager = InputManager::new(&input_cfg, &bindings_cfg);
        // The default quit binding should work
        let actions = manager.simulate_key_press(&bindings_cfg.quit);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], CompositorAction::Quit);
    }

    #[test]
    fn test_simulate_key_press_unknown_binding() {
        let (input_cfg, bindings_cfg) = make_configs();
        let mut manager = InputManager::new(&input_cfg, &bindings_cfg);
        let actions = manager.simulate_key_press("unknown+key+binding");
        assert!(actions.is_empty());
    }

    #[test]
    fn test_scroll_navigation() {
        let (input_cfg, bindings_cfg) = make_configs();
        let mut manager = InputManager::new(&input_cfg, &bindings_cfg);

        // Large scroll right should trigger workspace scroll right
        let actions = manager.process_input_event(InputEvent::Scroll {
            x: 100.0,
            y: 100.0,
            delta_x: 20.0,
            delta_y: 0.0,
        });
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], CompositorAction::ScrollWorkspaceRight);

        // Large scroll left should trigger workspace scroll left
        let actions = manager.process_input_event(InputEvent::Scroll {
            x: 100.0,
            y: 100.0,
            delta_x: -20.0,
            delta_y: 0.0,
        });
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], CompositorAction::ScrollWorkspaceLeft);
    }

    #[test]
    fn test_scroll_no_action_small() {
        let (input_cfg, bindings_cfg) = make_configs();
        let mut manager = InputManager::new(&input_cfg, &bindings_cfg);

        // Small scroll should not trigger any action
        let actions = manager.process_input_event(InputEvent::Scroll {
            x: 100.0,
            y: 100.0,
            delta_x: 2.0,
            delta_y: 0.0,
        });
        assert!(actions.is_empty());
    }

    #[test]
    fn test_keyboard_event_modifiers() {
        let (input_cfg, bindings_cfg) = make_configs();
        let mut manager = InputManager::new(&input_cfg, &bindings_cfg);

        // Press Super key
        let _actions = manager.process_input_event(InputEvent::Keyboard {
            key: "Super_L".into(),
            modifiers: vec!["Super".into()],
            pressed: true,
        });
        // Super key alone might not have a binding, but should track modifiers
        assert!(manager.is_modifier_active("Super"));

        // Release (must include the modifier being released)
        let _ = manager.process_input_event(InputEvent::Keyboard {
            key: "Super_L".into(),
            modifiers: vec!["Super".into()],
            pressed: false,
        });
        assert!(!manager.is_modifier_active("Super"));
    }

    #[test]
    fn test_shutdown() {
        let (input_cfg, bindings_cfg) = make_configs();
        let mut manager = InputManager::new(&input_cfg, &bindings_cfg);
        manager.shutdown();
    }
}
