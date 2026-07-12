//! Configuration management for Axiom
//!
//! This module handles loading, parsing, and validating configuration
//! from TOML files. It combines settings for workspaces, effects,
//! input handling, and more.
//!
//! The configuration is composed of several sections:
//! - [`WorkspaceConfig`]: Scrollable workspace behavior
//! - [`EffectsConfig`]: Visual effects and animations
//! - [`WindowConfig`]: Window management and placement
//! - [`InputConfig`]: Input device handling
//! - [`BindingsConfig`]: Key binding mappings
//! - [`XWaylandConfig`]: X11 compatibility settings
//! - [`GeneralConfig`]: Global compositor settings

use anyhow::{Context, Result};
use log::warn;
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

    /// Backend selection (winit / drm / noop). Default is `winit` for
    /// development; production users override via `--backend=drm`.
    /// Stored as `String` here so the config schema is self-contained
    /// and parses without pulling in the backend module.
    #[serde(default)]
    pub backend: BackendConfig,

    /// Feature kill-switches for features we deliberately *minimize* to
    /// keep the implementation surface focused. Both are `false` by
    /// default so out-of-the-box Axiom does NOT:
    /// 1. Draw / handle a minimize button on the titlebar (there is no
    ///    Wayland minimize protocol per se — this would require building
    ///    a compositor-internal iconified-window list + a round-trip
    ///    with a synthetic Wayland surface to notify clients, which is
    ///    deeper protocol work than the current milestone aims for).
    /// 2. Register the `zxdg_decoration_manager_v1` global for clients
    ///    to negotiate SSD↔CSD preference with us. The live compositor
    ///    output still does **not** render visible SSD chrome yet, so
    ///    the current handler negotiates **client-side decorations**
    ///    even when the protocol global is enabled.
    ///
    /// Users can opt back into either independently by setting the
    /// matching flag to `true` (and then supplying the corresponding
    /// implementation when wiring time comes).
    #[serde(default)]
    pub features: FeaturesConfig,

    /// General compositor settings
    #[serde(default)]
    pub general: GeneralConfig,
}

/// Feature kill-switches. Both flags default to `false` — see the
/// [`AxiomConfig::features`] field for the rationale. The fields are
/// `pub` so anyone reading the config directly can see the public
/// surface; the helpers below just exist for `#[serde(default = ...)]`
/// to point at so TOML deserialization works without a `[features]`
/// header at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeaturesConfig {
    /// Enable the titlebar's minimize button + `DecorationAction::Minimize`
    /// event handling. Disabled by default.
    #[serde(default = "FeaturesConfig::default_enable_minimize")]
    pub enable_minimize: bool,

    /// Enable the `xdg-decoration-unstable-v1` Wayland protocol global so
    /// clients can negotiate SSD/CSD with the compositor. Disabled by
    /// default. When enabled today, the compositor still negotiates
    /// **client-side decorations** because visible SSD rendering is not
    /// part of the live output path yet.
    #[serde(default = "FeaturesConfig::default_enable_xdg_decoration_protocol")]
    pub enable_xdg_decoration_protocol: bool,
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            enable_minimize: Self::default_enable_minimize(),
            enable_xdg_decoration_protocol: Self::default_enable_xdg_decoration_protocol(),
        }
    }
}

impl FeaturesConfig {
    /// Serde default accessor so `[features]` can be omitted entirely.
    /// Kept as a static method to match the existing accessors on the
    /// other config substructs (see [`WorkspaceConfig::default_momentum_friction`]).
    fn default_enable_minimize() -> bool {
        false
    }
    fn default_enable_xdg_decoration_protocol() -> bool {
        false
    }
}

