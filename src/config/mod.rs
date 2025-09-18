//! Configuration management for Axiom
//!
//! This module handles loading, parsing, and validating configuration
//! from TOML files. It combines settings for workspaces, effects,
//! input handling, and more.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Main configuration struct containing all Axiom settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AxiomConfig {
    /// Workspace configuration (scrollable behavior)
    #[serde(default)]
    pub workspace: WorkspaceConfig,

    /// Visual effects configuration  
    #[serde(default)]
    pub effects: EffectsConfig,

    /// Window management settings
    #[serde(default)]
    pub window: WindowConfig,

    /// Input handling and keybindings
    #[serde(default)]
    pub input: InputConfig,

    /// Key bindings
    #[serde(default)]
    pub bindings: BindingsConfig,

    /// XWayland configuration
    #[serde(default)]
    pub xwayland: XWaylandConfig,

    /// General compositor settings
    #[serde(default)]
    pub general: GeneralConfig,
}

/// Scrollable workspace configuration (niri-inspired)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceConfig {
    /// Speed of workspace scrolling (1.0 = normal)
    pub scroll_speed: f64,

    /// Enable infinite scrolling (vs bounded workspaces)
    pub infinite_scroll: bool,

    /// Auto-scroll to fit content
    pub auto_scroll: bool,

    /// Width of each virtual workspace column (pixels)
    pub workspace_width: u32,

    /// Gaps between windows (pixels)
    pub gaps: u32,

    /// Enable smooth scrolling animations
    pub smooth_scrolling: bool,

    /// Momentum friction factor (0.0-1.0, closer to 0 = fast decay, closer to 1 = slow decay)
    #[serde(default = "WorkspaceConfig::default_momentum_friction")]
    pub momentum_friction: f64,

    /// Minimum velocity to keep momentum scrolling (px/s)
    #[serde(default = "WorkspaceConfig::default_momentum_min_velocity")]
    pub momentum_min_velocity: f64,

    /// Snap-to-column distance threshold in pixels
    #[serde(default = "WorkspaceConfig::default_snap_threshold")]
    pub snap_threshold_px: f64,
}

/// Visual effects configuration (Hyprland-inspired)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectsConfig {
    /// Enable/disable all visual effects
    pub enabled: bool,

    /// Animation settings
    pub animations: AnimationConfig,

    /// Blur effect settings
    pub blur: BlurConfig,

    /// Rounded corners settings
    pub rounded_corners: RoundedCornersConfig,

    /// Drop shadow settings
    pub shadows: ShadowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnimationConfig {
    /// Enable animations
    pub enabled: bool,

    /// Default animation duration (milliseconds)
    pub duration: u32,

    /// Animation curve ("linear", "ease", "ease-in", "ease-out", "ease-in-out")
    pub curve: String,

    /// Workspace transition animation duration
    pub workspace_transition: u32,

    /// Window open/close animation duration
    pub window_animation: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlurConfig {
    /// Enable blur effects
    pub enabled: bool,

    /// Blur radius (pixels)
    pub radius: u32,

    /// Blur intensity (0.0-1.0)
    pub intensity: f64,

    /// Enable blur on window backgrounds
    pub window_backgrounds: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundedCornersConfig {
    /// Enable rounded corners
    pub enabled: bool,

    /// Corner radius (pixels)
    pub radius: u32,

    /// Anti-aliasing quality (1-4)
    pub antialiasing: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowConfig {
    /// Enable drop shadows
    pub enabled: bool,

    /// Shadow size (pixels)
    pub size: u32,

    /// Shadow blur radius
    pub blur_radius: u32,

    /// Shadow opacity (0.0-1.0)
    pub opacity: f64,

    /// Shadow color (hex: #RRGGBB)
    pub color: String,
}

/// Window management configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowConfig {
    /// Default window placement algorithm
    pub placement: String, // "smart", "center", "mouse"

    /// Focus follows mouse
    pub focus_follows_mouse: bool,

    /// Border width (pixels)
    pub border_width: u32,

    /// Active border color
    pub active_border_color: String,

    /// Inactive border color  
    pub inactive_border_color: String,

    /// Gap between windows (pixels)
    pub gap: u32,

    /// Default layout algorithm ("horizontal", "vertical")
    pub default_layout: String,

    /// Force client-side decorations (CSD) for all clients.
    /// When true, the compositor will always configure zxdg_toplevel_decoration_v1
    /// to client-side. When false, the compositor will honor client requests and
    /// prefer server-side decorations by default.
    #[serde(default)]
    pub force_client_side_decorations: bool,
}

/// Input configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InputConfig {
    /// Keyboard repeat delay (milliseconds)
    pub keyboard_repeat_delay: u32,

    /// Keyboard repeat rate (per second)
    pub keyboard_repeat_rate: u32,

    /// Mouse acceleration
    pub mouse_accel: f64,

    /// Touchpad tap to click
    pub touchpad_tap: bool,

    /// Natural scrolling
    pub natural_scrolling: bool,

    /// Gesture threshold for horizontal pan (px)
    #[serde(default = "InputConfig::default_pan_threshold")]
    pub pan_threshold: f64,

    /// Scroll threshold for horizontal scroll (px)
    #[serde(default = "InputConfig::default_scroll_threshold")]
    pub scroll_threshold: f64,

    /// Swipe threshold (px)
    #[serde(default = "InputConfig::default_swipe_threshold")]
    pub swipe_threshold: f64,

    /// Drag threshold (px) for chorded drag actions
    #[serde(default = "InputConfig::default_drag_threshold")]
    pub drag_threshold: f64,
}

/// Key bindings configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BindingsConfig {
    /// Scroll workspace left
    pub scroll_left: String,

    /// Scroll workspace right
    pub scroll_right: String,

    /// Move window left
    pub move_window_left: String,

    /// Move window right
    pub move_window_right: String,

    /// Close window
    pub close_window: String,

    /// Toggle fullscreen for focused window
    pub toggle_fullscreen: String,

    /// Launch terminal
    pub launch_terminal: String,

    /// Launch application launcher
    pub launch_launcher: String,

    /// Toggle effects
    pub toggle_effects: String,

    /// Quit compositor
    pub quit: String,

    /// Mouse bindings: left button action name (e.g., "toggle_fullscreen")
    #[serde(default)]
    pub mouse_left: String,
    /// Mouse bindings: right button action name
    #[serde(default)]
    pub mouse_right: String,
    /// Mouse bindings: middle button action name
    #[serde(default)]
    pub mouse_middle: String,

    /// Drag move chord modifier(s), e.g., "Super" or "Super+Shift"
    #[serde(default)]
    pub drag_move_modifier: String,
    /// Drag resize chord modifier(s)
    #[serde(default)]
    pub drag_resize_modifier: String,
}

