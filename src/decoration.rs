//! Server-side decoration system for Axiom compositor
#![allow(clippy::approx_constant)]
//!
//! This module handles drawing window decorations (titlebars, borders, buttons)
//! when clients request server-side decorations (SSD).

use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;

use crate::config::WindowConfig;
use crate::effects::WindowEffectState;
use crate::window::Rectangle;

/// Default window width (pixels) used as a placeholder for button-position
/// calculations when the real window width isn't available.
/// TODO: thread real window width through from the backend / compositor.
const PLACEHOLDER_WINDOW_WIDTH: i32 = 800;

/// Decoration mode for windows
#[allow(clippy::approx_constant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationMode {
    /// Client-side decorations (app draws its own titlebar)
    ClientSide,
    /// Server-side decorations (compositor draws titlebar)
    ServerSide,
    /// No decorations at all
    None,
}

/// Per-window decoration state (mode, focus, title, titlebar buttons) plus
/// the user's preferred decoration mode.
#[derive(Debug, Clone)]
pub struct WindowDecoration {
    /// Current decoration mode for this window
    pub mode: DecorationMode,

    /// Whether the window wants server-side decorations
    pub prefers_server_side: bool,

    /// Current titlebar height (0 if no titlebar)
    pub titlebar_height: u32,

    /// Window title text
    pub title: String,

    /// Whether window has focus (affects decoration appearance)
    pub focused: bool,

    /// Button states
    pub buttons: TitlebarButtons,
}

/// Titlebar button states
#[derive(Debug, Clone, Default)]
pub struct TitlebarButtons {
    pub close: ButtonState,
    pub minimize: ButtonState,
    pub maximize: ButtonState,
}

/// Individual button state
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ButtonState {
    pub visible: bool,
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub bounds: Rectangle,
}

/// Decoration theme settings
#[derive(Debug, Clone)]
pub struct DecorationTheme {
    /// Titlebar height in pixels
    pub titlebar_height: u32,

    /// Border width for focused windows
    pub border_width_focused: u32,

    /// Border width for unfocused windows  
    pub border_width_unfocused: u32,

    /// Titlebar background color (focused)
    pub titlebar_bg_focused: [f32; 4], // RGBA

    /// Titlebar background color (unfocused)
    pub titlebar_bg_unfocused: [f32; 4], // RGBA

    /// Titlebar text color (focused)
    pub text_color_focused: [f32; 4], // RGBA

    /// Titlebar text color (unfocused)
    pub text_color_unfocused: [f32; 4], // RGBA

    /// Border color (focused)
    pub border_color_focused: [f32; 4], // RGBA

    /// Border color (unfocused)
    pub border_color_unfocused: [f32; 4], // RGBA

    /// Button size
    pub button_size: u32,

    /// Button colors
    pub button_normal: [f32; 4],
    pub button_hovered: [f32; 4],
    pub button_pressed: [f32; 4],

    /// Close button specific colors
    pub close_normal: [f32; 4],
    pub close_hovered: [f32; 4],
    pub close_pressed: [f32; 4],

    /// Corner radius for rounded decorations
    pub corner_radius: f32,

    /// Font size for title text
    pub font_size: f32,
}

/// Server-side decoration manager
#[derive(Debug)]
pub struct DecorationManager {
    /// Configuration (wired for future use; kept for config-change propagation)
    #[allow(dead_code)]
    config: WindowConfig,

    /// Theme settings
    theme: DecorationTheme,

    /// Window decoration states by window ID
    decorations: HashMap<u64, WindowDecoration>,

    /// Default decoration preferences
    default_mode: DecorationMode,
}

impl Default for ButtonState {
    fn default() -> Self {
        Self {
            visible: true,
            enabled: true,
            hovered: false,
            pressed: false,
            bounds: Rectangle {
                x: 0,
                y: 0,
                width: 24,
                height: 24,
            },
        }
    }
}