/// Backend selection section of [`AxiomConfig`].
///
/// The `kind` field accepts `"winit"`, `"drm"`, or `"noop"` (plus
/// `from_config_str` aliases like `"kms"`, `"tty"`, `"windowed"`, etc.).
/// Unknown values fall back to `winit` and emit a warning so a typo
/// never bricks startup. See [`BackendKind`](crate::backend::BackendKind)
/// for the parsed enum used by the rest of the compositor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackendConfig {
    /// Backend kind name. See [`BackendConfig::default`] for valid values.
    pub kind: String,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: "winit".to_string(),
        }
    }
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

    /// Gap between windows (pixels).
    /// Deprecated: use `workspace.gaps` instead. This field is accepted
    /// for backward compatibility but does not affect layout.
    pub gap: u32,

    /// Default layout algorithm ("horizontal", "vertical")
    pub default_layout: String,
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

    /// Toggle floating state for focused window
    pub toggle_floating: String,

    /// Toggle the focused window's minimized state. Disabled in
    /// practice when `[features].enable_minimize = false`, but the
    /// binding itself is always loaded so the input layer does not
    /// have to know about feature flags.
    pub toggle_minimize: String,

    /// Launch terminal
    pub launch_terminal: String,

    /// Launch application launcher
    pub launch_launcher: String,

    /// Toggle effects
    pub toggle_effects: String,

    /// Quit compositor
    pub quit: String,

    /// ── Mouse button bindings ─────────────────────────────────────────
    /// Each field holds an action name (see `CompositorAction` variants):
    ///   "scroll_left", "scroll_right", "close_window",
    ///   "toggle_fullscreen", "toggle_floating", "toggle_minimize",
    ///   "toggle_effects", "quit".
    /// Button codes follow Linux input event codes:
    ///   0x112 = BTN_MIDDLE,  0x113 = BTN_SIDE (back),
    ///   0x114 = BTN_EXTRA (forward).
    /// Empty string = no binding.

    /// Action for mouse back button (BTN_SIDE, 0x113).
    #[serde(default = "BindingsConfig::default_mouse_back")]
    pub mouse_back: String,

    /// Action for mouse forward button (BTN_EXTRA, 0x114).
    #[serde(default = "BindingsConfig::default_mouse_forward")]
    pub mouse_forward: String,

    /// Action for middle mouse button (BTN_MIDDLE, 0x112).
    #[serde(default = "BindingsConfig::default_mouse_middle")]
    pub mouse_middle: String,
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

    /// Max FPS limit (0 = unlimited, default: 60)
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
            scroll_left: "Super+Left".to_string(),
            scroll_right: "Super+Right".to_string(),
            move_window_left: "Super+Shift+Left".to_string(),
            move_window_right: "Super+Shift+Right".to_string(),
            close_window: "Super+q".to_string(),
            toggle_fullscreen: "Super+f".to_string(),
            toggle_floating: "Super+Shift+Space".to_string(),
            // `grave` (`) is a common minimize hotkey (Hyprland default).
            // The action is a no-op when `[features].enable_minimize = false`,
            // so a user who sets the flag off won't be confused.
            toggle_minimize: "Super+grave".to_string(),
            launch_terminal: "Super+Enter".to_string(),
            launch_launcher: "Super+Space".to_string(),
            toggle_effects: "Super+e".to_string(),
            quit: "Super+Shift+q".to_string(),
            mouse_back: Self::default_mouse_back(),
            mouse_forward: Self::default_mouse_forward(),
            mouse_middle: Self::default_mouse_middle(),
        }
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
            max_fps: 60,
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

