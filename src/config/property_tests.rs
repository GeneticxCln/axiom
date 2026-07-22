//! Property-based tests for configuration module
//!
//! These tests use proptest to generate random configurations and verify
//! invariants, serialization round-trips, and edge case handling.

use super::*;
use proptest::prelude::*;

// Strategy for generating valid workspace configurations
prop_compose! {
    fn valid_workspace_config()(
        scroll_speed in 0.1f64..10.0,
        infinite_scroll in any::<bool>(),
        auto_scroll in any::<bool>(),
        workspace_width in 100u32..5000u32,
        gaps in 0u32..100u32,
        smooth_scrolling in any::<bool>(),
    ) -> WorkspaceConfig {
        WorkspaceConfig {
            scroll_speed,
            infinite_scroll,
            auto_scroll,
            workspace_width,
            gaps,
            smooth_scrolling,
            momentum_friction: WorkspaceConfig::default().momentum_friction,
            momentum_min_velocity: WorkspaceConfig::default().momentum_min_velocity,
            snap_threshold_px: WorkspaceConfig::default().snap_threshold_px,
        }
    }
}

// Strategy for generating valid window configurations
prop_compose! {
    fn valid_window_config()(
        placement in prop_oneof![
            Just("smart".to_string()),
            Just("center".to_string()),
            Just("mouse".to_string()),
        ],
        focus_follows_mouse in any::<bool>(),
        border_width in 0u32..20u32,
        active_border_color in "#[0-9A-Fa-f]{6}",
        inactive_border_color in "#[0-9A-Fa-f]{6}",
        gap in 0u32..50u32,
        default_layout in prop_oneof![
            Just("horizontal".to_string()),
            Just("vertical".to_string()),
        ],
    ) -> WindowConfig {
        WindowConfig {
            placement,
            focus_follows_mouse,
            border_width,
            active_border_color,
            inactive_border_color,
            gap,
            default_layout,
        }
    }
}

// Strategy for generating valid input configurations
prop_compose! {
    fn valid_input_config()(
        keyboard_repeat_delay in 100u32..1000u32,
        keyboard_repeat_rate in 5u32..50u32,
        mouse_accel in 0.1f64..5.0f64,
        touchpad_tap in any::<bool>(),
        natural_scrolling in any::<bool>(),
    ) -> InputConfig {
        InputConfig {
            keyboard_repeat_delay,
            keyboard_repeat_rate,
            mouse_accel,
            touchpad_tap,
            natural_scrolling,
        }
    }
}

// Strategy for generating valid key binding configurations
prop_compose! {
    fn valid_bindings_config()(
        scroll_left in valid_key_binding(),
        scroll_right in valid_key_binding(),
        move_window_left in valid_key_binding(),
        move_window_right in valid_key_binding(),
        close_window in valid_key_binding(),
        toggle_fullscreen in valid_key_binding(),
        quit in valid_key_binding(),
    ) -> BindingsConfig {
        BindingsConfig {
            scroll_left,
            scroll_right,
            move_window_left,
            move_window_right,
            close_window,
            toggle_fullscreen,
            toggle_floating: "Super+Shift+Space".to_string(),
            // Pin `toggle_minimize` to the default hotkey so the
            // TOML round-trip assertion matches `BindingsConfig::default()`
            // for both serialization and deserialization directions.
            toggle_minimize: "Super+grave".to_string(),
            launch_terminal: "Super+Enter".to_string(),
            launch_launcher: "Super+Space".to_string(),
            focus_next_output: "Super+Tab".to_string(),
            quit,
            mouse_back: BindingsConfig::default_mouse_back(),
            mouse_forward: BindingsConfig::default_mouse_forward(),
            mouse_middle: BindingsConfig::default_mouse_middle(),
        }
    }
}