/// XWayland configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct XWaylandConfig {
    /// Enable XWayland support
    pub enabled: bool,

    /// XWayland display number
    pub display: Option<u32>,
}

/// General compositor settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneralConfig {
    /// Enable debug logging
    pub debug: bool,

    /// Max FPS limit (0 = unlimited)
    pub max_fps: u32,

    /// Enable VSync
    pub vsync: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            scroll_speed: 1.0,
            infinite_scroll: true,
            auto_scroll: true,
            workspace_width: 1920,
            gaps: 10,
            smooth_scrolling: true,
            momentum_friction: Self::default_momentum_friction(),
            momentum_min_velocity: Self::default_momentum_min_velocity(),
            snap_threshold_px: Self::default_snap_threshold(),
        }
    }
}

impl Default for EffectsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            animations: AnimationConfig::default(),
            blur: BlurConfig::default(),
            rounded_corners: RoundedCornersConfig::default(),
            shadows: ShadowConfig::default(),
        }
    }
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            duration: 300,
            curve: "ease-out".to_string(),
            workspace_transition: 250,
            window_animation: 200,
        }
    }
}

impl Default for BlurConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 10,
            intensity: 0.8,
            window_backgrounds: true,
        }
    }
}

impl Default for RoundedCornersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 8,
            antialiasing: 2,
        }
    }
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            size: 20,
            blur_radius: 15,
            opacity: 0.6,
            color: "#000000".to_string(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            placement: "smart".to_string(),
            focus_follows_mouse: false,
            border_width: 2,
            active_border_color: "#7C3AED".to_string(), // Purple
            inactive_border_color: "#374151".to_string(), // Gray
            gap: 10,
            default_layout: "horizontal".to_string(),
            force_client_side_decorations: false,
        }
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            keyboard_repeat_delay: 600,
            keyboard_repeat_rate: 25,
            mouse_accel: 0.0,
            touchpad_tap: true,
            natural_scrolling: true,
            pan_threshold: Self::default_pan_threshold(),
            scroll_threshold: Self::default_scroll_threshold(),
            swipe_threshold: Self::default_swipe_threshold(),
            drag_threshold: Self::default_drag_threshold(),
        }
    }
}

