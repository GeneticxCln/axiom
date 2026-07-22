//! Multi-window stress test.
//!
//! Validates the compositor's behavior under load with many windows in various
//! states: mapped, minimized, fullscreen. Tests rapid creation/destruction
//! cycles and workspace switching. All tests are headless (noop backend or
//! direct subsystem access) so they run in CI without a display.

use anyhow::Result;
use parking_lot::RwLock;
use std::sync::Arc;

use axiom::{
    compositor::AxiomCompositor,
    config::AxiomConfig,
    input::InputManager,
    ipc::AxiomIPCServer,
    window::WindowManager,
    workspace::ScrollableWorkspaces,
};

// ============================================================================
// Helpers
// ============================================================================

/// Construct a fully-initialized test compositor with noop backend.
#[allow(clippy::type_complexity, clippy::arc_with_non_send_sync)]
fn make_test_compositor(
    config: AxiomConfig,
) -> Result<(
    AxiomCompositor,
    Arc<RwLock<ScrollableWorkspaces>>,
    Arc<RwLock<WindowManager>>,
    Arc<RwLock<InputManager>>,
)> {
    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let ipc_server = AxiomIPCServer::new();

    let mut config = config;
    config.backend.kind = "noop".to_string();

    let compositor = AxiomCompositor::new(
        config,
        false, // windowed
        workspace_manager.clone(),
        window_manager.clone(),
        input_manager.clone(),
        ipc_server,
    )?;

    Ok((compositor, workspace_manager, window_manager, input_manager))
}

/// Add `count` windows to the compositor, returning their IDs.
fn add_n_windows(compositor: &mut AxiomCompositor, count: usize) -> Vec<u64> {
    (0..count)
        .map(|i| compositor.add_window(format!("Stress Window {}", i)))
        .collect()
}

// ============================================================================
// Subsystem-level tests (no compositor — no serial needed)
// ============================================================================

/// Test creating 50+ windows in the workspace manager directly.
#[test]
fn test_stress_50_windows_workspace() -> Result<()> {
    let config = AxiomConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config.workspace);
    let window_config = axiom::config::WindowConfig::default();
    let mut wm = WindowManager::new(&window_config);

    let count = 50;
    let ids: Vec<u64> = (0..count)
        .map(|i| {
            let id = wm.add_window(format!("Win {}", i));
            workspaces.add_window(id);
            id
        })
        .collect();

    // All windows should be in the focused column
    assert_eq!(workspaces.active_column_count(), 1);
    let column_windows = workspaces.get_focused_column_windows();
    assert_eq!(column_windows.len(), count, "all {} windows in focused column", count);

    // Layouts should be calculable
    let layouts = workspaces.calculate_workspace_layouts();
    assert_eq!(layouts.len(), count, "all {} windows have layouts", count);

    // Each layout should have positive dimensions
    for rect in layouts.values() {
        assert!(rect.width > 0, "window width should be positive");
        assert!(rect.height > 0, "window height should be positive");
    }

    // Clean up
    for id in ids {
        workspaces.remove_window(id);
        wm.remove_window(id);
    }
    wm.shutdown();
    workspaces.shutdown();

    Ok(())
}

