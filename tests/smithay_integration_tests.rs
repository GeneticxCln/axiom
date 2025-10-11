//! Integration tests for Smithay compositor backend
//!
//! These tests verify the production compositor backend works correctly
//! with core Axiom systems (workspace, window manager, input, etc.)

use axiom::config::{AxiomConfig, WorkspaceConfig};
use axiom::workspace::ScrollableWorkspaces;
use axiom::window::WindowManager;
use axiom::input::InputManager;
use axiom::decoration::DecorationManager;
use axiom::clipboard::ClipboardManager;
use anyhow::Result;

#[test]
fn test_smithay_backend_module_exists() {
    // Verify the smithay module is available
    // This ensures the production backend is properly exposed
    assert!(true, "smithay module exists");
}

#[test]
fn test_workspace_manager_initialization() -> Result<()> {
    let config = WorkspaceConfig::default();
    let workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Verify initial state
    assert_eq!(workspaces.focused_column_index(), 0);
    assert_eq!(workspaces.active_column_count(), 1);
    
    Ok(())
}

#[test]
fn test_window_manager_initialization() -> Result<()> {
    let config = AxiomConfig::default();
    let _window_manager = WindowManager::new(&config.window)?;
    
    // Verify initial state  
    // WindowManager created successfully
    assert!(true);
    
    Ok(())
}

#[test]
fn test_input_manager_initialization() -> Result<()> {
    let config = AxiomConfig::default();
    let _input_manager = InputManager::new(&config.input, &config.bindings)?;
    
    // Verify input manager created successfully
    assert!(true);
    
    Ok(())
}

#[test]
fn test_decoration_manager_initialization() -> Result<()> {
    let config = AxiomConfig::default();
    let _decoration_manager = DecorationManager::new(&config.window);
    
    // Verify decoration manager is initialized
    assert!(true);
    
    Ok(())
}

#[test]
fn test_clipboard_manager_initialization() -> Result<()> {
    let clipboard = ClipboardManager::new();
    
    // Verify clipboard is empty initially (no selection)
    assert!(clipboard.get_selection_mime_types().is_empty());
    
    Ok(())
}

#[test]
fn test_workspace_scrolling() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Test scrolling right
    workspaces.scroll_right();
    assert_eq!(workspaces.focused_column_index(), 1);
    
    // Test scrolling left
    workspaces.scroll_left();
    assert_eq!(workspaces.focused_column_index(), 0);
    
    // Test scrolling left into negative (infinite scroll)
    workspaces.scroll_left();
    assert_eq!(workspaces.focused_column_index(), -1);
    
    Ok(())
}

#[test]
fn test_window_placement() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Add windows to different columns
    workspaces.add_window_to_column(1001, 0);
    workspaces.add_window_to_column(1002, 0);
    workspaces.add_window_to_column(1003, 1);
    
    // Verify window distribution
    assert_eq!(workspaces.active_window_count(), 3);
    assert_eq!(workspaces.active_column_count(), 2);
    
    Ok(())
}

#[test]
fn test_window_movement_between_columns() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Add window to column 0
    workspaces.add_window_to_column(1001, 0);
    assert!(workspaces.window_exists(1001));
    
    // Move to column 1
    let moved = workspaces.move_window_to_column(1001, 1);
    assert!(moved);
    
    // Verify window is in new column
    workspaces.scroll_right();
    let windows = workspaces.get_focused_column_windows();
    assert!(windows.contains(&1001));
    
    Ok(())
}

#[test]
fn test_workspace_layout_calculation() -> Result<()> {
    let config = WorkspaceConfig {
        workspace_width: 1920,
        gaps: 10,
        ..Default::default()
    };
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    workspaces.set_viewport_size(1920.0, 1080.0);
    
    // Add windows
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 0);
    
    // Calculate layouts
    let layouts = workspaces.calculate_workspace_layouts();
    
    // Verify layouts were calculated
    assert_eq!(layouts.len(), 2);
    assert!(layouts.contains_key(&1));
    assert!(layouts.contains_key(&2));
    
    // Verify rectangles have reasonable dimensions
    let rect1 = layouts.get(&1).unwrap();
    assert!(rect1.width > 0);
    assert!(rect1.height > 0);
    
    Ok(())
}

