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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct AxiomConfig {
    /// Workspace configuration (scrollable behavior)
    pub workspace: WorkspaceConfig,

    /// Visual effects configuration  
    pub effects: EffectsConfig,

    /// Window management settings
    pub window: WindowConfig,

    /// Input handling and keybindings
    pub input: InputConfig,

    /// Key bindings
    pub bindings: BindingsConfig,

    /// XWayland configuration
    pub xwayland: XWaylandConfig,

    /// General compositor settings
    pub general: GeneralConfig,
}

/// Scrollable workspace configuration (niri-inspired)
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Visual effects configuration (Hyprland-inspired)
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundedCornersConfig {
    /// Enable rounded corners
    pub enabled: bool,

    /// Corner radius (pixels)
    pub radius: u32,

    /// Anti-aliasing quality (1-4)
    pub antialiasing: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Input configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Key bindings configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Launch terminal
    pub launch_terminal: String,

    /// Launch application launcher
    pub launch_launcher: String,

    /// Toggle effects
    pub toggle_effects: String,

    /// Quit compositor
    pub quit: String,
}

/// XWayland configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XWaylandConfig {
    /// Enable XWayland support
    pub enabled: bool,

    /// XWayland display number
    pub display: Option<u32>,
}

/// General compositor settings
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
    }
}

impl Default for BindingsConfig {
    fn default() -> Self {
        Self {
            scroll_left: "Super_L+Left".to_string(),
            scroll_right: "Super_L+Right".to_string(),
            move_window_left: "Super_L+Shift+Left".to_string(),
            move_window_right: "Super_L+Shift+Right".to_string(),
            close_window: "Super_L+q".to_string(),
            launch_terminal: "Super_L+Return".to_string(),
            launch_launcher: "Super_L+d".to_string(),
            toggle_effects: "Super_L+e".to_string(),
            quit: "Super_L+Shift+q".to_string(),
        }
    }
}

impl Default for XWaylandConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            display: None, // Auto-select
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            debug: false,
            max_fps: 60,
            vsync: true,
        }
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
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let contents = toml::to_string_pretty(self).context("Failed to serialize configuration")?;

        fs::write(path, contents).context("Failed to write configuration file")?;

        Ok(())
    }
}
