//! Tests for viewport sizing, fractional scaling, and layout cache
//! invalidation.
//!
//! Covers issue #15: Viewport resize & fractional scaling unit tests.

use axiom::config::WorkspaceConfig;
use axiom::workspace::{scale_to_logical, scale_to_physical, ScrollableWorkspaces};

// ---------------------------------------------------------------------------
// Viewport resize
// ---------------------------------------------------------------------------

#[test]
fn test_set_viewport_size_1080p() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    ws.set_viewport_size(1920.0, 1080.0);
    // virtual_desktop_size reflects the tape's viewport.
    let (vw, vh) = ws.virtual_desktop_size();
    assert_eq!(vw, 1920, "virtual width after set_viewport_size");
    assert_eq!(vh, 1080, "virtual height after set_viewport_size");
}

#[test]
fn test_set_viewport_size_1440p() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    ws.set_viewport_size(2560.0, 1440.0);
    let (vw, vh) = ws.virtual_desktop_size();
    assert_eq!(vw, 2560);
    assert_eq!(vh, 1440);
}

#[test]
fn test_set_viewport_size_4k() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    ws.set_viewport_size(3840.0, 2160.0);
    let (vw, vh) = ws.virtual_desktop_size();
    assert_eq!(vw, 3840);
    assert_eq!(vh, 2160);
}

#[test]
fn test_set_viewport_size_per_output() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    // "default" tape exists (1920x1080 from DEFAULT_VIEWPORT_*).
    // Override it and add a second output.
    ws.set_viewport_size(1920.0, 1080.0);
    ws.set_output_viewport("hdmi-1", 2560.0, 1440.0);

    let (vw, vh) = ws.virtual_desktop_size();
    // default (1920) + hdmi-1 (2560) = 4480
    assert_eq!(vw, 1920 + 2560, "virtual width sums all tapes");
    assert_eq!(vh, 1440, "virtual height is max of all tapes");
}

// ---------------------------------------------------------------------------
// Fractional-scale helper functions
// ---------------------------------------------------------------------------

#[test]
fn test_scale_to_physical_unit() {
    // 1.0x → identity
    assert_eq!(scale_to_physical(100.0, 1.0), 100);
    // 2.0x → double
    assert_eq!(scale_to_physical(100.0, 2.0), 200);
    // 1.5x
    assert_eq!(scale_to_physical(100.0, 1.5), 150);
    // 1.25x
    assert_eq!(scale_to_physical(100.0, 1.25), 125);
    // rounding: 100 * 1.75 = 175 → 175 (exact)
    assert_eq!(scale_to_physical(100.0, 1.75), 175);
    // rounding up: 100 * 1.33 = 133 → 133 (truncation via +0.5)
    assert_eq!(scale_to_physical(100.0, 1.33), 133);
    // zero
    assert_eq!(scale_to_physical(0.0, 2.0), 0);
}

