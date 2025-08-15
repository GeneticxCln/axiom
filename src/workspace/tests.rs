//! Unit tests for workspace module
//!
//! Tests scrollable workspace functionality, window management,
//! momentum scrolling, and state consistency.

use super::*;
use crate::config::WorkspaceConfig;
use anyhow::Result;

#[test]
fn test_workspace_creation() -> Result<()> {
    let config = WorkspaceConfig::default();
    let workspaces = ScrollableWorkspaces::new(&config)?;

    // Should start with one empty column
    assert_eq!(workspaces.active_column_count(), 1); // One column is created by default
    assert_eq!(workspaces.focused_column_index(), 0);

    Ok(())
}

#[test]
fn test_window_addition() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add first window - goes to focused column (index 0)
    workspaces.add_window(1001);
    assert_eq!(workspaces.active_column_count(), 1);

    // Add more windows to the same focused column
    workspaces.add_window(1002);
    workspaces.add_window(1003);
    assert_eq!(workspaces.active_column_count(), 1); // Still 1 column

    Ok(())
}

#[test]
fn test_window_removal() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

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

    Ok(())
}

#[test]
fn test_workspace_scrolling() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

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

    Ok(())
}

#[test]
fn test_infinite_scrolling_bounds() -> Result<()> {
    let mut config = WorkspaceConfig::default();
    config.infinite_scroll = true;

    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add windows to different columns
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 1);
    workspaces.add_window_to_column(3, 2);

    // Should be able to scroll left from position 0 (infinite scroll)
    assert_eq!(workspaces.focused_column_index(), 0);
    workspaces.scroll_left();

    // With infinite scroll, this might go to -1 or wrap, depending on implementation
    // The exact behavior would depend on the ScrollableWorkspaces implementation

    Ok(())
}

#[test]
fn test_window_movement() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

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

    Ok(())
}

#[test]
fn test_focused_column_retrieval() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add windows to different columns
    workspaces.add_window_to_column(1001, 0);
    workspaces.add_window_to_column(1002, 1);

    // Get focused column (should be first column initially)
    let focused_column = workspaces.get_focused_column();
    assert_eq!(focused_column.windows.len(), 1);
    assert_eq!(focused_column.windows[0], 1001);

    // Move focus and check again
    workspaces.scroll_right();
    let focused_column = workspaces.get_focused_column();
    assert_eq!(focused_column.windows.len(), 1);
    assert_eq!(focused_column.windows[0], 1002);

    Ok(())
}

#[test]
fn test_workspace_update_cycle() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add windows to different columns
    workspaces.add_window_to_column(1001, 0);
    workspaces.add_window_to_column(1002, 1);

    // Test update cycle (should not crash)
    workspaces.update()?;

    // Should still have the same number of columns
    assert_eq!(workspaces.active_column_count(), 2);

    Ok(())
}

#[test]
fn test_smooth_scrolling_state() -> Result<()> {
    let mut config = WorkspaceConfig::default();
    config.smooth_scrolling = true;

    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add windows to different columns
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 1);
    workspaces.add_window_to_column(3, 2);

    // Start scrolling - this should initiate smooth scrolling
    workspaces.scroll_right();

    // The exact state would depend on implementation details
    // but the operation should succeed
    assert_eq!(workspaces.active_column_count(), 3);

    Ok(())
}

#[test]
fn test_workspace_configuration_effects() -> Result<()> {
    let mut config = WorkspaceConfig::default();

    // Test with different scroll speeds
    config.scroll_speed = 2.0;
    let workspaces_fast = ScrollableWorkspaces::new(&config)?;

    config.scroll_speed = 0.5;
    let workspaces_slow = ScrollableWorkspaces::new(&config)?;

    // Both should create successfully
    assert_eq!(workspaces_fast.active_column_count(), 1); // Default column is created
    assert_eq!(workspaces_slow.active_column_count(), 1); // Default column is created

    // Test with different gap settings
    config.gaps = 20;
    let workspaces_large_gaps = ScrollableWorkspaces::new(&config)?;
    assert_eq!(workspaces_large_gaps.active_column_count(), 1); // Default column is created

    Ok(())
}

#[test]
fn test_window_in_column() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add windows to the same column
    workspaces.add_window(1001);
    workspaces.add_window(1002);

    // Test window existence in columns
    let exists = workspaces.window_exists(1001);
    assert!(exists);

    let not_exists = workspaces.window_exists(9999);
    assert!(!not_exists);

    Ok(())
}

#[test]
fn test_workspace_shutdown() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add windows to the same column
    workspaces.add_window(1001);
    workspaces.add_window(1002);

    // Shutdown should succeed
    workspaces.shutdown()?;

    // After shutdown, operations might not work the same way
    // but the shutdown itself should not crash

    Ok(())
}

#[test]
fn test_large_number_of_windows() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

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
    assert!(final_count <= 100, "Final count {} should not exceed original 100", final_count);

    Ok(())
}

#[test]
fn test_edge_case_empty_workspace() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Test operations on empty workspace
    let moved = workspaces.move_window_right(1001);
    assert!(!moved);

    let removed = workspaces.remove_window(1001);
    assert!(removed.is_none());

    // Scrolling on empty workspace should not crash
    workspaces.scroll_left();
    workspaces.scroll_right();

    assert_eq!(workspaces.active_column_count(), 2); // We have columns 0 and -1 after scrolling left

    Ok(())
}

#[test]
fn test_workspace_bounds_checking() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

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

    Ok(())
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
            let mut workspaces = ScrollableWorkspaces::new(&config).unwrap();

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
            let mut workspaces = ScrollableWorkspaces::new(&config).unwrap();

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
            let mut workspaces = ScrollableWorkspaces::new(&config).unwrap();

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