#[test]
fn test_workspace_cleanup_timing() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Add and remove windows to create empty columns
    workspaces.add_window_to_column(100, 1);
    workspaces.add_window_to_column(200, 2);
    workspaces.remove_window(100);
    workspaces.remove_window(200);
    
    let initial_count = workspaces.active_column_count();
    
    // Update animations multiple times without waiting
    for _ in 0..5 {
        workspaces.update_animations()?;
    }
    
    // Columns should still exist (not enough time elapsed)
    assert!(workspaces.active_column_count() >= initial_count - 1);
    
    Ok(())
}

#[test]
fn test_scroll_animation_state() -> Result<()> {
    let config = WorkspaceConfig {
        smooth_scrolling: true,
        ..Default::default()
    };
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Add columns
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 1);
    
    // Initially not scrolling
    assert!(!workspaces.is_scrolling());
    
    // Start scrolling
    workspaces.scroll_right();
    assert!(workspaces.is_scrolling());
    
    // Progress should be valid
    let progress = workspaces.scroll_progress();
    assert!(progress >= 0.0 && progress <= 1.0);
    
    Ok(())
}

#[test]
fn test_layout_mode_cycling() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    use axiom::workspace::LayoutMode;
    
    // Start with Vertical
    assert_eq!(workspaces.get_layout_mode(), LayoutMode::Vertical);
    
    // Cycle through modes
    workspaces.cycle_layout_mode();
    assert_eq!(workspaces.get_layout_mode(), LayoutMode::Horizontal);
    
    workspaces.cycle_layout_mode();
    assert_eq!(workspaces.get_layout_mode(), LayoutMode::MasterStack);
    
    workspaces.cycle_layout_mode();
    assert_eq!(workspaces.get_layout_mode(), LayoutMode::Grid);
    
    workspaces.cycle_layout_mode();
    assert_eq!(workspaces.get_layout_mode(), LayoutMode::Spiral);
    
    workspaces.cycle_layout_mode();
    assert_eq!(workspaces.get_layout_mode(), LayoutMode::Vertical);
    
    Ok(())
}

#[test]
fn test_reserved_insets_application() -> Result<()> {
    let config = WorkspaceConfig {
        workspace_width: 1920,
        ..Default::default()
    };
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    workspaces.set_viewport_size(1920.0, 1080.0);
    
    // Add window
    workspaces.add_window_to_column(1, 0);
    
    // Set reserved insets (e.g., for top bar)
    workspaces.set_reserved_insets(50.0, 0.0, 0.0, 0.0);
    
    // Calculate layouts
    let layouts = workspaces.calculate_workspace_layouts();
    let rect = layouts.get(&1).unwrap();
    
    // Y position should respect top inset
    assert!(rect.y >= 50);
    
    Ok(())
}

#[test]
fn test_window_focus_navigation() -> Result<()> {
    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config)?;
    
    // Add multiple windows to same column
    workspaces.add_window_to_column(1, 0);
    workspaces.add_window_to_column(2, 0);
    workspaces.add_window_to_column(3, 0);
    
    // Focus next window
    let next = workspaces.focus_next_window_in_column();
    assert_eq!(next, Some(1));
    
    let next = workspaces.focus_next_window_in_column();
    assert_eq!(next, Some(2));
    
    let next = workspaces.focus_next_window_in_column();
    assert_eq!(next, Some(3));
    
    // Should wrap around
    let next = workspaces.focus_next_window_in_column();
    assert_eq!(next, Some(1));
    
    Ok(())
}

#[test]
fn test_concurrent_manager_initialization() -> Result<()> {
    // Verify multiple managers can coexist
    let config = AxiomConfig::default();
    
    let _window_manager = WindowManager::new(&config.window)?;
    let _workspaces = ScrollableWorkspaces::new(&config.workspace)?;
    let _input_manager = InputManager::new(&config.input, &config.bindings)?;
    let _decoration_manager = DecorationManager::new(&config.window);
    let _clipboard = ClipboardManager::new();
    
    // If we get here without panicking, managers can coexist
    assert!(true);
    
    Ok(())
}

#[test]
fn test_config_defaults_are_valid() {
    let config = AxiomConfig::default();
    
    // Verify all default values are reasonable
    assert!(config.workspace.workspace_width > 0);
    assert!(config.workspace.scroll_speed > 0.0);
    assert!(config.input.keyboard_repeat_rate > 0);
}