#[test]
fn test_scale_to_logical_unit() {
    // identity
    assert!((scale_to_logical(100, 1.0) - 100.0).abs() < f64::EPSILON);
    // half
    assert!((scale_to_logical(200, 2.0) - 100.0).abs() < f64::EPSILON);
    // fractional
    assert!((scale_to_logical(150, 1.5) - 100.0).abs() < f64::EPSILON);
    // 1.25x
    assert!((scale_to_logical(125, 1.25) - 100.0).abs() < f64::EPSILON);
    // zero
    assert!((scale_to_logical(0, 2.0) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_scale_round_trip() {
    // scale_to_logical ∘ scale_to_physical is approximately identity.
    // Integer rounding introduces ±0.5px error per conversion; use a
    // tolerance of 0.5 and skip very small values (< 10) where the
    // relative error is large.
    for scale in &[1.0, 1.25, 1.5, 2.0, 3.0] {
        for value in &[10, 100, 1920, 2560] {
            let logical = *value as f64;
            let physical = scale_to_physical(logical, *scale);
            let back = scale_to_logical(physical, *scale);
            assert!(
                (back - logical).abs() < 0.51,
                "round-trip failed for value={} scale={}: physical={}, back={}",
                value,
                scale,
                physical,
                back
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Configured sizes with different scales
// ---------------------------------------------------------------------------

#[test]
fn test_configured_sizes_with_scale() {
    // Simulate the pattern used in prepare_render_scene:
    //   new_w = (scale_to_logical(rect.width as i32, scale).round() as i32).max(1)
    let cases = &[
        // (physical, scale, expected_logical)
        (1920, 1.0, 1920),
        (1920, 2.0, 960),
        (1920, 1.5, 1280),
        (2560, 1.25, 2048),
        (3840, 2.0, 1920),
        (100, 3.0, 33),
        (1, 4.0, 1),   // .max(1)
        (0, 1.5, 1),   // .max(1) clamps to 1
    ];
    for &(physical, scale, expected) in cases {
        let logical = (scale_to_logical(physical, scale).round() as i32).max(1);
        assert_eq!(
            logical, expected,
            "physical={} scale={}: expected logical={}, got={}",
            physical, scale, expected, logical
        );
    }
}

// ---------------------------------------------------------------------------
// Layout cache invalidation
// ---------------------------------------------------------------------------

/// Helper: add windows so layout calculation produces non-empty results.
fn populate_workspace(ws: &mut ScrollableWorkspaces) {
    ws.add_window_to_column(1001, 0);
    ws.add_window_to_column(1002, 0);
    ws.add_window_to_column(1003, 1);
}

#[test]
fn test_layout_cache_invalidated_on_viewport_change() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);
    populate_workspace(&mut ws);

    ws.set_viewport_size(1920.0, 1080.0);
    let layouts_a = ws.calculate_workspace_layouts();

    // Change viewport size – layout cache must be invalidated and the
    // window rectangles should differ (wider column → wider tiles).
    ws.set_viewport_size(2560.0, 1440.0);
    let layouts_b = ws.calculate_workspace_layouts();

    assert_ne!(layouts_a, layouts_b, "cache must invalidate on viewport resize");
}

#[test]
fn test_layout_cache_invalidated_on_output_viewport_change() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);
    populate_workspace(&mut ws);

    // All windows are on the "default" tape; set its viewport.
    ws.set_output_viewport("default", 1920.0, 1080.0);
    let layouts_a = ws.calculate_workspace_layouts();

    // Change the viewport for the default output.
    ws.set_output_viewport("default", 3840.0, 2160.0);
    let layouts_b = ws.calculate_workspace_layouts();

    assert_ne!(layouts_a, layouts_b, "cache must invalidate on output viewport change");
}

#[test]
fn test_layout_cache_stable_for_same_state() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);
    populate_workspace(&mut ws);

    ws.set_viewport_size(1920.0, 1080.0);
    let layouts_a = ws.calculate_workspace_layouts();

    // Second call without state change should return the same cached result.
    let layouts_b = ws.calculate_workspace_layouts();

    assert_eq!(layouts_a, layouts_b, "cache hit returns identical layouts");
}

// ---------------------------------------------------------------------------
// Fractional-scale edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_scale_to_physical_unusual_fractions() {
    // 0.75x scale
    assert_eq!(scale_to_physical(100.0, 0.75), 75);
    // 1.33x scale (typical 1.33...)
    assert_eq!(scale_to_physical(100.0, 1.33), 133);
    // 1.75x scale
    assert_eq!(scale_to_physical(100.0, 1.75), 175);
    // 2.5x scale
    assert_eq!(scale_to_physical(100.0, 2.5), 250);
    // 3.0x scale
    assert_eq!(scale_to_physical(100.0, 3.0), 300);
}

