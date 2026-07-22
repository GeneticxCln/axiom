//! Configuration management for Axiom
//!
//! This module handles loading, parsing, and validating configuration
//! from TOML files. It combines settings for workspaces,
//! input handling, and more.
//!
//! The configuration is composed of several sections:
//! - [`WorkspaceConfig`]: Scrollable workspace behavior
//! - // Visual effects config removed with effects engine
//! - [`WindowConfig`]: Window management and placement
//! - [`InputConfig`]: Input device handling
//! - [`BindingsConfig`]: Key binding mappings
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

    /// Window management settings
    #[serde(default)]
    pub window: WindowConfig,

    /// Input handling and keybindings
    #[serde(default)]
    pub input: InputConfig,

    /// Key bindings
    #[serde(default)]
    pub bindings: BindingsConfig,

    /// Backend selection (winit / noop). Default is `winit` for
    /// development; tests/CI override via `backend.kind = "noop"`.
    /// Stored as `String` here so the config schema is self-contained
    /// and parses without pulling in the backend module.
    #[serde(default)]
    pub backend: BackendConfig,

    /// Feature kill-switches for features we keep modest to focus the
    /// implementation surface. `enable_minimize` defaults `false` so the
    /// titlebar minimize button is hidden (requires iconified-window protocol
    /// round-trips). `enable_xdg_decoration_protocol` defaults `false` but
    /// when enabled, the compositor negotiates `ServerSide` and renders
    /// visible SSD titlebars/buttons via GLES.
    /// Users can enable either independently via config.
    /// Users can opt back into either independently by setting the
    /// matching flag to `true` (and then supplying the corresponding
    /// implementation when wiring time comes).
    #[serde(default)]
    pub features: FeaturesConfig,

    /// Output configuration (multi-monitor layout)
    #[serde(default)]
    pub output: OutputConfig,

    /// General compositor settings
    #[serde(default)]
    pub general: GeneralConfig,
}

/// Output configuration (multi-monitor layout)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputConfig {
    /// Preferred output order for the horizontal strip layout.
    /// Output names (e.g. `"HDMI-A-1"`, `"DP-1"`) listed here appear in
    /// this order from left to right. Any connected output not listed is
    /// appended at the end in DRM-enumeration order.
    /// Leave empty to use the natural DRM enumeration order.
    #[serde(default)]
    pub order: Vec<String>,
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
/// The `kind` field accepts `"winit"` or `"noop"` (plus `from_config_str`
/// aliases like `"windowed"`). Unknown values fall back to `winit`.
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

    /// Quit compositor
    pub quit: String,

    /// Switch focus to next output
    pub focus_next_output: String,

    /// ── Mouse button bindings ─────────────────────────────────────────
    /// Each field holds an action name (see `CompositorAction` variants):
    ///   "scroll_left", "scroll_right", "close_window",
    ///   "toggle_fullscreen", "toggle_floating", "toggle_minimize",
    ///   "quit".
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

/// General compositor settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneralConfig {
    /// Enable debug logging
    pub debug: bool,

    /// Max FPS limit (0 = unlimited, default: 60)
    pub max_fps: u32,

    /// Enable VSync
    pub vsync: bool,

    /// Default terminal emulator command
    #[serde(default = "GeneralConfig::default_terminal")]
    pub default_terminal: String,

    /// Default application launcher command
    #[serde(default = "GeneralConfig::default_launcher")]
    pub default_launcher: String,
}

impl GeneralConfig {
    fn default_terminal() -> String {
        "xterm".into()
    }

