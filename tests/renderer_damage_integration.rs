//! Integration tests for renderer shared-state damage tracking and window stack

use axiom::renderer::{
    add_window_damage_region, add_window_to_stack, clear_frame_damage, get_window_render_order,
    has_pending_damage, mark_window_damaged, raise_window_to_top, remove_window_from_stack,
};

#[test]
fn test_pending_damage_toggle() {
    // Ensure starting state has no pending damage
    assert!(!has_pending_damage(), "initial state should have no pending damage");

    // Add a specific damage region to a window
    add_window_damage_region(42, 10, 20, 100, 50);
    assert!(has_pending_damage(), "damage should be pending after region add");

    // Clear frame damage and verify reset
    clear_frame_damage();
    assert!(
        !has_pending_damage(),
        "no pending damage after clear_frame_damage"
    );

    // Full-window damage
    mark_window_damaged(7);
    assert!(has_pending_damage(), "damage should be pending after full mark");
    clear_frame_damage();
    assert!(!has_pending_damage(), "cleared full damage should reset state");
}

#[test]
fn test_window_stack_public_api_flow() {
    // Add windows to stack and verify order
    add_window_to_stack(1);
    add_window_to_stack(2);
    add_window_to_stack(3);

    let order = get_window_render_order();
    assert_eq!(order, vec![1, 2, 3]);

    // Raise a window to top
    raise_window_to_top(1);
    let order = get_window_render_order();
    assert_eq!(order, vec![2, 3, 1]);

    // Remove a window and verify order and remaining elements
    remove_window_from_stack(3);
    let order = get_window_render_order();
    assert_eq!(order, vec![2, 1]);

    // Clean up remaining windows to avoid interference with other tests
    remove_window_from_stack(2);
    remove_window_from_stack(1);
}

#[test]
fn test_multiple_damage_regions_and_clear() {
    // Multiple regions on a single window
    add_window_damage_region(100, 0, 0, 10, 10);
    add_window_damage_region(100, 20, 0, 10, 10);
    add_window_damage_region(100, 40, 0, 10, 10);
    assert!(has_pending_damage(), "pending damage after multiple regions");

    // Another window full damage
    mark_window_damaged(200);
    assert!(has_pending_damage(), "still pending damage with multiple windows");

    // Clear for next tests
    clear_frame_damage();
    assert!(!has_pending_damage(), "no pending damage after clear");
}