impl BindingsConfig {
    fn default_mouse_back() -> String {
        "scroll_left".to_string()
    }
    fn default_mouse_forward() -> String {
        "scroll_right".to_string()
    }
    fn default_mouse_middle() -> String {
        String::new()
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

    /// Validate the configuration, covering all ~30 fields.
    pub fn validate(&self) -> Result<()> {
        // --- workspace ---
        if self.workspace.scroll_speed <= 0.0 || self.workspace.scroll_speed > 10.0 {
            anyhow::bail!("Invalid scroll_speed: must be in (0, 10]");
        }
        if self.workspace.workspace_width == 0 || self.workspace.workspace_width > 16_384 {
            anyhow::bail!("workspace_width must be in [1, 16384]");
        }
        if self.workspace.gaps > 500 {
            anyhow::bail!("gaps must be <= 500");
        }
        if !(0.0..=1.0).contains(&self.workspace.momentum_friction) {
            anyhow::bail!("momentum_friction must be in [0, 1]");
        }
        if self.workspace.momentum_min_velocity < 0.0
            || self.workspace.momentum_min_velocity > 10_000.0
        {
            anyhow::bail!("momentum_min_velocity must be in [0, 10000]");
        }
        if self.workspace.snap_threshold_px < 0.0 || self.workspace.snap_threshold_px > 10_000.0 {
            anyhow::bail!("snap_threshold_px must be in [0, 10000]");
        }

        // --- effects ---
        if self.effects.blur.radius > 256 {
            anyhow::bail!("blur.radius must be <= 256");
        }
        if !(0.0..=1.0).contains(&self.effects.blur.intensity) {
            anyhow::bail!("blur.intensity must be in [0, 1]");
        }
        if self.effects.animations.duration == 0 || self.effects.animations.duration > 60_000 {
            anyhow::bail!("animation duration must be in [1, 60000] ms");
        }
        if self.effects.animations.workspace_transition > 60_000 {
            anyhow::bail!("workspace transition duration must be <= 60000 ms");
        }
        if self.effects.animations.window_animation > 60_000 {
            anyhow::bail!("window animation duration must be <= 60000 ms");
        }
        let valid_curves = ["linear", "ease", "ease-in", "ease-out", "ease-in-out"];
        if !valid_curves.contains(&self.effects.animations.curve.as_str()) {
            anyhow::bail!("Invalid animation curve: {}", self.effects.animations.curve);
        }
        if self.effects.rounded_corners.radius > 256 {
            anyhow::bail!("rounded_corners.radius must be <= 256");
        }
        if !(1..=4).contains(&self.effects.rounded_corners.antialiasing) {
            anyhow::bail!("rounded_corners.antialiasing must be 1-4");
        }
        if !(0.0..=1.0).contains(&self.effects.shadows.opacity) {
            anyhow::bail!("shadows.opacity must be in [0, 1]");
        }
        if self.effects.shadows.size > 1024 {
            anyhow::bail!("shadows.size must be <= 1024");
        }
        if self.effects.shadows.blur_radius > 512 {
            anyhow::bail!("shadows.blur_radius must be <= 512");
        }
        // Validate shadow.color is a valid 6-char hex string.
        // The previous check was format-only (`#` prefix + length 7),
        // which accepted strings like `"#GGGGGG"` that contain
        // non-hex characters; downstream code that feeds this into a
        // GPU fragment shader would silently produce wrong colors
        // (or NaN-style integer overflow on hardware color conversion).
        // `is_ascii_hexdigit` covers A-F, a-f, and 0-9 — the same
        // character set the proptest regex generator in
        // `property_tests.rs` uses to produce valid inputs.
        let color = &self.effects.shadows.color;
        if !color.starts_with('#')
            || color.len() != 7
            || !color[1..].chars().all(|c| c.is_ascii_hexdigit())
        {
            anyhow::bail!(
                "shadows.color must be a #RRGGBB hex string (got {:?})",
                color
            );
        }

        // --- window ---
        if self.window.border_width > 100 {
            anyhow::bail!("border_width must be <= 100");
        }
        if self.window.gap > 500 {
            anyhow::bail!("window.gap must be <= 500");
        }
        let valid_placements = ["smart", "center", "mouse"];
        if !valid_placements.contains(&self.window.placement.as_str()) {
            anyhow::bail!("Invalid window placement: {}", self.window.placement);
        }
        let valid_layouts = ["horizontal", "vertical"];
        if !valid_layouts.contains(&self.window.default_layout.as_str()) {
            anyhow::bail!("Invalid default_layout: {}", self.window.default_layout);
        }

        // --- input ---
        if self.input.keyboard_repeat_delay > 10_000 {
            anyhow::bail!("keyboard_repeat_delay must be <= 10 000 ms");
        }
        if self.input.keyboard_repeat_rate == 0 || self.input.keyboard_repeat_rate > 1000 {
            anyhow::bail!("keyboard_repeat_rate must be in [1, 1000]");
        }
        if !(-1.0..=10.0).contains(&self.input.mouse_accel) {
            anyhow::bail!("mouse_accel must be in [-1, 10]");
        }

        // --- general ---
        if self.general.max_fps > 1000 {
            anyhow::bail!(
                "max_fps must be 0 (unlimited) or in [1, 1000], got {}",
                self.general.max_fps
            );
        }

        // --- xwayland ---
        if let Some(display) = self.xwayland.display {
            if display > 99 {
                anyhow::bail!("xwayland.display must be in [0, 99]");
            }
        }

        Ok(())
    }

    /// Save configuration to a TOML file (atomic write).
    ///
    /// Writes to a temp file in the same directory and renames, so a
    /// mid-write crash leaves either the old file or the new one
    /// intact. File permissions are set to 0600 to avoid leaking config.
    ///
    /// **Validates first.** `save()` runs `self.validate()?` before
    /// serialization so a corrupt configuration (e.g. `scroll_speed =
    /// -5.0`, an out-of-range `border_width`, or a malformed shadow
    /// color) never reaches disk — without this guard, the next
    /// `load()` would crash on the persisted file. The error is
    /// surfaced via `anyhow::Result` so callers (CLI, IPC, GUI) can
    /// tell the user *why* save refused.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        // Reject invalid configs before touching the filesystem so
        // we never leave a half-written tmp behind. The atomic rename
        // in the happy path keeps the on-disk file either entirely
        // old or entirely new; this check keeps it from being either
        // and the new one breaking load().
        self.validate()
            .context("Refusing to save invalid configuration")?;
        let contents = toml::to_string_pretty(self).context("Failed to serialize configuration")?;

        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, &contents)
            .with_context(|| format!("Failed to write temp config: {}", tmp_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)) {
                warn!("⚠️ Failed to set 0600 on config tmp file: {}", e);
            }
        }

        fs::rename(&tmp_path, path).with_context(|| {
            format!(
                "Failed to rename {} -> {}",
                tmp_path.display(),
                path.display()
            )
        })?;

        Ok(())
    }

    /// Merge a partial configuration into this one
    /// Non-default values from the partial config will override this config
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
