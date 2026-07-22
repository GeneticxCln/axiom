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