impl Default for DecorationTheme {
    fn default() -> Self {
        Self {
            titlebar_height: 32,
            border_width_focused: 2,
            border_width_unfocused: 1,
            titlebar_bg_focused: [0.15, 0.15, 0.15, 1.0], // Dark gray
            titlebar_bg_unfocused: [0.1, 0.1, 0.1, 1.0],  // Darker gray
            text_color_focused: [1.0, 1.0, 1.0, 1.0],     // White
            text_color_unfocused: [0.7, 0.7, 0.7, 1.0],   // Light gray
            border_color_focused: [0.482, 0.235, 0.929, 1.0], // Purple (#7C3AED)
            border_color_unfocused: [0.216, 0.255, 81.0 / 255.0, 1.0], // Gray (#374151)

            button_size: 24,
            button_normal: [0.2, 0.2, 0.2, 1.0],
            button_hovered: [0.3, 0.3, 0.3, 1.0],
            button_pressed: [0.1, 0.1, 0.1, 1.0],
            close_normal: [0.8, 0.2, 0.2, 1.0],  // Red
            close_hovered: [1.0, 0.3, 0.3, 1.0], // Bright red
            close_pressed: [0.6, 0.1, 0.1, 1.0], // Dark red
            corner_radius: 8.0,
            font_size: 14.0,
        }
    }
}

impl DecorationManager {
    pub fn new(config: &WindowConfig) -> Self {
        info!("🎨 Initializing server-side decoration manager...");

        // Create theme from window config
        let theme =
            DecorationTheme {
                border_width_focused: config.border_width,
                border_color_focused: Self::parse_color(&config.active_border_color)
                    .unwrap_or([0.482, 0.235, 0.929, 1.0]), // Default purple
                border_color_unfocused: Self::parse_color(&config.inactive_border_color)
                    .unwrap_or([0.216, 0.255, 81.0 / 255.0, 1.0]), // Default gray
                ..DecorationTheme::default()
            };

        info!("✅ Decoration manager initialized with theme:");
        info!("  📏 Titlebar height: {}px", theme.titlebar_height);
        info!("  🔲 Border width: {}px", theme.border_width_focused);
        info!("  🎨 Corner radius: {:.1}px", theme.corner_radius);

        Self {
            config: config.clone(),
            theme,
            decorations: HashMap::new(),
            default_mode: DecorationMode::ServerSide,
        }
    }

    /// Parse hex color string to RGBA float array
    fn parse_color(hex: &str) -> Option<[f32; 4]> {
        if !hex.starts_with('#') || hex.len() != 7 {
            return None;
        }

        let hex = &hex[1..]; // Remove #
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

        Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
    }

    /// Register a window for decoration management
    pub fn add_window(&mut self, window_id: u64, title: String, prefers_server_side: bool) {
        let mode = if prefers_server_side {
            self.default_mode
        } else {
            DecorationMode::ClientSide
        };

        let decoration = WindowDecoration {
            mode,
            prefers_server_side,
            titlebar_height: if mode == DecorationMode::ServerSide {
                self.theme.titlebar_height
            } else {
                0
            },
            title,
            focused: false,
            buttons: TitlebarButtons::default(),
        };

        // Update button positions
        let mut decoration = decoration;
        self.update_button_positions(window_id, &mut decoration);

        self.decorations.insert(window_id, decoration);

        debug!(
            "🪟 Added decoration for window {} (mode: {:?})",
            window_id, mode
        );
    }

    /// Remove window from decoration management
    pub fn remove_window(&mut self, window_id: u64) {
        if self.decorations.remove(&window_id).is_some() {
            debug!("🗑️ Removed decoration for window {}", window_id);
        }
    }

