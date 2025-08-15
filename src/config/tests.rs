//! Unit tests for configuration module
//!
//! Tests configuration parsing, validation, serialization/deserialization,
//! and edge cases in configuration handling.

use super::*;
use anyhow::Result;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_default_configuration_is_valid() {
    let config = AxiomConfig::default();

    // Test that all default values are reasonable
    assert!(config.effects.enabled);
    assert!(config.effects.blur.radius >= 0);
    assert!(config.effects.shadows.size >= 0);
    assert!(config.workspace.workspace_width > 0);
    assert!(config.workspace.gaps >= 0);
    assert!(config.workspace.scroll_speed > 0.0);

    // Test that input values are reasonable
    assert!(config.input.mouse_accel >= 0.0);
    assert!(config.input.keyboard_repeat_delay > 0);
}

#[test]
fn test_configuration_serialization_roundtrip() -> Result<()> {
    let original_config = AxiomConfig::default();

    // Serialize to TOML
    let toml_string = toml::to_string(&original_config)?;

    // Deserialize back
    let deserialized_config: AxiomConfig = toml::from_str(&toml_string)?;

    // Compare key values
    assert_eq!(
        original_config.effects.enabled,
        deserialized_config.effects.enabled
    );
    assert_eq!(
        original_config.effects.blur.radius,
        deserialized_config.effects.blur.radius
    );
    assert_eq!(
        original_config.workspace.workspace_width,
        deserialized_config.workspace.workspace_width
    );
    assert_eq!(
        original_config.input.mouse_accel,
        deserialized_config.input.mouse_accel
    );

    Ok(())
}

#[test]
fn test_configuration_from_file() -> Result<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test_config.toml");

    // Write test configuration
    let test_config = r#"
[effects]
enabled = true

[effects.animations]
enabled = true
duration = 300
curve = "ease-out"
workspace_transition = 250
window_animation = 200

[effects.blur]
enabled = true
radius = 15
intensity = 0.9
window_backgrounds = true

[effects.rounded_corners]
enabled = true
radius = 8
antialiasing = 2

[effects.shadows]
enabled = true
size = 25
blur_radius = 15
opacity = 0.7
color = '#000000'

[workspace]
scroll_speed = 1.5
infinite_scroll = true
auto_scroll = true
workspace_width = 1600
gaps = 15
smooth_scrolling = true

[window]
placement = "smart"
focus_follows_mouse = false
border_width = 2
active_border_color = "7C3AED"
inactive_border_color = "374151"
gap = 10
default_layout = "horizontal"

[input]
keyboard_repeat_delay = 500
keyboard_repeat_rate = 25
mouse_accel = 0.5
touchpad_tap = true
natural_scrolling = true

[bindings]
scroll_left = 'Super+Left'
scroll_right = 'Super+Right'
move_window_left = 'Super+Shift+Left'
move_window_right = 'Super+Shift+Right'
close_window = 'Super+q'
toggle_fullscreen = 'Super+f'
quit = 'Super+Shift+q'

[xwayland]
enabled = false
lazy_loading = true
scale_factor = 1.0

[general]
compositor_name = "axiom"
socket_name = "axiom"
log_level = "info"
enable_debug_output = false
max_clients = 100
config_path = '~/.config/axiom/config.toml'
startup_apps = []
"#;

    fs::write(&file_path, test_config)?;

    // Load configuration from file
    let config = AxiomConfig::load(&file_path)?;

    // Verify loaded values
    assert_eq!(config.effects.blur.radius, 15);
    assert_eq!(config.effects.shadows.size, 25);
    assert_eq!(config.workspace.workspace_width, 1600);
    assert_eq!(config.input.mouse_accel, 0.5);
    assert_eq!(config.xwayland.enabled, false);

    Ok(())
}

#[test]
fn test_partial_configuration_merge() -> Result<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("partial_config.toml");

    // Write partial configuration (only effects section)
    let partial_config = r#"
[effects]
enabled = false

[effects.animations]
enabled = true
duration = 300
curve = "ease-out"
workspace_transition = 250
window_animation = 200