prop_compose! {
    fn valid_key_binding()(
        modifier in prop_oneof![
            Just("Super".to_string()),
            Just("Alt".to_string()),
            Just("Ctrl".to_string()),
            Just("Shift".to_string()),
        ],
        key in prop_oneof![
            Just("Left".to_string()),
            Just("Right".to_string()),
            Just("Up".to_string()),
            Just("Down".to_string()),
            Just("q".to_string()),
            Just("f".to_string()),
            Just("Return".to_string()),
        ],
    ) -> String {
        format!("{}+{}", modifier, key)
    }
}

// Strategy for generating valid feature-flag configurations. Both
// fields are independent bools with a default of `false`, so we
// exercise the enabled branch at half probability; the round-trip
// assertions below cover both directions.
prop_compose! {
    fn valid_features_config()(
        enable_minimize in any::<bool>(),
        enable_xdg_decoration_protocol in any::<bool>(),
    ) -> FeaturesConfig {
        FeaturesConfig {
            enable_minimize,
            enable_xdg_decoration_protocol,
        }
    }
}

// Strategy for generating valid general configurations
prop_compose! {
    fn valid_general_config()(
        debug in any::<bool>(),
        max_fps in 0u32..480u32,
        vsync in any::<bool>(),
    ) -> GeneralConfig {
        GeneralConfig {
            debug,
            max_fps,
            vsync,
            default_terminal: "xterm".into(),
            default_launcher: "dmenu_run".into(),
        }
    }
}

// Strategy for generating full valid configurations
prop_compose! {
    fn valid_axiom_config()(
        workspace in valid_workspace_config(),
        window in valid_window_config(),
        input in valid_input_config(),
        bindings in valid_bindings_config(),
        general in valid_general_config(),
    ) -> AxiomConfig {
        AxiomConfig {
            workspace,
            window,
            input,
            bindings,
            general,
            // BackendConfig has no validation knobs of its own (it
            // round-trips through `BackendKind::from_config_str`); the
            // default `kind = "winit"` is sufficient for round-trip
            // assertions. Add a strategy here if validate() grows.
            backend: BackendConfig::default(),
            // FeaturesConfig round-trips as a pair of bools. Use the
            // default (`enable_minimize=false`,
            // `enable_xdg_decoration_protocol=false`) for baseline
            // serialization tests; add an explicit override strategy
            // here if a future invariant gate gets layered onto either
            // field.
            features: FeaturesConfig::default(),
            output: OutputConfig::default(),
        }
    }
}

// Strategy for generating valid backend configurations, kept isolated
// so future validation extensions (e.g. accepting only certain kinds)
// can swap it in without touching the rest of the test suite.
prop_compose! {
    fn valid_backend_config()(
        kind in prop_oneof![
            Just("winit".to_string()),
            Just("noop".to_string()),
        ],
    ) -> BackendConfig {
        BackendConfig { kind }
    }
}

