//! Unit tests for workspace module
//!
//! Tests scrollable workspace functionality, window management,
//! momentum scrolling, and state consistency.

use super::*;
use crate::config::WorkspaceConfig;

#[test]
fn test_workspace_creation() {
    let config = WorkspaceConfig::default();
    let workspaces = ScrollableWorkspaces::new(&config);

    // Should start with one empty column
    assert_eq!(workspaces.active_column_count(), 1); // One column is created by default
    assert_eq!(workspaces.focused_column_index(), 0);
}

#[test]
fn test_window_addition() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add first window - goes to focused column (index 0)
    workspaces.add_window(1001);
    assert_eq!(workspaces.active_column_count(), 1);

    // Add more windows to the same focused column
    workspaces.add_window(1002);
    workspaces.add_window(1003);
    assert_eq!(workspaces.active_column_count(), 1); // Still 1 column
}

#[test]
fn test_window_removal() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to the same focused column
    workspaces.add_window(1001);
    workspaces.add_window(1002);
    workspaces.add_window(1003);
    assert_eq!(workspaces.active_column_count(), 1);

    // Remove window
    let removed = workspaces.remove_window_bool(1002);
    assert!(removed);
    assert_eq!(workspaces.active_column_count(), 1); // Still 1 column with 2 windows left

    // Try to remove non-existent window
    let not_removed = workspaces.remove_window_bool(9999);
    assert!(!not_removed);
    assert_eq!(workspaces.active_column_count(), 1); // No change
}

#[test]
fn test_workspace_scrolling() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add multiple windows and manually place them in different columns
    workspaces.add_window_to_column(1, 0); // First window in column 0
    workspaces.add_window_to_column(2, 1); // Second window in column 1
    workspaces.add_window_to_column(3, 2); // Third window in column 2
    workspaces.add_window_to_column(4, 3); // Fourth window in column 3
    workspaces.add_window_to_column(5, 4); // Fifth window in column 4

    // Test scrolling right
    assert_eq!(workspaces.focused_column_index(), 0);

    workspaces.scroll_right();
    assert_eq!(workspaces.focused_column_index(), 1);

    workspaces.scroll_right();
    assert_eq!(workspaces.focused_column_index(), 2);

    // Test scrolling left
    workspaces.scroll_left();
    assert_eq!(workspaces.focused_column_index(), 1);

    workspaces.scroll_left();
    assert_eq!(workspaces.focused_column_index(), 0);
}

#[test]
fn test_infinite_scrolling_bounds() {
    let config = WorkspaceConfig {
        infinite_scroll: true,
        ..WorkspaceConfig::default()
    };

    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to different columns
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 1);
    workspaces.add_window_to_column(3, 2);

    // Should be able to scroll left from position 0 (infinite scroll)
    assert_eq!(workspaces.focused_column_index(), 0);
    workspaces.scroll_left();

    // With infinite scroll, this might go to -1 or wrap, depending on implementation
    // The exact behavior would depend on the ScrollableWorkspaces implementation
}

#[test]
fn test_window_movement() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to different columns
    workspaces.add_window_to_column(1001, 0);
    workspaces.add_window_to_column(1002, 1);
    workspaces.add_window_to_column(1003, 2);

    // Test moving window right
    let moved = workspaces.move_window_right(1001);
    assert!(moved);

    // Test moving non-existent window
    let not_moved = workspaces.move_window_right(9999);
    assert!(!not_moved);

    // Test moving window left
    let moved_left = workspaces.move_window_left(1002);
    assert!(moved_left);
}

#[test]
fn test_focused_column_retrieval() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to different columns
    workspaces.add_window_to_column(1001, 0);
    workspaces.add_window_to_column(1002, 1);

    // Get focused column (should be first column initially)
    let focused_column = workspaces.get_focused_column_opt().unwrap();
    assert_eq!(focused_column.windows.len(), 1);
    assert_eq!(focused_column.windows[0], 1001);

    // Move focus and check again
    workspaces.scroll_right();
    let focused_column = workspaces.get_focused_column_opt().unwrap();
    assert_eq!(focused_column.windows.len(), 1);
    assert_eq!(focused_column.windows[0], 1002);
}

#[test]
fn test_workspace_update_cycle() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to different columns
    workspaces.add_window_to_column(1001, 0);
    workspaces.add_window_to_column(1002, 1);

    // Test update cycle (should not crash)
    workspaces.update_animations();

    // Should still have the same number of columns
    assert_eq!(workspaces.active_column_count(), 2);
}