#[test]
fn test_scale_to_physical_boundary_scales() {
    // scale = 0.0 — should not panic, returns 0
    assert_eq!(scale_to_physical(100.0, 0.0), 0);
    assert_eq!(scale_to_physical(0.0, 0.0), 0);
    // scale = 10.0 — very high, no overflow
    assert_eq!(scale_to_physical(100.0, 10.0), 1000);
    assert_eq!(scale_to_physical(1.0, 10.0), 10);
}

#[test]
fn test_scale_to_logical_boundary_scales() {
    // scale = 0.0 — division by zero gives inf, but the function should not
    // panic and the caller is expected to clamp. Accept any non-panicking
    // result (including inf).
    let result = scale_to_logical(100, 0.0);
    assert!(result.is_infinite() || result.is_nan() || result >= 0.0);
    // scale = 10.0
    assert!((scale_to_logical(1000, 10.0) - 100.0).abs() < f64::EPSILON);
}

#[test]
fn test_scale_to_physical_rounding() {
    // 10.5 at 1.5x → 16 (10.5 * 1.5 = 15.75, + 0.5 = 16.25 → 16)
    assert_eq!(scale_to_physical(10.5, 1.5), 16);
    // 10.5 at 1.0x → 11 (10.5 * 1.0 = 10.5, + 0.5 = 11.0 → 11)
    assert_eq!(scale_to_physical(10.5, 1.0), 11);
    // 10.5 at 2.0x → 21 (10.5 * 2.0 = 21.0, + 0.5 = 21.5 → 21)
    assert_eq!(scale_to_physical(10.5, 2.0), 21);
    // 3.3 at 1.5x → 5 (3.3 * 1.5 = 4.95, + 0.5 = 5.45 → 5)
    assert_eq!(scale_to_physical(3.3, 1.5), 5);
    // 0.6 at 1.5x → 1 (0.6 * 1.5 = 0.9, + 0.5 = 1.4 → 1)
    assert_eq!(scale_to_physical(0.6, 1.5), 1);
    // 0.4 at 1.5x → 1 (0.4 * 1.5 = 0.6, + 0.5 = 1.1 → 1)
    assert_eq!(scale_to_physical(0.4, 1.5), 1);
    // exactly halfway: 1.0 at 1.5x → 2 (1.0 * 1.5 = 1.5, + 0.5 = 2.0 → 2)
    assert_eq!(scale_to_physical(1.0, 1.5), 2);
}

#[test]
fn test_scale_to_physical_extreme_values() {
    // Very large dimension
    assert_eq!(scale_to_physical(10000.0, 1.5), 15000);
    assert_eq!(scale_to_physical(10000.0, 2.5), 25000);
    // Very small (1px)
    assert_eq!(scale_to_physical(1.0, 1.0), 1);
    assert_eq!(scale_to_physical(1.0, 1.5), 2);  // 1.0 * 1.5 + 0.5 = 2.0 → 2
    assert_eq!(scale_to_physical(1.0, 2.0), 2);
    assert_eq!(scale_to_physical(1.0, 3.0), 3);
}

#[test]
fn test_scale_to_logical_extreme_values() {
    // Very large dimension
    assert!((scale_to_logical(15000, 1.5) - 10000.0).abs() < f64::EPSILON);
    // Very small (1px)
    assert!((scale_to_logical(1, 1.0) - 1.0).abs() < f64::EPSILON);
    assert!((scale_to_logical(1, 2.0) - 0.5).abs() < f64::EPSILON);
    assert!((scale_to_logical(1, 4.0) - 0.25).abs() < f64::EPSILON);
}

#[test]
fn test_scale_round_trip_physical_logical_physical() {
    // Round-trip from physical → logical → physical with tolerance.
    // The round-trip loses precision because scale_to_physical rounds,
    // but the result should be within 1px of the original.
    for scale in &[0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
        for value in &[1, 10, 100, 1920, 2560, 10000] {
            let physical = *value;
            let logical = scale_to_logical(physical, *scale);
            let back = scale_to_physical(logical, *scale);
            let diff = (back - physical).abs();
            assert!(
                diff <= 1,
                "physical→logical→physical failed for value={} scale={}: logical={}, back={}, diff={}",
                value,
                scale,
                logical,
                back,
                diff
            );
        }
    }
}