    /// Set window focus state
    pub fn set_window_focus(&mut self, window_id: u64, focused: bool) {
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            if decoration.focused != focused {
                decoration.focused = focused;
                debug!(
                    "🎯 Window {} focus: {}",
                    window_id,
                    if focused { "gained" } else { "lost" }
                );
            }
        }
    }

    /// Update window title
    pub fn set_window_title(&mut self, window_id: u64, title: String) {
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            if decoration.title != title {
                decoration.title = title;
                debug!("📝 Updated title for window {}", window_id);
            }
        }
    }

    /// Set decoration mode for a window
    pub fn set_decoration_mode(&mut self, window_id: u64, mode: DecorationMode) {
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            if decoration.mode != mode {
                decoration.mode = mode;
                decoration.titlebar_height = if mode == DecorationMode::ServerSide {
                    self.theme.titlebar_height
                } else {
                    0
                };

                info!(
                    "🎨 Changed decoration mode for window {} to {:?}",
                    window_id, mode
                );
            }
        }

        // Update button positions after releasing the mutable borrow
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            if decoration.mode == DecorationMode::ServerSide {
                let button_size = self.theme.button_size;
                let titlebar_height = self.theme.titlebar_height;
                let button_y = (titlebar_height - button_size) / 2;
                let ww = PLACEHOLDER_WINDOW_WIDTH;
                let button_margin = 8;
                decoration.buttons.close.bounds =
                    Self::button_rect(ww, button_size, button_y, button_margin, 0);
                decoration.buttons.maximize.bounds =
                    Self::button_rect(ww, button_size, button_y, button_margin, 1);
                decoration.buttons.minimize.bounds =
                    Self::button_rect(ww, button_size, button_y, button_margin, 2);
            }
        }
    }

    /// Get window decoration
    pub fn get_decoration(&self, window_id: u64) -> Option<&WindowDecoration> {
        self.decorations.get(&window_id)
    }

    /// Get mutable window decoration
    pub fn get_decoration_mut(&mut self, window_id: u64) -> Option<&mut WindowDecoration> {
        self.decorations.get_mut(&window_id)
    }

    /// Calculate the content area rectangle for a window (accounting for decorations)
    pub fn get_content_rect(&self, window_id: u64, window_rect: Rectangle) -> Rectangle {
        if let Some(decoration) = self.decorations.get(&window_id) {
            match decoration.mode {
                DecorationMode::ServerSide => {
                    let border_width = if decoration.focused {
                        self.theme.border_width_focused
                    } else {
                        self.theme.border_width_unfocused
                    } as i32;

                    Rectangle {
                        x: window_rect.x + border_width,
                        y: window_rect.y + decoration.titlebar_height as i32 + border_width,
                        width: window_rect.width.saturating_sub((border_width * 2) as u32),
                        height: window_rect
                            .height
                            .saturating_sub(decoration.titlebar_height + (border_width * 2) as u32),
                    }
                }
                _ => window_rect, // Client-side or no decorations
            }
        } else {
            window_rect
        }
    }

    /// Calculate the total window rectangle including decorations
    pub fn get_window_rect(&self, window_id: u64, content_rect: Rectangle) -> Rectangle {
        if let Some(decoration) = self.decorations.get(&window_id) {
            match decoration.mode {
                DecorationMode::ServerSide => {
                    let border_width = if decoration.focused {
                        self.theme.border_width_focused
                    } else {
                        self.theme.border_width_unfocused
                    } as i32;

                    Rectangle {
                        x: content_rect.x - border_width,
                        y: content_rect.y - decoration.titlebar_height as i32 - border_width,
                        width: content_rect.width + (border_width * 2) as u32,
                        height: content_rect.height
                            + decoration.titlebar_height
                            + (border_width * 2) as u32,
                    }
                }
                _ => content_rect, // Client-side or no decorations
            }
        } else {
            content_rect
        }
    }

    /// Handle mouse button press on decorations
    pub fn handle_button_press(
        &mut self,
        window_id: u64,
        x: i32,
        y: i32,
    ) -> Option<DecorationAction> {
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            if decoration.mode != DecorationMode::ServerSide {
                return None;
            }

            // Check if click is on titlebar buttons
            if decoration.buttons.close.bounds.contains_point(x, y) {
                decoration.buttons.close.pressed = true;
                return Some(DecorationAction::Close);
            }

            if decoration.buttons.minimize.bounds.contains_point(x, y) {
                decoration.buttons.minimize.pressed = true;
                return Some(DecorationAction::Minimize);
            }

            if decoration.buttons.maximize.bounds.contains_point(x, y) {
                decoration.buttons.maximize.pressed = true;
                return Some(DecorationAction::ToggleMaximize);
            }

            // Check if click is on titlebar (for dragging).
            // The titlebar spans the full width from x=0 to the
            // window's right edge. Since we don't have the window
            // width at this level, accept any x >= 0 as long as
            // y is within the titlebar height and x is not on a
            // button (buttons are already checked above).
            if y >= 0 && y < decoration.titlebar_height as i32 && x >= 0 {
                return Some(DecorationAction::StartMove);
            }
        }

        None
    }

    /// Handle mouse button release
    pub fn handle_button_release(&mut self, window_id: u64, _x: i32, _y: i32) {
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            decoration.buttons.close.pressed = false;
            decoration.buttons.minimize.pressed = false;
            decoration.buttons.maximize.pressed = false;
        }
    }

    /// Handle mouse movement for hover effects
    pub fn handle_mouse_motion(&mut self, window_id: u64, x: i32, y: i32) {
        if let Some(decoration) = self.decorations.get_mut(&window_id) {
            // Update button hover states
            decoration.buttons.close.hovered = decoration.buttons.close.bounds.contains_point(x, y);
            decoration.buttons.minimize.hovered =
                decoration.buttons.minimize.bounds.contains_point(x, y);
            decoration.buttons.maximize.hovered =
                decoration.buttons.maximize.bounds.contains_point(x, y);
        }
    }

    /// Update button positions based on window size and theme
    fn update_button_positions(&self, _window_id: u64, decoration: &mut WindowDecoration) {
        if decoration.mode != DecorationMode::ServerSide {
            return;
        }

        let button_size = self.theme.button_size;
        let titlebar_height = self.theme.titlebar_height;
        let button_y = (titlebar_height - button_size) / 2;
        let ww = PLACEHOLDER_WINDOW_WIDTH;
        let button_margin = 8;
        decoration.buttons.close.bounds =
            Self::button_rect(ww, button_size, button_y, button_margin, 0);
        decoration.buttons.maximize.bounds =
            Self::button_rect(ww, button_size, button_y, button_margin, 1);
        decoration.buttons.minimize.bounds =
            Self::button_rect(ww, button_size, button_y, button_margin, 2);
    }

    /// Get the current theme
    pub fn theme(&self) -> &DecorationTheme {
        &self.theme
    }

    /// Update theme settings
    pub fn update_theme(&mut self, theme: DecorationTheme) {
        self.theme = theme;
        info!("🎨 Updated decoration theme");
        let button_size = self.theme.button_size;
        let titlebar_height = self.theme.titlebar_height;
        let button_y = (titlebar_height - button_size) / 2;
        let ww = PLACEHOLDER_WINDOW_WIDTH;
        let button_margin = 8;

        // Update all window button positions
        let window_ids: Vec<u64> = self.decorations.keys().copied().collect();
        for window_id in window_ids {
            if let Some(decoration) = self.decorations.get_mut(&window_id) {
                if decoration.mode == DecorationMode::ServerSide {
                    decoration.buttons.close.bounds =
                        Self::button_rect(ww, button_size, button_y, button_margin, 0);
                    decoration.buttons.maximize.bounds =
                        Self::button_rect(ww, button_size, button_y, button_margin, 1);
                    decoration.buttons.minimize.bounds =
                        Self::button_rect(ww, button_size, button_y, button_margin, 2);
                }
            }
        }
    }

    /// Helper: compute button rectangle at position `idx` (0 = close, 1 =
    /// maximize, 2 = minimize) from the right edge.
    fn button_rect(window_w: i32, size: u32, y: u32, margin: i32, idx: usize) -> Rectangle {
        Rectangle {
            x: window_w - (size as i32 + margin) * (idx as i32 + 1),
            y: y as i32,
            width: size,
            height: size,
        }
    }

    /// Render window decorations (placeholder for GPU implementation)
    pub fn render_decorations(
        &self,
        window_id: u64,
        window_rect: Rectangle,
        effects: Option<&WindowEffectState>,
    ) -> Result<DecorationRenderData> {
        let decoration = self.decorations.get(&window_id).ok_or_else(|| {
            anyhow::anyhow!("Window {} not found in decoration manager", window_id)
        })?;

        if decoration.mode != DecorationMode::ServerSide {
            return Ok(DecorationRenderData::None);
        }

        // Generate rendering commands for GPU pipeline
        let border_width = if decoration.focused {
            self.theme.border_width_focused
        } else {
            self.theme.border_width_unfocused
        };

        let border_color = if decoration.focused {
            self.theme.border_color_focused
        } else {
            self.theme.border_color_unfocused
        };

        let titlebar_bg = if decoration.focused {
            self.theme.titlebar_bg_focused
        } else {
            self.theme.titlebar_bg_unfocused
        };

        let text_color = if decoration.focused {
            self.theme.text_color_focused
        } else {
            self.theme.text_color_unfocused
        };

        // Apply effects if available
        let mut opacity = 1.0;
        let mut corner_radius = self.theme.corner_radius;

        if let Some(effects) = effects {
            opacity *= effects.opacity;
            corner_radius = effects.corner_radius;
        }

        let render_data = DecorationRenderData::ServerSide {
            titlebar_rect: Rectangle {
                x: window_rect.x,
                y: window_rect.y,
                width: window_rect.width,
                height: decoration.titlebar_height,
            },
            titlebar_bg: [
                titlebar_bg[0],
                titlebar_bg[1],
                titlebar_bg[2],
                titlebar_bg[3] * opacity,
            ],
            border_width,
            border_color: [
                border_color[0],
                border_color[1],
                border_color[2],
                border_color[3] * opacity,
            ],
            corner_radius,
            title: decoration.title.clone(),
            text_color: [
                text_color[0],
                text_color[1],
                text_color[2],
                text_color[3] * opacity,
            ],
            font_size: self.theme.font_size,
            buttons: decoration.buttons.clone(),
        };

        debug!(
            "🎨 Generated decoration render data for window {}",
            window_id
        );

        Ok(render_data)
    }
}