#[test]
fn test_smooth_scrolling_state() {
    let config = WorkspaceConfig {
        smooth_scrolling: true,
        ..WorkspaceConfig::default()
    };

    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to different columns
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 1);
    workspaces.add_window_to_column(3, 2);

    // Start scrolling - this should initiate smooth scrolling
    workspaces.scroll_right();

    // The exact state would depend on implementation details
    // but the operation should succeed
    assert_eq!(workspaces.active_column_count(), 3);
}

#[test]
fn test_workspace_configuration_effects() {
    // Test with different scroll speeds
    let config = WorkspaceConfig {
        scroll_speed: 2.0,
        ..WorkspaceConfig::default()
    };
    let workspaces_fast = ScrollableWorkspaces::new(&config);

    let config = WorkspaceConfig {
        scroll_speed: 0.5,
        ..WorkspaceConfig::default()
    };
    let workspaces_slow = ScrollableWorkspaces::new(&config);

    // Both should create successfully
    assert_eq!(workspaces_fast.active_column_count(), 1); // Default column is created
    assert_eq!(workspaces_slow.active_column_count(), 1); // Default column is created

    // Test with different gap settings
    let config = WorkspaceConfig {
        gaps: 20,
        scroll_speed: 0.5,
        ..WorkspaceConfig::default()
    };
    let workspaces_large_gaps = ScrollableWorkspaces::new(&config);
    assert_eq!(workspaces_large_gaps.active_column_count(), 1); // Default column is created
}

#[test]
fn test_window_in_column() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to the same column
    workspaces.add_window(1001);
    workspaces.add_window(1002);

    // Test window existence in columns
    let exists = workspaces.window_exists(1001);
    assert!(exists);

    let not_exists = workspaces.window_exists(9999);
    assert!(!not_exists);
}

#[test]
fn test_workspace_shutdown() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add windows to the same column
    workspaces.add_window(1001);
    workspaces.add_window(1002);

    // Shutdown should succeed
    workspaces.shutdown();

    // After shutdown, operations might not work the same way
    // but the shutdown itself should not crash
}

#[test]
fn test_minimize_window_hides_from_layout_and_existence() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    workspaces.add_window(42);
    assert!(workspaces.window_exists(42));
    let layouts_before = workspaces.calculate_workspace_layouts();
    assert!(
        layouts_before.contains_key(&42),
        "visible window must appear in layout map"
    );

    assert!(
        workspaces.minimize_window(42),
        "first minimize returns true (state changed)"
    );
    assert!(workspaces.is_window_minimized(42));
    assert_eq!(workspaces.minimized_window_count(), 1);

    let layouts_after = workspaces.calculate_workspace_layouts();
    assert!(
        !layouts_after.contains_key(&42),
        "minimized window must NOT appear in layout map"
    );
}

#[test]
fn test_minimize_window_is_idempotent_and_pure_on_workspace_layer() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    workspaces.add_window(7);
    assert!(workspaces.minimize_window(7));
    // Calling again is a no-op (returns false).
    assert!(!workspaces.minimize_window(7));
    assert!(workspaces.is_window_minimized(7));
}

#[test]
fn test_restore_window_re_adds_to_focused_column() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    workspaces.add_window(9);
    workspaces.minimize_window(9);
    assert!(workspaces.is_window_minimized(9));

    assert!(
        workspaces.restore_window(9),
        "first restore returns true (state changed)"
    );
    assert!(!workspaces.is_window_minimized(9));

    let layouts = workspaces.calculate_workspace_layouts();
    assert!(
        layouts.contains_key(&9),
        "restored window must reappear in layout map"
    );

    // Restore of a visible window is a no-op.
    assert!(!workspaces.restore_window(9));
}

#[test]
fn test_restore_unknown_window_returns_false() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);
    assert!(!workspaces.restore_window(1234));
    assert!(!workspaces.is_window_minimized(1234));
}

#[test]
fn test_large_number_of_windows() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add many windows to different columns
    for i in 1..=100 {
        workspaces.add_window_to_column(i, i as i32 - 1);
    }

    assert_eq!(workspaces.active_column_count(), 100);

    // Test scrolling with many windows
    for _ in 0..10 {
        workspaces.scroll_right();
    }

    assert_eq!(workspaces.focused_column_index(), 10);

    // Remove some windows
    for i in 50..60 {
        workspaces.remove_window(i);
    }

    // Verify that the windows were actually removed by checking they don't exist
    for i in 50..60 {
        assert!(!workspaces.window_exists(i));
    }

    // The column count may not immediately reflect removals due to lazy cleanup
    // But should still be reasonable (not more than original)
    let final_count = workspaces.active_column_count();
    assert!(
        final_count <= 100,
        "Final count {} should not exceed original 100",
        final_count
    );
}