    fn default_launcher() -> String {
        "dmenu_run".into()
    }
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
            launch_terminal: "Super+Return".to_string(),
            launch_launcher: "Super+Space".to_string(),
            quit: "Super+Shift+q".to_string(),
            focus_next_output: "Super+Tab".to_string(),
            mouse_back: Self::default_mouse_back(),
            mouse_forward: Self::default_mouse_forward(),
            mouse_middle: Self::default_mouse_middle(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            debug: false,
            max_fps: 60,
            vsync: true,
            default_terminal: Self::default_terminal(),
            default_launcher: Self::default_launcher(),
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

        // --- window ---
        if self.window.border_width > 100 {
            anyhow::bail!("border_width must be <= 100");
        }
        if self.window.gap > 500 {
            anyhow::bail!("window.gap must be <= 500");
        }
        if self.window.gap != 10 {
            log::warn!("window.gap is deprecated — use workspace.gaps instead. This field does not affect layout.");
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

        // --- bindings ---
        for (field_name, binding) in [
            ("scroll_left", &self.bindings.scroll_left),
            ("scroll_right", &self.bindings.scroll_right),
            ("move_window_left", &self.bindings.move_window_left),
            ("move_window_right", &self.bindings.move_window_right),
            ("close_window", &self.bindings.close_window),
            ("toggle_fullscreen", &self.bindings.toggle_fullscreen),
            ("toggle_floating", &self.bindings.toggle_floating),
            ("toggle_minimize", &self.bindings.toggle_minimize),
            ("launch_terminal", &self.bindings.launch_terminal),
            ("launch_launcher", &self.bindings.launch_launcher),
            ("quit", &self.bindings.quit),
        ] {
            if binding.is_empty() {
                anyhow::bail!("bindings.{} must not be empty", field_name);
            }
            if !binding.contains("Super")
                && !binding.contains("Alt")
                && !binding.contains("Ctrl")
                && !binding.contains("Shift")
            {
                anyhow::bail!(
                    "bindings.{} = {:?} must contain at least one modifier (Super, Alt, Ctrl, or Shift)",
                    field_name, binding
                );
            }
        }

        // --- general ---
        if self.general.max_fps > 1000 {
            anyhow::bail!(
                "max_fps must be 0 (unlimited) or in [1, 1000], got {}",
                self.general.max_fps
            );
        }

        // --- output ---
        // Validate that all entries in output.order are non-empty and
        // contain only valid identifier characters. DRM connector names
        // like "HDMI-A-1" are the expected format.
        for (i, name) in self.output.order.iter().enumerate() {
            if name.is_empty() {
                anyhow::bail!(
                    "output.order[{}] is empty — each entry must be a non-empty connector name",
                    i
                );
            }
            if name.len() > 256 {
                anyhow::bail!("output.order[{}] name too long (max 256 chars)", i);
            }
            // Allow alphanumeric, hyphen, underscore, dash
            if !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                anyhow::bail!(
                    "output.order[{}] = {:?} contains invalid characters — use alphanumeric, hyphen, or underscore",
                    i, name
                );
            }
        }
        // Check for duplicates in output.order
        {
            let mut seen = std::collections::HashSet::new();
            for name in &self.output.order {
                if !seen.insert(name) {
                    anyhow::bail!("output.order contains duplicate entry {:?}", name);
                }
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

    /// Merge a partial configuration into this one.
    ///
    /// ## Limitation
    ///
    /// This method compares each section of `partial` against `AxiomConfig::default()`
    /// to decide whether the caller intentionally provided a value. This means a
    /// section whose fields happen to equal the defaults **cannot be distinguished
    /// from an absent section** — values cannot be "reset to default" through a
    /// partial merge. For that use case, use [`reset_to_defaults`](Self::reset_to_defaults).
    ///
    /// Non-default values from the partial config will override this config.
    pub fn merge_partial(mut self, partial: AxiomConfig) -> Self {
        let default_config = AxiomConfig::default();

        // Helper to decide if a section in partial differs from default (meaningfully provided)
        let workspace_changed = partial.workspace != default_config.workspace;
        let window_changed = partial.window != default_config.window;
        let input_changed = partial.input != default_config.input;
        let bindings_changed = partial.bindings != default_config.bindings;
        let output_changed = partial.output != default_config.output;
        let general_changed = partial.general != default_config.general;

        if workspace_changed {
            self.workspace = partial.workspace;
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
        if output_changed {
            self.output = partial.output;
        }
        if general_changed {
            self.general = partial.general;
        }

        self
    }

    /// Reset all fields to their default values.
    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;