/// Actions that can be triggered by decoration interactions
#[derive(Debug, Clone, PartialEq)]
pub enum DecorationAction {
    Close,
    Minimize,
    ToggleMaximize,
    StartMove,
    StartResize(ResizeEdge),
}

/// Resize edge identification
#[derive(Debug, Clone, PartialEq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Decoration rendering data for GPU pipeline
#[derive(Debug, Clone)]
pub enum DecorationRenderData {
    None,
    ServerSide {
        titlebar_rect: Rectangle,
        titlebar_bg: [f32; 4],
        border_width: u32,
        border_color: [f32; 4],
        corner_radius: f32,
        title: String,
        text_color: [f32; 4],
        font_size: f32,
        buttons: TitlebarButtons,
    },
}

// Helper trait for Rectangle
impl Rectangle {
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WindowConfig;

    #[test]
    fn test_decoration_manager_initialization() {
        let mgr = DecorationManager::new(&WindowConfig::default());
        assert_eq!(mgr.default_mode, DecorationMode::ServerSide);
        assert!(mgr.theme().corner_radius > 0.0);
    }

    #[test]
    fn test_add_and_remove_window() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.add_window(1, "Test".into(), true);
        assert!(mgr.get_decoration(1).is_some());
        assert_eq!(mgr.get_decoration(1).unwrap().title, "Test");