/// Test windows with various states: minimized, fullscreen, mixed.
#[test]
fn test_stress_window_states() -> Result<()> {
    let config = AxiomConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config.workspace);
    let window_config = axiom::config::WindowConfig::default();
    let mut wm = WindowManager::new(&window_config);

    // Create 30 windows
    let ids: Vec<u64> = (0..30)
        .map(|i| {
            let id = wm.add_window(format!("Win {}", i));
            workspaces.add_window(id);
            id
        })
        .collect();

    // Minimize 10 windows (indices 0..10)
    for &id in ids.iter().take(10) {
        assert!(workspaces.minimize_window(id), "minimize window {}", id);
        assert!(wm.minimize_window(id), "wm minimize window {}", id);
    }

    // Fullscreen 5 windows (indices 10..15)
    for &id in ids.iter().skip(10).take(5) {
        wm.toggle_fullscreen(id);
    }

    // Verify minimized windows
    for &id in ids.iter().take(10) {
        assert!(workspaces.is_window_minimized(id), "window {} should be minimized", id);
        assert!(wm.is_minimized(id), "wm: window {} should be minimized", id);
    }

    // Fullscreen windows should NOT be minimized
    for &id in ids.iter().skip(10).take(5) {
        assert!(!workspaces.is_window_minimized(id), "fullscreen window {} should not be minimized", id);
    }

    // Layouts should exclude minimized windows
    let layouts = workspaces.calculate_workspace_layouts();
    // 30 total - 10 minimized = 20 visible
    assert_eq!(layouts.len(), 20, "10 minimized + 20 visible = 30 total");

    // Restore minimized windows
    for &id in ids.iter().take(10) {
        assert!(workspaces.restore_window(id), "restore window {}", id);
        assert!(wm.restore_window(id), "wm restore window {}", id);
    }

    // After restore, all 30 windows should have layouts
    let layouts = workspaces.calculate_workspace_layouts();
    assert_eq!(layouts.len(), 30, "all windows restored");

    // Clean up
    for id in ids {
        workspaces.remove_window(id);
        wm.remove_window(id);
    }
    wm.shutdown();
    workspaces.shutdown();

    Ok(())
}

/// Test rapid window creation/destruction cycles.
#[test]
fn test_stress_rapid_create_destroy() -> Result<()> {
    let config = AxiomConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config.workspace);
    let window_config = axiom::config::WindowConfig::default();
    let mut wm = WindowManager::new(&window_config);

    // 3 cycles of 100 create/destroy
    for cycle in 0..3 {
        let mut ids = Vec::with_capacity(100);

        for i in 0..100 {
            let id = wm.add_window(format!("Cycle{}-Win{}", cycle, i));
            workspaces.add_window(id);
            ids.push(id);
        }

        assert_eq!(
            workspaces.get_focused_column_windows().len(),
            100,
            "cycle {}: all 100 windows created",
            cycle
        );

        // Remove in reverse order
        for id in ids.into_iter().rev() {
            workspaces.remove_window(id);
            wm.remove_window(id);
        }

        assert_eq!(
            workspaces.get_focused_column_windows().len(),
            0,
            "cycle {}: all windows removed",
            cycle
        );

        // Layouts should be empty
        let layouts = workspaces.calculate_workspace_layouts();
        assert!(layouts.is_empty(), "cycle {}: no layouts after removal", cycle);
    }

    wm.shutdown();
    workspaces.shutdown();

    Ok(())
}

/// Test workspace switching with many windows distributed across columns.
#[test]
fn test_stress_workspace_switch() -> Result<()> {
    let config = AxiomConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config.workspace);
    let window_config = axiom::config::WindowConfig::default();
    let mut wm = WindowManager::new(&window_config);

    // Create windows directly into specific columns.
    // WindowManager IDs auto-increment: 1, 2, 3, ...
    let mut ids = Vec::new();

    // Column 0: 30 windows (indices 0..30)
    for i in 0..30 {
        let id = wm.add_window(format!("Win {}", i));
        workspaces.add_window_to_column(id, 0);
        ids.push(id);
    }

    // Column 1: 15 windows (indices 30..44)
    for i in 30..45 {
        let id = wm.add_window(format!("Win {}", i));
        workspaces.add_window_to_column(id, 1);
        ids.push(id);
    }

    // Column 2: 15 windows (indices 45..59)
    for i in 45..60 {
        let id = wm.add_window(format!("Win {}", i));
        workspaces.add_window_to_column(id, 2);
        ids.push(id);
    }

    assert_eq!(ids.len(), 60, "total 60 windows created");

    // Total: 3 columns (0, 1, 2)
    // Column 0: 30 windows, Column 1: 15, Column 2: 15

    // Focused column starts at 0
    {
        let column_windows = workspaces.get_focused_column_windows();
        assert_eq!(column_windows.len(), 30, "column 0 should have 30 windows");
    }

    // Scroll to column 1
    workspaces.scroll_right();
    {
        let column_windows = workspaces.get_focused_column_windows();
        assert_eq!(column_windows.len(), 15, "column 1 should have 15 windows");
    }

    // Scroll to column 2
    workspaces.scroll_right();
    {
        let column_windows = workspaces.get_focused_column_windows();
        assert_eq!(column_windows.len(), 15, "column 2 should have 15 windows");
    }

    // Scroll back to column 0
    workspaces.scroll_left();
    workspaces.scroll_left();
    {
        let column_windows = workspaces.get_focused_column_windows();
        assert_eq!(column_windows.len(), 30, "column 0 should have 30 windows");
    }

    // Layouts should still be calculable
    let layouts = workspaces.calculate_workspace_layouts();
    assert!(layouts.len() <= 60, "layouts should not exceed total windows");

    // Clean up
    for id in ids {
        workspaces.remove_window(id);
        wm.remove_window(id);
    }
    wm.shutdown();
    workspaces.shutdown();

    Ok(())
}