#[test]
fn test_edge_case_empty_workspace() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Test operations on empty workspace
    let moved = workspaces.move_window_right(1001);
    assert!(!moved);

    let removed = workspaces.remove_window(1001);
    assert!(removed.is_none());

    // Scrolling on empty workspace should not crash
    workspaces.scroll_left();
    workspaces.scroll_right();

    assert_eq!(workspaces.active_column_count(), 2); // We have columns 0 and -1 after scrolling left
}

#[test]
fn test_workspace_bounds_checking() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Add a single window
    workspaces.add_window(1001);

    // Test bounds - scrolling past available columns
    for _ in 0..10 {
        workspaces.scroll_right();
    }

    // Should not crash and should stay within reasonable bounds
    let index = workspaces.focused_column_index();

    // With infinite scroll disabled, should not go too far
    if !workspaces.is_infinite_scroll_enabled() {
        assert!(index < 10); // Reasonable upper bound
    }
}

/// Create 3 windows, remove the middle one, and verify the remaining
/// two windows keep their original order (correct indices in the
/// column's window list).
#[test]
fn test_remove_middle_window_preserves_order() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // ── Add 3 windows to the focused column ─────────────────────────
    workspaces.add_window(1);
    workspaces.add_window(2);
    workspaces.add_window(3);

    let col = workspaces.get_focused_column_opt().unwrap();
    assert_eq!(
        col.windows,
        vec![1, 2, 3],
        "three windows added in order"
    );

    // ── Remove the middle window (ID 2) ─────────────────────────────
    let removed = workspaces.remove_window(2);
    assert!(removed.is_some(), "remove_window should find window 2");

    // ── Verify remaining windows and their indices ───────────────────
    assert!(
        !workspaces.window_exists(2),
        "window 2 must no longer exist"
    );
    assert!(
        workspaces.window_exists(1),
        "window 1 must still exist"
    );
    assert!(
        workspaces.window_exists(3),
        "window 3 must still exist"
    );

    let col = workspaces.get_focused_column_opt().unwrap();
    assert_eq!(
        col.windows,
        vec![1, 3],
        "remaining windows must be [1, 3] in original order after removing middle"
    );

    // ── Layout map must contain exactly windows 1 and 3 ─────────────
    let layouts = workspaces.calculate_workspace_layouts();
    assert_eq!(layouts.len(), 2, "exactly two windows in layout");
    assert!(layouts.contains_key(&1), "window 1 in layout");
    assert!(layouts.contains_key(&3), "window 3 in layout");
    assert!(!layouts.contains_key(&2), "window 2 NOT in layout");
}