[effects.blur]
enabled = true
radius = 25
intensity = 0.8
window_backgrounds = true

[effects.rounded_corners]
enabled = true
radius = 8
antialiasing = 2

[effects.shadows]
enabled = true
size = 20
blur_radius = 15
opacity = 0.6
color = '#000000'
"#;

    fs::write(&file_path, partial_config)?;

    // Load configuration - should merge with defaults
    let config = AxiomConfig::load(&file_path)?;

    // Verify overridden values
    assert_eq!(config.effects.blur.radius, 25);
    assert_eq!(config.effects.enabled, false);

    // Verify default values are still present
    assert!(config.workspace.workspace_width > 0);
    assert!(config.input.keyboard_repeat_delay > 0);

    Ok(())
}

#[test]
fn test_malformed_toml_handling() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("malformed_config.toml");

    // Write malformed TOML
    let malformed_config = r#"
[effects
enabled = true
missing_bracket

[workspace]
workspace_width = "not a number"
"#;

    fs::write(&file_path, malformed_config).unwrap();

    // Should return error for malformed TOML
    let result = AxiomConfig::load(&file_path);
    assert!(result.is_err());
}

#[test]
fn test_configuration_validation() {
    let config = AxiomConfig::default();

    // Test that validation passes for default config
    assert!(config.validate().is_ok());

    // Test scroll speed bounds
    let mut invalid_config = config.clone();
    invalid_config.workspace.scroll_speed = 0.0;
    assert!(invalid_config.validate().is_err());

    invalid_config.workspace.scroll_speed = 15.0;
    assert!(invalid_config.validate().is_err());
}

#[test]
fn test_bindings_config_validation() {
    let config = BindingsConfig::default();

    // Test that default bindings are present
    assert!(!config.scroll_left.is_empty());
    assert!(!config.scroll_right.is_empty());
    assert!(!config.close_window.is_empty());

    // Test that keybindings are valid format
    assert!(is_valid_keybinding(&config.scroll_left));
    assert!(is_valid_keybinding(&config.quit));
}

#[test]
fn test_xwayland_config() {
    let mut config = XWaylandConfig::default();

    // Test enabling/disabling XWayland
    config.enabled = false;
    assert!(!config.enabled);

    config.enabled = true;
    assert!(config.enabled);

    // Test scale_factor field
    assert_eq!(config.scale_factor, 1.0);

    config.scale_factor = 1.5;
    assert_eq!(config.scale_factor, 1.5);
}

// Helper function to validate keybinding format
fn is_valid_keybinding(binding: &str) -> bool {
    // Simple validation - real implementation would be more comprehensive
    !binding.is_empty()
        && (binding.contains("Super")
            || binding.contains("Alt")
            || binding.contains("Ctrl")
            || binding.contains("Shift")
            || binding
                .chars()
                .all(|c| c.is_alphanumeric() || "+-_".contains(c)))
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_workspace_width_bounds(width in 100u32..10000u32) {
            let mut config = AxiomConfig::default();
            config.workspace.workspace_width = width;

            // Should always be reasonable
            prop_assert!(config.workspace.workspace_width >= 100);
            prop_assert!(config.workspace.workspace_width <= 10000);
        }

        #[test]
        fn test_scroll_speed_bounds(speed in 0.1f64..20.0f64) {
            let mut config = AxiomConfig::default();
            config.workspace.scroll_speed = speed;

            // Validation should handle extreme values
            let result = config.validate();
            if speed <= 10.0 && speed > 0.0 {
                prop_assert!(result.is_ok());
            } else {
                prop_assert!(result.is_err());
            }
        }

        #[test]
        fn test_blur_intensity_bounds(intensity in 0.0f64..2.0f64) {
            let mut config = AxiomConfig::default();
            config.effects.blur.intensity = intensity;

            // Validation should handle bounds
            let result = config.validate();
            if intensity >= 0.0 && intensity <= 1.0 {
                prop_assert!(result.is_ok());
            } else {
                prop_assert!(result.is_err());
            }
        }
    }
}