// ============================================================================
// Compositor-level tests (requires serial_test due to socket binding)
// ============================================================================

/// Stress test: 50 windows through the compositor, verify lifecycle.
#[test]
#[serial_test::serial]
fn test_stress_compositor_50_windows() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, workspace_manager, window_manager, _input_manager) =
        make_test_compositor(config)?;

    let ids = add_n_windows(&mut compositor, 50);

    // All windows should be tracked in the window manager
    {
        let wm = window_manager.read();
        assert_eq!(wm.window_count(), 50);
        for &id in &ids {
            assert!(wm.get_window(id).is_some(), "window {} should exist", id);
        }
    }

    // All windows should be in the workspace
    {
        let ws = workspace_manager.read();
        let column_windows = ws.get_focused_column_windows();
        assert_eq!(column_windows.len(), 50);
    }

    // Layouts should be calculable
    {
        let ws = workspace_manager.read();
        let layouts = ws.calculate_workspace_layouts();
        assert_eq!(layouts.len(), 50);
    }

    // Ticking should work fine
    assert!(compositor.tick_for_test().is_ok(), "tick with 50 windows should succeed");

    // Remove all windows
    for id in ids {
        compositor.remove_window(id);
    }

    // Verify empty
    {
        let wm = window_manager.read();
        assert_eq!(wm.window_count(), 0);
    }

    Ok(())
}

/// Stress test: compositor lifecycle with rapid creation/destruction.
#[test]
#[serial_test::serial]
fn test_stress_compositor_rapid_cycle() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, workspace_manager, window_manager, _input_manager) =
        make_test_compositor(config)?;

    // 5 cycles of 20 windows
    for cycle in 0..5 {
        let ids = add_n_windows(&mut compositor, 20);

        // Tick between creation and destruction
        assert!(compositor.tick_for_test().is_ok(), "tick after cycle {}", cycle);

        for id in ids {
            compositor.remove_window(id);
        }

        assert!(compositor.tick_for_test().is_ok(), "tick after removal cycle {}", cycle);

        // Verify empty
        {
            let wm = window_manager.read();
            assert_eq!(wm.window_count(), 0, "cycle {}: all windows removed", cycle);
        }
        {
            let ws = workspace_manager.read();
            assert_eq!(ws.get_focused_column_windows().len(), 0, "cycle {}: no windows in workspace", cycle);
        }
    }

    Ok(())
}

/// Stress test: minimize/restore/fullscreen cycles through the compositor.
#[test]
#[serial_test::serial]
fn test_stress_compositor_state_transitions() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, _workspace_manager, window_manager, _input_manager) =
        make_test_compositor(config)?;

    let ids = add_n_windows(&mut compositor, 30);

    // Minimize 10 windows
    for &id in ids.iter().take(10) {
        assert!(compositor.minimize_window(id), "minimize window {}", id);
    }

    // Verify minimized state
    {
        let wm = window_manager.read();
        for &id in ids.iter().take(10) {
            assert!(wm.is_minimized(id), "window {} should be minimized", id);
        }
        for &id in ids.iter().skip(10) {
            assert!(!wm.is_minimized(id), "window {} should not be minimized", id);
        }
    }

    // Tick with minimized windows
    assert!(compositor.tick_for_test().is_ok(), "tick with minimized windows");

    // Restore all minimized windows
    for &id in ids.iter().take(10) {
        assert!(compositor.restore_window(id), "restore window {}", id);
    }

    // Fullscreen toggle on 5 windows
    for &id in ids.iter().skip(20).take(5) {
        compositor.toggle_fullscreen(id);
    }

    // Tick with fullscreen windows
    assert!(compositor.tick_for_test().is_ok(), "tick with fullscreen windows");

    // Verify fullscreen state
    {
        let wm = window_manager.read();
        for &id in ids.iter().skip(20).take(5) {
            let win = wm.get_window(id).expect("window should exist");
            assert!(win.properties.fullscreen, "window {} should be fullscreen", id);
        }
    }

    // Clean up
    for id in ids {
        compositor.remove_window(id);
    }

    Ok(())
}