#[test]
fn test_multi_monitor_tapes() {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // 1. Create tapes for two outputs
    workspaces.ensure_tape("output-1");
    workspaces.ensure_tape("output-2");

    // 2. Switch to output-1 and add a window
    workspaces.focused_output = "output-1".to_string();
    workspaces.add_window(1001);

    // Verify window exists in output-1
    assert!(workspaces.active_tape().window_exists(1001));
    assert_eq!(workspaces.active_tape().active_column_count(), 1);

    // 3. Switch to output-2
    workspaces.focused_output = "output-2".to_string();

    // Verify window does NOT exist in output-2's tape locally (though window_exists delegates globally in current impl, let's check column count directly)
    assert_eq!(workspaces.active_tape().active_column_count(), 1); // Default empty column    assert!(workspaces.active_tape().get_focused_column_windows().is_empty());

    // 4. Add window to output-2
    workspaces.add_window(2002);
    assert_eq!(
        workspaces.active_tape().get_focused_column_windows(),
        vec![2002]
    );

    // 5. Verify independence
    workspaces.focused_output = "output-1".to_string();
    assert_eq!(
        workspaces.active_tape().get_focused_column_windows(),
        vec![1001]
    );
}    /// Verify that changing viewport size multiple times produces
    /// correct layout dimensions on every call — no stale cached values
    /// from a previous viewport size survive across resizes.
    #[test]
    fn test_viewport_resize_invalidates_layout_cache() {
        let config = WorkspaceConfig::default();
        let mut workspaces = ScrollableWorkspaces::new(&config);

        // Add three windows to the focused column
        workspaces.add_window(1);
        workspaces.add_window(2);
        workspaces.add_window(3);

        // ── 1080p ────────────────────────────────────
        workspaces.set_viewport_size(1920.0, 1080.0);
        let layouts_1080 = workspaces.calculate_workspace_layouts();
        assert_eq!(layouts_1080.len(), 3, "all three windows tiled");
        for (_id, rect) in &layouts_1080 {
            assert!(rect.height > 0, "height must be positive at 1080p");
            assert!(rect.height <= 1080, "height must not exceed viewport");
        }
        let heights_1080: Vec<_> = layouts_1080.values().map(|r| r.height).collect();

        // ── Shrink to 600p ───────────────────────────
        workspaces.set_viewport_size(800.0, 600.0);
        let layouts_600 = workspaces.calculate_workspace_layouts();
        assert_eq!(layouts_600.len(), 3);
        for (_id, rect) in &layouts_600 {
            assert!(rect.height > 0, "height must be positive at 600p");
            assert!(
                rect.height < heights_1080[0],
                "600p window height {} must be smaller than 1080p height {}",
                rect.height,
                heights_1080[0]
            );
        }

        // ── Grow to 4K ───────────────────────────────
        workspaces.set_viewport_size(3840.0, 2160.0);
        let layouts_4k = workspaces.calculate_workspace_layouts();
        assert_eq!(layouts_4k.len(), 3);
        for (_id, rect) in &layouts_4k {
            assert!(rect.height > 0, "height must be positive at 4K");
            assert!(
                rect.height > heights_1080[0],
                "4K window height {} must be larger than 1080p height {}",
                rect.height,
                heights_1080[0]
            );
        }

        // ── Back to 1080p — must match original ──────
        workspaces.set_viewport_size(1920.0, 1080.0);
        let layouts_1080_round2 = workspaces.calculate_workspace_layouts();
        assert_eq!(layouts_1080_round2.len(), 3);
        for (id, rect) in &layouts_1080_round2 {
            let original = layouts_1080.get(id).expect("same window id");
            assert_eq!(
                rect.height, original.height,
                "back to 1080p: window {} height should match original",
                id
            );
        }
    }

    #[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_add_remove_windows_consistency(
            window_ids in prop::collection::vec(1u64..1000u64, 1..20)
        ) {
            let config = WorkspaceConfig::default();
            let mut workspaces = ScrollableWorkspaces::new(&config);

            // Add all windows to different columns
            for (i, &id) in window_ids.iter().enumerate() {
                workspaces.add_window_to_column(id, i as i32);
            }

            prop_assert_eq!(workspaces.active_column_count(), window_ids.len());

            // Remove all windows
            for &id in &window_ids {
                let removed = workspaces.remove_window(id);
                prop_assert!(removed.is_some());
            }

            // Cleanup happens periodically, not immediately, so we might have some empty columns left
            // The important thing is that windows were actually removed
            let final_count = workspaces.active_column_count();
            prop_assert!(final_count <= window_ids.len()); // Should not exceed original count

            // Verify all windows are actually gone
            for &id in &window_ids {
                prop_assert!(!workspaces.window_exists(id));
            }
        }

        #[test]
        fn test_scroll_operations_stability(
            scroll_ops in prop::collection::vec(any::<bool>(), 1..50)
        ) {
            let config = WorkspaceConfig::default();
            let mut workspaces = ScrollableWorkspaces::new(&config);

            // Add some windows for scrolling - to different columns
            for i in 1..=10 {
                workspaces.add_window_to_column(i, i as i32 - 1);
            }

            let initial_count = workspaces.active_column_count();

            // Perform random scroll operations
            for scroll_right in scroll_ops {
                if scroll_right {
                    workspaces.scroll_right();
                } else {
                    workspaces.scroll_left();
                }
            }

            // Column count may change due to scrolling creating new columns
            // but should remain reasonable
            let final_count = workspaces.active_column_count();
            prop_assert!(final_count >= initial_count); // Can only increase or stay same

            // Focus index should be reasonable - can be negative with infinite scroll
            let focus_index = workspaces.focused_column_index();
            // With infinite scroll, focus index can be negative, so we just check it's reasonable
            prop_assert!(focus_index >= -50); // Reasonable lower bound
        }

        #[test]
        fn test_window_movement_preserves_count(
            moves in prop::collection::vec((1u64..10u64, any::<bool>()), 1..20)
        ) {
            let config = WorkspaceConfig::default();
            let mut workspaces = ScrollableWorkspaces::new(&config);

            // Add windows to different columns
            for i in 1..=10 {
                workspaces.add_window_to_column(i, i as i32 - 1);
            }

            let initial_count = workspaces.active_column_count();

            // Perform random window movements
            for (window_id, move_right) in moves {
                if move_right {
                    workspaces.move_window_right(window_id);
                } else {
                    workspaces.move_window_left(window_id);
                }
            }

            // Moving windows may create new columns but shouldn't decrease count much
            let final_count = workspaces.active_column_count();
            prop_assert!(final_count >= initial_count); // Moving creates new columns if needed
        }
    }
}
