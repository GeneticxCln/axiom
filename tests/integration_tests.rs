//! Integration tests for Axiom compositor
//!
//! These tests verify end-to-end functionality including IPC communication,
//! compositor lifecycle, and interaction between major subsystems.

use anyhow::Result;
use parking_lot::RwLock;
use std::sync::Arc;

// Import Axiom modules
use axiom::{
    compositor::AxiomCompositor,
    config::AxiomConfig,
    input::InputManager,
    ipc::{AxiomIPCServer, AxiomMessage, LazyUIMessage},
    window::WindowManager,
    workspace::ScrollableWorkspaces,
};

/// Test IPC server startup and basic communication
#[tokio::test]
async fn test_ipc_server_startup() -> Result<()> {
    let ipc_server = AxiomIPCServer::new();

    // Test that socket path is correctly configured
    let socket_path = ipc_server.socket_path().to_path_buf();

    // Verify the socket path ends with axiom.sock
    assert!(socket_path.ends_with("axiom.sock") || socket_path.ends_with("axiom-lazy-ui.sock"));

    Ok(())
}

/// Test IPC message serialization/deserialization
#[tokio::test]
async fn test_ipc_message_protocol() -> Result<()> {
    // Test AxiomMessage serialization
    let perf_message = AxiomMessage::PerformanceMetrics {
        timestamp: 1234567890,
        cpu_usage: 25.5,
        memory_usage: 45.2,
        gpu_usage: 12.1,
        frame_time: 16.67,
        active_windows: 5,
        current_workspace: 2,
    };

    let json = serde_json::to_string(&perf_message)?;
    let _deserialized: AxiomMessage = serde_json::from_str(&json)?;

    // Test LazyUIMessage deserialization
    let optimize_json =
        r#"{"type":"OptimizeConfig","changes":{"blur_radius":5.0},"reason":"performance"}"#;
    let _message: LazyUIMessage = serde_json::from_str(optimize_json)?;

    Ok(())
}

/// Test compositor initialization with all subsystems
#[tokio::test]
#[serial_test::serial]
async fn test_compositor_initialization() -> Result<()> {
    let config = AxiomConfig::default();
    let (compositor, ..) = make_test_compositor(config).await?;

    // Verify basic state
    assert!(!compositor.is_windowed());
    let cfg = compositor.config();
    assert!(cfg.effects.enabled);

    // Verify workspace info is accessible
    let (column, _pos, _count, _scrolling) = compositor.get_workspace_info();
    assert!(column >= 0);

    Ok(())
}

/// Test configuration loading and validation
#[tokio::test]
async fn test_configuration_system() -> Result<()> {
    // Test default configuration
    let default_config = AxiomConfig::default();
    assert!(default_config.effects.enabled);
    assert!(default_config.workspace.workspace_width > 0);

    // Test configuration serialization
    let toml_str = toml::to_string(&default_config)?;
    let _parsed_config: AxiomConfig = toml::from_str(&toml_str)?;

    Ok(())
}

/// Test workspace management without compositor
#[tokio::test]
async fn test_workspace_logic() -> Result<()> {
    use axiom::config::WorkspaceConfig;
    use axiom::workspace::ScrollableWorkspaces;

    let config = WorkspaceConfig::default();
    let mut workspaces = ScrollableWorkspaces::new(&config);

    // Test adding windows (they go into the focused column)
    workspaces.add_window(1001);
    workspaces.add_window(1002);
    workspaces.add_window(1003);

    // All windows go into the same focused column
    assert_eq!(workspaces.active_column_count(), 1);

    // Test scrolling
    workspaces.scroll_right();
    assert_eq!(workspaces.focused_column_index(), 1);

    workspaces.scroll_left();
    assert_eq!(workspaces.focused_column_index(), 0);

    // Test window movement
    let moved = workspaces.move_window_right(1001);
    assert!(moved);

    Ok(())
}