/// Stress test: workspace switching through the compositor with many windows.
#[test]
#[serial_test::serial]
fn test_stress_compositor_workspace_switch() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, workspace_manager, _window_manager, _input_manager) =
        make_test_compositor(config)?;

    let ids = add_n_windows(&mut compositor, 40);

    // Move windows to different columns
    for &id in ids.iter().take(20) {
        compositor.move_window_right(id);
    }

    // Tick after moves
    assert!(compositor.tick_for_test().is_ok(), "tick after window moves");

    // Verify workspace info
    {
        let ws = workspace_manager.read();
        // Column 0 has 20 windows, column 1 has 20 windows
        assert_eq!(ws.active_column_count(), 2, "two columns should exist");
    }

    // Switch workspace (scroll right)
    // We can't call scroll_right directly on the compositor, but we can
    // use the workspace manager through the Arc
    {
        let mut ws = workspace_manager.write();
        ws.scroll_right();
    }

    assert!(compositor.tick_for_test().is_ok(), "tick after workspace switch");

    // Switch back
    {
        let mut ws = workspace_manager.write();
        ws.scroll_left();
    }

    assert!(compositor.tick_for_test().is_ok(), "tick after workspace switch back");

    // Clean up
    for id in ids {
        compositor.remove_window(id);
    }

    Ok(())
}

/// Stress test: 100 windows through the compositor with tick.
#[test]
#[serial_test::serial]
fn test_stress_compositor_100_windows() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, _workspace_manager, window_manager, _input_manager) =
        make_test_compositor(config)?;

    let ids = add_n_windows(&mut compositor, 100);

    // Verify all windows tracked
    {
        let wm = window_manager.read();
        assert_eq!(wm.window_count(), 100);
    }

    // Tick with 100 windows
    assert!(compositor.tick_for_test().is_ok(), "tick with 100 windows");

    // Remove half
    for id in ids.iter().take(50) {
        compositor.remove_window(*id);
    }

    assert!(compositor.tick_for_test().is_ok(), "tick after removing 50 windows");

    {
        let wm = window_manager.read();
        assert_eq!(wm.window_count(), 50, "50 windows remaining");
    }

    // Remove the rest
    for &id in ids.iter().skip(50) {
        compositor.remove_window(id);
    }

    assert!(compositor.tick_for_test().is_ok(), "tick after removing all windows");

    {
        let wm = window_manager.read();
        assert_eq!(wm.window_count(), 0, "all windows removed");
    }

    Ok(())
}

/// Stress test: interleaved operations on many windows.
#[test]
#[serial_test::serial]
fn test_stress_interleaved_operations() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, _workspace_manager, window_manager, _input_manager) =
        make_test_compositor(config)?;

    // Phase 1: create 20 windows
    let mut ids = add_n_windows(&mut compositor, 20);

    // Phase 2: interleave minimize, fullscreen, move, remove, add
    for i in 0..10 {
        // Minimize and restore
        let _ = compositor.minimize_window(ids[i]);
        let _ = compositor.restore_window(ids[i]);

        // Move some windows right
        compositor.move_window_right(ids[i + 10]);

        // Add a new window
        let new_id = compositor.add_window(format!("Interleaved {}", i));
        ids.push(new_id);
    }

    // Phase 3: fullscreen toggle on all original windows
    for &id in ids.iter().take(20) {
        compositor.toggle_fullscreen(id);
    }

    // Tick
    assert!(compositor.tick_for_test().is_ok(), "tick after interleaved operations");

    // Phase 4: remove everything
    for id in ids {
        compositor.remove_window(id);
    }

    assert!(compositor.tick_for_test().is_ok(), "tick after final cleanup");

    {
        let wm = window_manager.read();
        assert_eq!(wm.window_count(), 0, "all windows removed after interleaved ops");
    }

    Ok(())
}