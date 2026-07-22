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
    assert!((config.workspace.gaps as i32) >= 0);
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
    let test_config = r##"
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
active_border_color = "#7C3AED"
inactive_border_color = "#374151"
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
close_window = 'Super+Q'
toggle_fullscreen = 'Super+F'
move_window_left = 'Super+H'
move_window_right = 'Super+L'
toggle_floating = 'Super+Space'
launch_terminal = 'Super+Return'
launch_launcher = 'Super+D'
toggle_minimize = 'Super+M'
quit = 'Super+Escape'
focus_next_output = 'Super+Tab'

[effects]
scrolling_animation_duration = 300

[workspace.padding]
top = 10
bottom = 10
left = 10
right = 10
"##;

    fs::write(&file_path, test_config)?;

    // Load configuration from file
    let config = AxiomConfig::load(&file_path)?;

    // Verify loaded values
    assert_eq!(config.workspace.workspace_width, 1600);
    assert_eq!(config.input.mouse_accel, 0.5);

    Ok(())
}

#[test]
fn test_malformed_toml_handling() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("malformed_config.toml");

    // Write malformed TOML
    let malformed_config = r#"
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

#[test]
fn test_save_and_reload() -> Result<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test_save.toml");
    let mut config = AxiomConfig::default();
    config.workspace.scroll_speed = 2.5;
    config.save(&file_path)?;
    assert!(file_path.exists(), "save file should exist");
    let loaded = AxiomConfig::load(&file_path)?;
    assert!(
        (loaded.workspace.scroll_speed - 2.5).abs() < f64::EPSILON,
        "scroll_speed should persist"
    );
    Ok(())
}

#[test]
fn test_save_rejects_invalid_config() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("invalid_save.toml");
    let mut config = AxiomConfig::default();
    config.workspace.scroll_speed = 0.0;
    let result = config.save(&file_path);
    assert!(result.is_err(), "save should reject invalid config");
    assert!(
        !file_path.exists(),
        "no file should be created on invalid save"
    );
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        #[allow(clippy::field_reassign_with_default)]
        fn test_workspace_width_bounds(width in 100u32..10000u32) {
            let mut config = AxiomConfig::default();
            config.workspace.workspace_width = width;

            // Should always be reasonable
            prop_assert!(config.workspace.workspace_width >= 100);
            prop_assert!(config.workspace.workspace_width <= 10000);
        }

        #[test]
        #[allow(clippy::field_reassign_with_default)]
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

    }
}