impl Default for BindingsConfig {
    fn default() -> Self {
        Self {
            scroll_left: "Super+Left".to_string(),
            scroll_right: "Super+Right".to_string(),
            move_window_left: "Super+Shift+Left".to_string(),
            move_window_right: "Super+Shift+Right".to_string(),
            close_window: "Super+q".to_string(),
            toggle_fullscreen: "Super+f".to_string(),
            launch_terminal: "Super+Enter".to_string(),
            launch_launcher: "Super+Space".to_string(),
            toggle_effects: "Super+e".to_string(),
            quit: "Super+Shift+q".to_string(),
            mouse_left: String::new(),
            mouse_right: String::new(),
            mouse_middle: String::new(),
            drag_move_modifier: String::from("Super"),
            drag_resize_modifier: String::new(),
        }
    }
}

impl InputConfig {
    fn default_pan_threshold() -> f64 {
        10.0
    }
    fn default_scroll_threshold() -> f64 {
        5.0
    }
    fn default_swipe_threshold() -> f64 {
        20.0
    }
    fn default_drag_threshold() -> f64 {
        12.0
    }
}

impl Default for XWaylandConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            display: None,
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            debug: false,
            max_fps: 0,
            vsync: true,
        }
    }
}

impl WorkspaceConfig {
    fn default_momentum_friction() -> f64 {
        0.95
    }
    fn default_momentum_min_velocity() -> f64 {
        1.0
    }
    fn default_snap_threshold() -> f64 {
        48.0
    }
}

impl AxiomConfig {
    /// Load configuration from a TOML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // Expand ~ to home directory
        let expanded_path = if path.to_string_lossy().starts_with('~') {
            let home = std::env::var("HOME").context("Failed to get HOME environment variable")?;
            Path::new(&home).join(path.strip_prefix("~").unwrap_or(path))
        } else {
            path.to_path_buf()
        };

        let contents = fs::read_to_string(&expanded_path)
            .with_context(|| format!("Failed to read config file: {}", expanded_path.display()))?;

        let config: AxiomConfig = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", expanded_path.display()))?;

        config.validate()?;

        Ok(config)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate scroll speed
        if self.workspace.scroll_speed <= 0.0 || self.workspace.scroll_speed > 10.0 {
            anyhow::bail!("Invalid scroll_speed: must be between 0.0 and 10.0");
        }

        // Validate animation curve
        let valid_curves = ["linear", "ease", "ease-in", "ease-out", "ease-in-out"];
        if !valid_curves.contains(&self.effects.animations.curve.as_str()) {
            anyhow::bail!("Invalid animation curve: {}", self.effects.animations.curve);
        }

        // Validate blur intensity
        if self.effects.blur.intensity < 0.0 || self.effects.blur.intensity > 1.0 {
            anyhow::bail!("Invalid blur intensity: must be between 0.0 and 1.0");
        }

        // Validate shadow opacity
        if self.effects.shadows.opacity < 0.0 || self.effects.shadows.opacity > 1.0 {
            anyhow::bail!("Invalid shadow opacity: must be between 0.0 and 1.0");
        }

        Ok(())
    }

    /// Save configuration to a TOML file
    #[allow(dead_code)]
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let contents = toml::to_string_pretty(self).context("Failed to serialize configuration")?;

        fs::write(path, contents).context("Failed to write configuration file")?;

        Ok(())
    }

    /// Merge a partial configuration into this one
    /// Non-default values from the partial config will override this config
    #[allow(dead_code)]
    pub fn merge_partial(mut self, partial: AxiomConfig) -> Self {
        let default_config = AxiomConfig::default();

        // Helper to decide if a section in partial differs from default (meaningfully provided)
        let workspace_changed = partial.workspace != default_config.workspace;
        let effects_changed = partial.effects != default_config.effects;
        let window_changed = partial.window != default_config.window;
        let input_changed = partial.input != default_config.input;
        let bindings_changed = partial.bindings != default_config.bindings;
        let xwayland_changed = partial.xwayland != default_config.xwayland;
        let general_changed = partial.general != default_config.general;

        if workspace_changed {
            self.workspace = partial.workspace;
        }
        if effects_changed {
            self.effects = partial.effects;
        }
        if window_changed {
            self.window = partial.window;
        }
        if input_changed {
            self.input = partial.input;
        }
        if bindings_changed {
            self.bindings = partial.bindings;
        }
        if xwayland_changed {
            self.xwayland = partial.xwayland;
        }
        if general_changed {
            self.general = partial.general;
        }

        self
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;