/// Test input event processing
#[tokio::test]
async fn test_input_processing() -> Result<()> {
    use axiom::config::{BindingsConfig, InputConfig};
    use axiom::input::{CompositorAction, InputEvent, InputManager};

    let input_config = InputConfig::default();
    let bindings_config = BindingsConfig::default();

    let mut input_manager = InputManager::new(&input_config, &bindings_config);

    // Test scroll event processing
    let scroll_event = InputEvent::Scroll {
        x: 100.0,
        y: 100.0,
        delta_x: 50.0,
        delta_y: 0.0,
    };

    let actions = input_manager.process_input_event(scroll_event);

    // Should generate workspace scroll actions
    assert!(!actions.is_empty());

    // Verify we get expected action types
    for action in &actions {
        match action {
            CompositorAction::ScrollWorkspaceLeft | CompositorAction::ScrollWorkspaceRight => {
                // Expected actions
            }
            _ => {
                // Other actions might be present too
            }
        }
    }

    input_manager.shutdown();
    Ok(())
}

/// Test concurrent operations
#[tokio::test]
async fn test_concurrent_operations() -> Result<()> {
    use axiom::config::WorkspaceConfig;
    use axiom::workspace::ScrollableWorkspaces;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let config = WorkspaceConfig::default();
    let workspaces = Arc::new(Mutex::new(ScrollableWorkspaces::new(&config)));

    // Spawn multiple tasks that modify workspaces concurrently
    let mut handles = vec![];

    for task_id in 0..5 {
        let workspaces_clone = Arc::clone(&workspaces);
        let handle = tokio::spawn(async move {
            let mut ws = workspaces_clone.lock().await;

            // Each task adds some windows
            for i in 0..10 {
                let window_id = (task_id * 10 + i) as u64;
                ws.add_window(window_id);
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await?;
    }

    // All 50 windows end up in 1 column (each task adds to focused column)
    let ws = workspaces.lock().await;
    assert_eq!(ws.active_column_count(), 1);

    Ok(())
}

// ============================================================================
// Window Lifecycle & Layout Tests
// ============================================================================

/// Test window lifecycle: create, track, remove
#[tokio::test]
async fn test_window_lifecycle() -> Result<()> {
    use axiom::config::WindowConfig;
    use axiom::window::WindowManager;

    let config = WindowConfig::default();
    let mut wm = WindowManager::new(&config);

    // Create windows
    let w1 = wm.add_window("Window 1".into());
    let w2 = wm.add_window("Window 2".into());
    let w3 = wm.add_window("Window 3".into());

    // Verify they exist and have sequential IDs
    assert!(wm.get_window(w1).is_some());
    assert!(wm.get_window(w2).is_some());
    assert!(wm.get_window(w3).is_some());
    assert_eq!(w1, 1);
    assert_eq!(w2, 2);
    assert_eq!(w3, 3);

    // Focus window 2
    wm.focus_window(w2);
    assert_eq!(wm.focused_window_id(), Some(w2));

    // Remove window 2
    wm.remove_window(w2);
    assert!(wm.get_window(w2).is_none());
    // Window 1 and 3 still exist
    assert!(wm.get_window(w1).is_some());
    assert!(wm.get_window(w3).is_some());

    // Focus should change after removing focused window
    wm.focus_window(w3);
    assert_eq!(wm.focused_window_id(), Some(w3));

    // Clean up
    wm.remove_window(w1);
    wm.remove_window(w3);
    wm.shutdown();

    Ok(())
}

/// Test window lifecycle with workspace integration
#[tokio::test]
async fn test_window_layout_with_workspaces() -> Result<()> {
    use axiom::config::{WindowConfig, WorkspaceConfig};
    use axiom::window::WindowManager;
    use axiom::workspace::ScrollableWorkspaces;

    let window_config = WindowConfig::default();
    let workspace_config = WorkspaceConfig::default();

    let mut wm = WindowManager::new(&window_config);
    let mut workspaces = ScrollableWorkspaces::new(&workspace_config);

    // Create windows and add to workspace
    let ids: Vec<u64> = (0..5)
        .map(|i| {
            let id = wm.add_window(format!("Win {}", i));
            workspaces.add_window(id);
            id
        })
        .collect();

    // All in focused column
    let column_windows = workspaces.get_focused_column_windows();
    assert_eq!(column_windows.len(), 5);

    // Layouts should be calculable
    let layouts = workspaces.calculate_workspace_layouts();
    assert_eq!(layouts.len(), 5);

    // Each layout should have positive dimensions
    for rect in layouts.values() {
        assert!(rect.width > 0, "window width should be positive");
        assert!(rect.height > 0, "window height should be positive");
    }

    // Move a window to a new column
    assert!(workspaces.move_window_right(ids[0]));

    // Verify window count after move
    let remaining = workspaces.get_focused_column_windows();
    assert_eq!(remaining.len(), 4, "one window moved to the right");

    // Remove all windows
    for id in ids {
        workspaces.remove_window(id);
        wm.remove_window(id);
    }

    wm.shutdown();
    workspaces.shutdown();

    Ok(())
}

// ============================================================================
// Compositor Initialization Test
// ============================================================================

/// Helper: construct a fully-initialized test compositor with all subsystems.
/// Returns the compositor and the subsystem Arcs for tests that need direct access.
///
/// `Arc<parking_lot::RwLock<ScrollableWorkspaces>>` is intentionally
/// `!Sync` (the layout cache uses `RefCell` for the single-threaded hot
/// path). These `Arc`s never cross thread boundaries — every caller is
/// a `tokio::test` task — so the lint is harmless for tests but we
/// allow it explicitly to avoid future contributors being puzzled by
/// the warning.
#[allow(clippy::type_complexity, clippy::arc_with_non_send_sync)]
async fn make_test_compositor(
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

    // Use the headless Noop backend so tests don't create a winit event loop
    // (winit allows only one per process, which breaks parallel test binaries).
    let mut config = config;
    config.backend.kind = "noop".to_string();

    let compositor = AxiomCompositor::new(
        config,
        false, // windowed
        workspace_manager.clone(),
        window_manager.clone(),
        input_manager.clone(),
        ipc_server,
    )
    .await?;

    Ok((
        compositor,
        workspace_manager,
        window_manager,
        input_manager,
    ))
}

/// Test compositor initialization with all subsystems (replaces old ignored test)
#[tokio::test]
#[serial_test::serial]
async fn test_compositor_full_initialization() -> Result<()> {
    let config = AxiomConfig::default();
    let (compositor, ..) = make_test_compositor(config).await?;

    // Verify basic state
    assert!(!compositor.is_windowed());
    let cfg = compositor.config();
    assert!(cfg.effects.enabled);

    // Verify workspace info is accessible
    let (column, _pos, _count, _scrolling) = compositor.get_workspace_info();
    assert!(column >= 0);

    Ok(())
}

// ============================================================================
// Decoration Manager Integration Tests
// ============================================================================

/// Integration test: `DecorationManager::add_window` wired end-to-end
/// with real window geometry from `WindowManager`. Verifies that the
/// `DEFAULT_WINDOW_WIDTH` placeholder-free path works correctly:
/// button positions are derived from the live `BackendWindow` width,
/// title matches, and `set_window_width` updates positions on resize.
#[tokio::test]
async fn test_decoration_manager_with_real_window_geometry() -> Result<()> {
    use axiom::config::WindowConfig;
    use axiom::decoration::{DecorationAction, DecorationManager, DecorationMode};
    use axiom::window::WindowManager;

    let window_config = WindowConfig::default();
    let mut wm = WindowManager::new(&window_config);

    // ── Create a real window via WindowManager ─────────────────────
    let window_id = wm.add_window("Integration Test Window".into());
    assert_eq!(window_id, 1, "first window ID is 1");

    let axiom_window = wm.get_window(window_id).expect("window exists after add");
    let real_width = axiom_window.window.size.0 as i32; // 800 by default
    assert_eq!(real_width, 800, "default BackendWindow width is 800");
    let real_title = axiom_window.window.title.clone();
    assert_eq!(real_title, "Integration Test Window");

    // ── Feed real geometry into DecorationManager (no placeholder) ─
    let mut deco = DecorationManager::new(&window_config, /* minimize_enabled */ false);
    deco.add_window(
        window_id,
        real_title.clone(),
        /* prefers_server_side */ true,
        real_width,
    );

    // ── Verify the decoration was created with the real width ──────
    let decoration = deco
        .get_decoration(window_id)
        .expect("decoration should exist after add_window");
    assert_eq!(
        decoration.mode,
        DecorationMode::ServerSide,
        "default mode should be ServerSide"
    );
    assert_eq!(decoration.title, "Integration Test Window");
    assert!(!decoration.focused, "new window is not focused by default");
    assert_eq!(decoration.titlebar_height, 32, "default titlebar height");

    // ── Button positions must be derived from the real 800px width ──
    // Geometry: button_size=24, margin=8, titlebar_height=32, button_y=4.
    // close idx=0: x = 800 - (24+8)*1 = 768
    // maximize idx=1: x = 800 - (24+8)*2 = 736
    // minimize idx=2: x = 800 - (24+8)*3 = 704 → zeroed when disabled
    let close_bounds = &decoration.buttons.close.bounds;
    assert_eq!(close_bounds.x, 768, "close button x at ww=800");
    assert_eq!(close_bounds.y, 4, "close button y");
    assert_eq!(close_bounds.width, 24);
    assert_eq!(close_bounds.height, 24);

    let max_bounds = &decoration.buttons.maximize.bounds;
    assert_eq!(max_bounds.x, 736, "maximize button x at ww=800");
    assert_eq!(max_bounds.y, 4);

    // Minimize disabled → bounds zeroed
    let min_bounds = &decoration.buttons.minimize.bounds;
    assert_eq!(min_bounds.width, 0, "minimize width zeroed when disabled");
    assert_eq!(min_bounds.height, 0, "minimize height zeroed when disabled");

    // ── Titlebar click detection: buttons must NOT overlap at ww=800 ──
    // Click on close button at (770, 12) → inside close bounds [768, 792)
    let action = deco.handle_button_press(window_id, 770, 12);
    assert_eq!(
        action,
        Some(DecorationAction::Close),
        "click on close button should return Close"
    );
    deco.handle_button_release(window_id, 770, 12);

    // Click on maximize button at (740, 10) → inside maximize [736, 760)
    let action = deco.handle_button_press(window_id, 740, 10);
    assert_eq!(
        action,
        Some(DecorationAction::ToggleMaximize),
        "click on maximize button should return ToggleMaximize"
    );
    deco.handle_button_release(window_id, 740, 10);

    // Click on titlebar (away from buttons) → StartMove
    let action = deco.handle_button_press(window_id, 100, 10);
    assert_eq!(
        action,
        Some(DecorationAction::StartMove),
        "click on titlebar away from buttons should return StartMove"
    );
    deco.handle_button_release(window_id, 100, 10);

    // Click below the titlebar (y=50) → no action
    let action = deco.handle_button_press(window_id, 100, 50);
    assert!(action.is_none(), "click below titlebar should return None");

    // ── Resize window and verify button positions update ───────────
    // Simulate the compositor resizing the window to 1200px wide
    deco.set_window_width(window_id, 1200);
    let decoration = deco.get_decoration(window_id).unwrap();
    // New positions for ww=1200:
    // close idx=0: x = 1200 - 32 = 1168
    // maximize idx=1: x = 1200 - 64 = 1136
    assert_eq!(
        decoration.buttons.close.bounds.x, 1168,
        "close button x updated after resize to 1200"
    );
    assert_eq!(
        decoration.buttons.maximize.bounds.x, 1136,
        "maximize button x updated after resize to 1200"
    );
    // Verify buttons still work at the new positions
    let action = deco.handle_button_press(window_id, 1170, 10);
    assert_eq!(
        action,
        Some(DecorationAction::Close),
        "close button still clickable after resize"
    );
    deco.handle_button_release(window_id, 1170, 10);

    // ── set_window_width is a no-op for the same value ─────────────
    // Verify it doesn't panic or corrupt state
    deco.set_window_width(window_id, 1200);
    let decoration = deco.get_decoration(window_id).unwrap();
    assert_eq!(
        decoration.buttons.close.bounds.x, 1168,
        "no-op set_window_width unchanged"
    );

    // ── Focus change ──────────────────────────────────────────────
    deco.set_window_focus(window_id, true);
    assert!(deco.get_decoration(window_id).unwrap().focused);
    deco.set_window_focus(window_id, false);
    assert!(!deco.get_decoration(window_id).unwrap().focused);

    // ── Title update ──────────────────────────────────────────────
    deco.set_window_title(window_id, "Renamed Window".into());
    assert_eq!(
        deco.get_decoration(window_id).unwrap().title,
        "Renamed Window"
    );

    // ── Remove window from decoration manager ─────────────────────
    deco.remove_window(window_id);
    assert!(
        deco.get_decoration(window_id).is_none(),
        "decoration removed after window removal"
    );

    // Clean up WindowManager
    wm.remove_window(window_id);
    wm.shutdown();

    Ok(())
}

// ============================================================================
// IPC Dispatch Integration Tests
// ============================================================================

/// Test that IPC server can process WorkspaceCommand messages end-to-end
#[tokio::test]
async fn test_ipc_workspace_command_flow() -> Result<()> {
    use axiom::config::AxiomConfig;
    use axiom::ipc::LazyUIMessage;

    let mut config = AxiomConfig::default();
    let mut ipc_server = AxiomIPCServer::new();

    // Start the server (creates broadcast and command channels)
    ipc_server.start()?;

    // Simulate sending a WorkspaceCommand through the command channel
    // (same path the per-client handler uses)
    let cmd = LazyUIMessage::WorkspaceCommand {
        action: "add_window".into(),
        parameters: serde_json::json!({"title": "IPC Window"}),
    };

    if let Some(sender) = ipc_server.command_sender_for_test() {
        sender.send(cmd).await.unwrap();
    }

    // Process the message
    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    // add_window is a WorkspaceCommand -> forwarded to pending_actions
    // (compositor dispatches it); no config changes
    assert!(!changed, "WorkspaceCommand should not change config");
    assert_eq!(actions.len(), 1, "one pending action");
    match &actions[0] {
        LazyUIMessage::WorkspaceCommand { action, .. } => {
            assert_eq!(action, "add_window");
        }
        _ => panic!("Expected WorkspaceCommand"),
    }

    ipc_server.shutdown().await?;

    Ok(())
}

/// Test IPC OptimizeConfig message correctly mutates config
#[tokio::test]
async fn test_ipc_optimize_config_flow() -> Result<()> {
    use axiom::config::AxiomConfig;
    use axiom::ipc::LazyUIMessage;
    use std::collections::HashMap;

    let mut config = AxiomConfig::default();
    let mut ipc_server = AxiomIPCServer::new();

    ipc_server.start()?;

    let original_scroll = config.workspace.scroll_speed;

    let mut changes = HashMap::new();
    changes.insert("workspace.scroll_speed".into(), serde_json::json!(2.5));
    let cmd = LazyUIMessage::OptimizeConfig {
        changes,
        reason: "test".into(),
    };

    if let Some(sender) = ipc_server.command_sender_for_test() {
        sender.send(cmd).await.unwrap();
    }

    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    assert!(changed, "OptimizeConfig should change config");
    assert!(actions.is_empty(), "no pending actions for config changes");
    assert!(
        (config.workspace.scroll_speed - 2.5).abs() < f64::EPSILON,
        "scroll speed should be updated"
    );
    assert_ne!(config.workspace.scroll_speed, original_scroll);

    ipc_server.shutdown().await?;

    Ok(())
}

/// Test that SetClipboard IPC message is correctly forwarded
#[tokio::test]
async fn test_ipc_set_clipboard_flow() -> Result<()> {
    use axiom::config::AxiomConfig;
    use axiom::ipc::LazyUIMessage;

    let mut config = AxiomConfig::default();
    let mut ipc_server = AxiomIPCServer::new();

    // Start the server (creates broadcast and command channels)
    ipc_server.start()?;

    // Simulate sending a SetClipboard command through the command channel
    let cmd = LazyUIMessage::SetClipboard {
        text: "Hello from IPC test".into(),
    };

    if let Some(sender) = ipc_server.command_sender_for_test() {
        sender.send(cmd).await.unwrap();
    }

    // Process the message
    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    // SetClipboard is a command-type message — forwarded to pending_actions
    assert!(!changed, "SetClipboard should not change config");
    assert_eq!(actions.len(), 1, "one pending action");
    match &actions[0] {
        LazyUIMessage::SetClipboard { text } => {
            assert_eq!(text, "Hello from IPC test");
        }
        _ => panic!("Expected SetClipboard"),
    }

    ipc_server.shutdown().await?;

    Ok(())
}

/// Test that SetClipboard IPC command is dispatched through the compositor
#[tokio::test]
#[serial_test::serial]
async fn test_compositor_set_clipboard_dispatch() -> Result<()> {
    use axiom::config::AxiomConfig;
    use axiom::ipc::LazyUIMessage;

    let config = AxiomConfig::default();
    let (mut compositor, _ws, _wm, _im) = make_test_compositor(config).await?;

    // Get the IPC server's command sender from the compositor
    let sender = compositor.ipc_command_sender();

    // Send SetClipboard command
    let cmd = LazyUIMessage::SetClipboard {
        text: "compositor test clipboard".into(),
    };
    sender.send(cmd).await.unwrap();

    // Run a tick — this should process the IPC message
    // and call set_clipboard_data on the backend.
    let result = compositor.tick_for_test().await;
    assert!(result.is_ok(), "tick should succeed");

    // Verify the clipboard cache was populated
    let cached = compositor.debug_clipboard_cache();
    assert!(cached.is_some(), "clipboard cache should be populated");
    assert_eq!(
        cached.as_deref().unwrap(),
        b"compositor test clipboard",
        "clipboard data should match set text"
    );

    Ok(())
}

// ============================================================================
// Compositor Event Loop Integration Tests
// ============================================================================

/// Test that tick() with 5+ consecutive errors triggers emergency shutdown.
/// Uses `force_next_tick_error` to simulate real errors in the event loop,
/// verifying that the count accumulates and resets correctly.
#[tokio::test]
#[serial_test::serial]
async fn test_tick_error_recovery() -> Result<()> {
    // `consecutive_error_count` now DECREMENTS with each clean tick
    // instead of snapping to zero. `N` consecutive errors need at
    // least `N` clean ticks before the counter fully resets
    // (audit Bug 1 fix).
    let (mut compositor, ..) = make_test_compositor(AxiomConfig::default()).await?;

    // 1) Clean tick on a fresh compositor: count stays 0, running.
    assert!(
        compositor.tick_for_test().await.is_ok(),
        "clean tick should return Ok",
    );
    assert!(
        compositor.is_running(),
        "should be running after clean tick",
    );

    // 2) Three error ticks push count to 3 (still running, below
    //    threshold of 5).
    for _ in 0..3 {
        compositor.force_next_tick_error();
        assert!(
            compositor.tick_for_test().await.is_ok(),
            "count 1..=3 should not yet trigger shutdown",
        );
    }
    assert!(compositor.is_running(), "3 errors: still running");

    // 3) One clean tick DECREMENTS the counter (`3 - 1 = 2`), it does
    //    NOT snap to zero. This is the audit Bug 1 fix: a single
    //    clean tick must not mask prior consecutive failures.
    assert!(
        compositor.tick_for_test().await.is_ok(),
        "clean tick should succeed after 3 errors",
    );
    // No public getter for `consecutive_error_count`, so we drive it
    // through `set_errors_for_test` and `force_next_tick_error`. The
    // recovery is observable as: shutdown happens at the CUMULATIVE
    // 5th error, not 3 more errors.
    assert!(
        compositor.is_running(),
        "1 clean tick: still running (count=2)"
    );

    // 4) Two more error ticks push count from 2 to 4 (still running).
    for _ in 0..2 {
        compositor.force_next_tick_error();
        assert!(
            compositor.tick_for_test().await.is_ok(),
            "count <5 should be ok",
        );
        assert!(compositor.is_running(), "still running at count <5");
    }

    // 5) The 3rd additional error pushes count from 4 to 5, hitting
    //    the threshold and triggering emergency shutdown. With
    //    snap-to-zero (the old buggy behaviour), the clean tick
    //    would have reset the counter, so this same sequence of
    //    3 errors AFTER recovery would not have crossed the
    //    threshold. The test now enforces the corrected semantics.
    compositor.force_next_tick_error();
    let result = compositor.tick_for_test().await;
    assert!(
        result.is_err(),
        "cumulative (post-recovery) 3rd error must push count to 5 and shut down",
    );
    assert!(
        !compositor.is_running(),
        "compositor must stop after cumulative threshold",
    );

    Ok(())
}

/// Test frame pacing: tick() should complete quickly with unlimited FPS (max_fps=0)
#[tokio::test]
#[serial_test::serial]
async fn test_frame_pacing() -> Result<()> {
    use std::time::Instant;

    let mut config = AxiomConfig::default();
    config.general.max_fps = 0; // unlimited
    let (mut compositor, ..) = make_test_compositor(config).await?;

    let start = Instant::now();
    compositor.tick_for_test().await?;
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "unlimited FPS tick should complete quickly, took {:?}",
        elapsed
    );

    Ok(())
}

/// Test that a tick with a 60 FPS limit completes within a reasonable time.
#[tokio::test]
#[serial_test::serial]
async fn test_frame_pacing_with_fps_limit() -> Result<()> {
    use std::time::Instant;

    let mut config = AxiomConfig::default();
    config.general.max_fps = 60;
    let (mut compositor, ..) = make_test_compositor(config).await?;

    let start = Instant::now();
    compositor.tick_for_test().await?;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "60 FPS tick should complete within 100ms, took {:?}",
        elapsed
    );

    Ok(())
}

/// Test that viewport resize doesn't panic and layouts remain valid.
#[tokio::test]
#[serial_test::serial]
async fn test_viewport_resize_propagates_to_layouts() -> Result<()> {
    let (mut compositor, workspace_manager, ..) =
        make_test_compositor(AxiomConfig::default()).await?;

    // Add a single window so layout produces one entry
    compositor.add_window("Resize Test".into());

    // Resize to 4K — window height should reflect the tall viewport
    compositor.set_viewport_size(3840, 2160);
    let wm = workspace_manager.read();
    let layouts_4k = wm.calculate_workspace_layouts();
    assert_eq!(layouts_4k.len(), 1, "one window → one layout");
    let height_4k = layouts_4k.values().next().unwrap().height;
    // 1 window, gap=10: height = viewport_height - 2*gap = 2160 - 20 = 2140
    assert!(
        height_4k > 2000,
        "4K window height should be >2000, got {}",
        height_4k
    );
    drop(wm);

    // Resize to a small viewport — window height should shrink proportionally
    compositor.set_viewport_size(800, 600);
    let wm = workspace_manager.read();
    let layouts_small = wm.calculate_workspace_layouts();
    let height_small = layouts_small.values().next().unwrap().height;
    // 1 window, gap=10: height = 600 - 20 = 580
    assert!(
        height_small < 600,
        "small viewport height should be <600, got {}",
        height_small
    );
    assert!(
        height_small < height_4k,
        "window height should shrink with viewport: {} → {}",
        height_4k,
        height_small
    );

    Ok(())
}

/// Test IPC HealthCheck and GetPerformanceReport don't mutate config
#[tokio::test]
#[serial_test::serial]
async fn test_ipc_readonly_messages() -> Result<()> {
    use axiom::config::AxiomConfig;
    use axiom::ipc::LazyUIMessage;

    let mut config = AxiomConfig::default();
    let config_clone = config.clone();

    let mut ipc_server = AxiomIPCServer::new();
    ipc_server.start()?;

    // HealthCheck
    if let Some(sender) = ipc_server.command_sender_for_test() {
        sender.send(LazyUIMessage::HealthCheck).await.unwrap();
        sender
            .send(LazyUIMessage::GetPerformanceReport)
            .await
            .unwrap();
    }

    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    assert!(!changed, "HealthCheck should not change config");
    assert!(actions.is_empty(), "HealthCheck produces no actions");

    // Config should be unchanged
    assert_eq!(config.workspace.scroll_speed, config_clone.workspace.scroll_speed);

    ipc_server.shutdown().await?;

    Ok(())
}