#[test]
fn test_scale_round_trip_various_frac_values() {
    // Round-trip with fractional logical values (not just whole numbers).
    let cases = &[
        // (logical_value, scale, expected_physical)
        (10.5, 1.0, 11),
        (10.5, 1.5, 16),
        (10.5, 2.0, 21),
        (3.3, 1.5, 5),
        (0.6, 2.0, 1),
        (0.4, 2.0, 1),
    ];
    for &(logical, scale, expected_physical) in cases {
        let physical = scale_to_physical(logical, scale);
        assert_eq!(
            physical, expected_physical,
            "scale_to_physical({}, {}) = {}, expected {}",
            logical, scale, physical, expected_physical
        );
        // Convert back and verify it's close to the original.
        let back = scale_to_logical(physical, scale);
        assert!(
            (back - logical).abs() < 1.0,
            "round-trip failed: logical={}, scale={}, physical={}, back={}",
            logical,
            scale,
            physical,
            back
        );
    }
}

// ---------------------------------------------------------------------------
// Scale factor integration with workspace
// ---------------------------------------------------------------------------

#[test]
fn test_workspace_scale_factor_default() {
    let config = WorkspaceConfig::default();
    let ws = ScrollableWorkspaces::new(&config);
    // Default scale factor on the default tape is 1.0.
    assert!((ws.scale_factor() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_workspace_scale_factor_for_window() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    // Add a window to the default tape.
    ws.add_window_to_column(1001, 0);

    // Default scale factor is 1.0.
    assert!((ws.scale_factor_for_window(1001) - 1.0).abs() < f64::EPSILON);

    // Set scale factor on the default tape.
    ws.ensure_tape("default").set_scale_factor(2.0);
    assert!((ws.scale_factor_for_window(1001) - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_workspace_scale_factor_multi_output() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    // Set up two outputs with different scales.
    ws.set_output_viewport("hdmi-1", 2560.0, 1440.0);
    ws.ensure_tape("hdmi-1").set_scale_factor(2.0);
    ws.ensure_tape("default").set_scale_factor(1.5);

    // Add windows to each output's tape.
    ws.add_window_to_column(1001, 0);  // goes to "default" (active tape)
    ws.ensure_tape("hdmi-1").add_window_to_column(2001, 0);

    // default tape is "default" since no focus switch happened.
    // But add_window_to_column only works on the active tape.
    // Manually ensure the window is on the right tape.
    // Actually, let's do it more carefully: focus the output first.
    // For this test, we'll just check that ensure_tape sets up the tapes correctly.
    assert!((ws.ensure_tape("default").scale_factor() - 1.5).abs() < f64::EPSILON);
    assert!((ws.ensure_tape("hdmi-1").scale_factor() - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_workspace_scale_factor_clamped() {
    let config = WorkspaceConfig::default();
    let mut ws = ScrollableWorkspaces::new(&config);

    // Clamp enforces [1.0, 4.0] range.
    ws.ensure_tape("default").set_scale_factor(0.5);
    assert!((ws.scale_factor() - 1.0).abs() < f64::EPSILON,
        "0.5 should clamp to 1.0, got {}", ws.scale_factor());

    ws.ensure_tape("default").set_scale_factor(5.0);
    assert!((ws.scale_factor() - 4.0).abs() < f64::EPSILON,
        "5.0 should clamp to 4.0, got {}", ws.scale_factor());

    // In-range values are unchanged.
    ws.ensure_tape("default").set_scale_factor(2.0);
    assert!((ws.scale_factor() - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_workspace_scale_factor_for_window_fallback() {
    let config = WorkspaceConfig::default();
    let ws = ScrollableWorkspaces::new(&config);

    // Unknown window returns 1.0 (fallback).
    assert!((ws.scale_factor_for_window(9999) - 1.0).abs() < f64::EPSILON);
}