        mgr.remove_window(1);
        assert!(mgr.get_decoration(1).is_none());
    }

    #[test]
    fn test_set_window_focus_flips() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.add_window(7, "X".into(), true);
        assert!(!mgr.get_decoration(7).unwrap().focused);
        mgr.set_window_focus(7, true);
        assert!(mgr.get_decoration(7).unwrap().focused);
        mgr.set_window_focus(7, false);
        assert!(!mgr.get_decoration(7).unwrap().focused);
    }

    #[test]
    fn test_set_window_focus_unknown_noop() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.set_window_focus(999, true); // shouldn't panic
    }

    #[test]
    fn test_set_window_title_updates() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.add_window(1, "Old".into(), true);
        mgr.set_window_title(1, "New".into());
        assert_eq!(mgr.get_decoration(1).unwrap().title, "New");
    }

    #[test]
    fn test_parse_color_valid_hex() {
        let c = DecorationManager::parse_color("#FFAA33").unwrap();
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert!((c[1] - (0xAA as f32 / 255.0)).abs() < 1e-6);
        assert!((c[2] - (0x33 as f32 / 255.0)).abs() < 1e-6);
        assert!((c[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_color_rejects_invalid() {
        assert!(DecorationManager::parse_color("FFAA33").is_none()); // no '#'
        assert!(DecorationManager::parse_color("#FFF").is_none()); // wrong length
        assert!(DecorationManager::parse_color("#ZZZZZZ").is_none()); // not hex
        assert!(DecorationManager::parse_color("").is_none());
    }

    #[test]
    fn test_client_side_decoration_skips_titlebar() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        // prefers_server_side=false => ClientSide => no titlebar
        mgr.add_window(1, "CSD".into(), false);
        assert_eq!(
            mgr.get_decoration(1).unwrap().mode,
            DecorationMode::ClientSide
        );
        assert_eq!(mgr.get_decoration(1).unwrap().titlebar_height, 0);
    }

    #[test]
    fn test_button_press_in_titlebar_returns_start_move() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.add_window(1, "T".into(), true);
        // titlebar_rect has width 1000 in helper code for now;
        // a click at (10, 5) is well inside the titlebar (height default = 32)
        let action = mgr.handle_button_press(1, 10, 5);
        assert_eq!(action, Some(DecorationAction::StartMove));
    }

    #[test]
    fn test_button_press_outside_returns_none() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.add_window(1, "T".into(), true);
        // y=500 is well below the 32-pixel titlebar
        let action = mgr.handle_button_press(1, 10, 500);
        assert!(action.is_none());
    }

    #[test]
    fn test_button_press_then_release_clears_pressed() {
        let mut mgr = DecorationManager::new(&WindowConfig::default());
        mgr.add_window(1, "T".into(), true);
        // Baseline: nothing is pressed.
        assert!(!mgr.get_decoration(1).unwrap().buttons.close.pressed);
        // The titlebar rect in handle_button_press is hardcoded width=1000,
        // height=titlebar_height (32). A click at (10, 5) is inside the
        // titlebar and outside any button bounds (which start at x≈704).
        let _action = mgr.handle_button_press(1, 10, 5);
        // Trigger a button-press by clicking on the close button bounds
        // (close.bounds.x = 800 - 24 - 8 = 768 in update_button_positions).
        let close_action = mgr.handle_button_press(1, 770, 12);
        assert_eq!(close_action, Some(DecorationAction::Close));
        assert!(mgr.get_decoration(1).unwrap().buttons.close.pressed);
        // Releasing must clear the pressed flag.
        mgr.handle_button_release(1, 770, 12);
        assert!(!mgr.get_decoration(1).unwrap().buttons.close.pressed);
    }

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
}