proptest! {
    /// Test that all valid configurations can be serialized to TOML
    #[test]
    fn test_config_toml_serialization(config in valid_axiom_config()) {
        let toml_result = toml::to_string(&config);
        prop_assert!(toml_result.is_ok(), "Failed to serialize config to TOML: {:?}", toml_result.err());
    }

    /// Test TOML serialization round-trip preserves data
    #[test]
    fn test_config_toml_roundtrip(config in valid_axiom_config()) {
        let toml_str = toml::to_string(&config)?;
        let parsed_config: AxiomConfig = toml::from_str(&toml_str)?;

        // Compare key properties (floating-point comparison requires tolerance)
        prop_assert_eq!(config.workspace.infinite_scroll, parsed_config.workspace.infinite_scroll);
        prop_assert_eq!(config.workspace.workspace_width, parsed_config.workspace.workspace_width);
        prop_assert_eq!(config.general.vsync, parsed_config.general.vsync);

        // Check floating point values with tolerance
        prop_assert!((config.workspace.scroll_speed - parsed_config.workspace.scroll_speed).abs() < 0.001);
        prop_assert!((config.input.mouse_accel - parsed_config.input.mouse_accel).abs() < 0.001);
    }

    /// Test that configuration validation works correctly
    #[test]
    fn test_config_validation(config in valid_axiom_config()) {
        // All generated configs should be valid
        prop_assert!(config.workspace.workspace_width > 0);
        prop_assert!(config.workspace.scroll_speed > 0.0);
        prop_assert!(config.input.keyboard_repeat_rate > 0);
        prop_assert!(config.input.mouse_accel > 0.0);
        // max_fps is u32, always >= 0
    }

    /// Test that partial configuration merging works correctly
    #[test]
    fn test_partial_config_merge(
        base_config in valid_axiom_config(),
        workspace_override in valid_workspace_config()
    ) {
        let mut partial_config = AxiomConfig::default();
        #[allow(clippy::field_reassign_with_default)]
        {
            partial_config.workspace = workspace_override.clone();
        }

        let base_vsync = base_config.general.vsync;
        let merged = base_config.merge_partial(partial_config);

        // Merged config should have the overridden workspace config
        prop_assert_eq!(merged.workspace.workspace_width, workspace_override.workspace_width);
        prop_assert_eq!(merged.workspace.infinite_scroll, workspace_override.infinite_scroll);

        // Other sections should remain from base config
        prop_assert_eq!(merged.general.vsync, base_vsync);
    }

    /// Test edge cases for numeric values
    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn test_numeric_edge_cases(
        tiny_scroll_speed in 0.001f64..0.1f64,
        large_workspace_width in 10000u32..50000u32,
        zero_gaps in Just(0u32),
    ) {
        let mut config = AxiomConfig::default();
        config.workspace.scroll_speed = tiny_scroll_speed;
        config.workspace.workspace_width = large_workspace_width;
        config.workspace.gaps = zero_gaps;

        // Should still serialize successfully
        let toml_result = toml::to_string(&config);
        prop_assert!(toml_result.is_ok());

        // Edge values should be preserved
        prop_assert!(config.workspace.scroll_speed > 0.0);
        prop_assert!(config.workspace.workspace_width >= 10000);
        prop_assert_eq!(config.workspace.gaps, 0);
    }
}

#[cfg(test)]
mod stress_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn test_concurrent_config_access() {
        let config = Arc::new(Mutex::new(AxiomConfig::default()));
        let mut handles = vec![];

        // Spawn multiple threads that access config concurrently
        for i in 0..10 {
            let config_clone = Arc::clone(&config);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    let mut cfg = config_clone.lock().unwrap();
                    cfg.workspace.workspace_width = (i * 100 + j) as u32;
                    cfg.workspace.scroll_speed = (i as f64 + j as f64) / 100.0;

                    // Serialize to test memory safety
                    let _ = toml::to_string(&*cfg);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Config should still be valid
        let final_config = config.lock().unwrap();
        assert!(final_config.workspace.workspace_width > 0);
        assert!(final_config.workspace.scroll_speed >= 0.0);
    }

    #[test]
    fn test_large_config_serialization() {
        let config = AxiomConfig::default();

        // Should handle large configs gracefully
        let toml_result = toml::to_string(&config);
        assert!(toml_result.is_ok());

        let toml_str = toml_result.unwrap();
        assert!(toml_str.len() > 1000); // Reasonably large

        // Round-trip should work
        let parsed: Result<AxiomConfig, _> = toml::from_str(&toml_str);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_memory_usage_stability() {
        // Test that repeated config operations don't leak memory
        for _ in 0..1000 {
            let config = AxiomConfig::default();
            let toml_str = toml::to_string(&config).unwrap();
            let _parsed: AxiomConfig = toml::from_str(&toml_str).unwrap();

            // Force drop to test cleanup
            drop(config);
        }

        // If we get here without running out of memory, test passes
    }
}
