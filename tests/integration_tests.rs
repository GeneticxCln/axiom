//! Integration tests for Axiom compositor
//!
//! These tests verify end-to-end functionality including IPC communication,
//! compositor lifecycle, and interaction between major subsystems.

use anyhow::Result;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

// Import Axiom modules
use axiom::{
    compositor::AxiomCompositor,
    config::AxiomConfig,
    effects::EffectsEngine,
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

/// Test effects engine initialization and basic operations
#[tokio::test]
async fn test_effects_engine() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    let config = EffectsConfig::default();
    let mut effects = EffectsEngine::new(&config)?;

    // Test animation creation
    effects.animate_window_move(1001, (100.0, 100.0), (200.0, 200.0));

    // Test update cycle
    effects.update()?;

    // Test performance stats
    let (_frame_time, quality, _active_count) = effects.get_performance_stats();
    assert!((0.0..=1.0).contains(&quality));

    // Test shutdown
    effects.shutdown();

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

/// Stress test: Create many windows and test performance
#[tokio::test]
async fn test_stress_many_windows() -> Result<()> {
    use axiom::config::{EffectsConfig, WorkspaceConfig};
    use axiom::effects::EffectsEngine;
    use axiom::workspace::ScrollableWorkspaces;

    let workspace_config = WorkspaceConfig::default();
    let effects_config = EffectsConfig::default();

    let mut workspaces = ScrollableWorkspaces::new(&workspace_config);
    let mut effects = EffectsEngine::new(&effects_config)?;

    // Add 100 windows (they all go into the focused column)
    for i in 1..=100 {
        workspaces.add_window(i);
        // Add some effects to test performance
        if i % 10 == 0 {
            effects.animate_window_move(i, (0.0, 0.0), (100.0, 100.0));
        }
    }

    // All windows are in one column (the focused column)
    assert_eq!(workspaces.active_column_count(), 1);

    // Test scrolling through many workspaces
    let start = std::time::Instant::now();
    for _ in 0..50 {
        workspaces.scroll_right();
        effects.update()?;
    }
    let elapsed = start.elapsed();

    // Should complete within reasonable time (1 second for 50 operations)
    assert!(elapsed < Duration::from_secs(1));

    effects.shutdown();
    workspaces.shutdown();

    Ok(())
}

/// Test error handling and recovery scenarios
#[tokio::test]
async fn test_error_recovery() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    // Test effects engine with invalid configuration
    let mut bad_config = EffectsConfig::default();
    bad_config.blur.intensity = -1.0; // Invalid value

    // Should handle gracefully or provide meaningful error
    let result = EffectsEngine::new(&bad_config);
    match result {
        Ok(mut engine) => {
            // If it accepts invalid config, it should sanitize it
            let (_, quality, _) = engine.get_performance_stats();
            assert!((0.0..=1.0).contains(&quality));
            engine.shutdown();
        }
        Err(_) => {
            // Expected to reject invalid configuration
        }
    }

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
#[allow(clippy::type_complexity)]
async fn make_test_compositor(
    config: AxiomConfig,
) -> Result<(
    AxiomCompositor,
    Arc<RwLock<ScrollableWorkspaces>>,
    Arc<RwLock<WindowManager>>,
    Arc<RwLock<EffectsEngine>>,
    Arc<RwLock<InputManager>>,
)> {
    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let effects_engine = Arc::new(RwLock::new(EffectsEngine::new(&config.effects)?));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));

    let compositor = AxiomCompositor::new_for_test(
        config,
        workspace_manager.clone(),
        effects_engine.clone(),
        window_manager.clone(),
        input_manager.clone(),
    )
    .await?;

    Ok((
        compositor,
        workspace_manager,
        window_manager,
        effects_engine,
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

    let axiom_window = wm
        .get_window(window_id)
        .expect("window exists after add");
    let real_width = axiom_window.window.size.0 as i32; // 800 by default
    assert_eq!(real_width, 800, "default BackendWindow width is 800");
    let real_title = axiom_window.window.title.clone();
    assert_eq!(real_title, "Integration Test Window");

    // ── Feed real geometry into DecorationManager (no placeholder) ─
    let mut deco =
        DecorationManager::new(&window_config, /* minimize_enabled */ false);
    deco.add_window(window_id, real_title.clone(), /* prefers_server_side */ true, real_width);

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
    assert!(
        action.is_none(),
        "click below titlebar should return None"
    );

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
    assert_eq!(decoration.buttons.close.bounds.x, 1168, "no-op set_window_width unchanged");

    // ── Focus change ──────────────────────────────────────────────
    deco.set_window_focus(window_id, true);
    assert!(deco.get_decoration(window_id).unwrap().focused);
    deco.set_window_focus(window_id, false);
    assert!(!deco.get_decoration(window_id).unwrap().focused);

    // ── Title update ──────────────────────────────────────────────
    deco.set_window_title(window_id, "Renamed Window".into());
    assert_eq!(deco.get_decoration(window_id).unwrap().title, "Renamed Window");

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
// Renderer Integration Tests
// ============================================================================

/// Helper: build a small 64×64 RGBA test texture with a diagonal gradient.
fn make_test_texture_rgba() -> Vec<u8> {
    let w = 64usize;
    let h = 64usize;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 4) as u8;
            let g = (y * 4) as u8;
            let b = 128u8;
            let a = 255u8;
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(a);
        }
    }
    pixels
}

/// Integration test: `prepare_window_resources` creates uniform and vertex
/// buffers for windows that have a real texture. After feeding a real RGBA
/// texture via `update_window_texture`, calling `prepare_window_resources`
/// must populate both `cached_uniform_buffer` and `cached_vertex_buffer`.
#[tokio::test]
#[serial_test::serial]
async fn test_prepare_window_resources_creates_buffers() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;

    // Add a window and give it a real texture
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // Before prepare, cached buffers should be None (default)
    {
        let w = &renderer.windows[0];
        assert!(w.texture_view.is_some(), "texture should be uploaded");
        assert!(w.cached_uniform_buffer.is_none(), "uniform buffer starts None");
        assert!(w.cached_vertex_buffer.is_none(), "vertex buffer starts None");
    }

    // After prepare, both buffers should be populated
    renderer.prepare_window_resources();

    {
        let w = &renderer.windows[0];
        assert!(
            w.cached_uniform_buffer.is_some(),
            "uniform buffer should exist after prepare"
        );
        assert!(
            w.cached_vertex_buffer.is_some(),
            "vertex buffer should exist after prepare"
        );
        // Cached state should now match live state
        assert!(
            (w.cached_opacity - w.opacity).abs() < f32::EPSILON,
            "cached_opacity should sync to live opacity"
        );
        assert_eq!(w.cached_uniform_size, w.size, "cached_uniform_size should sync");
        assert_eq!(w.cached_position, w.position, "cached_position should sync");
        assert_eq!(w.cached_size, w.size, "cached_size should sync");
    }

    Ok(())
}

/// Integration test: changing a window's opacity via `upsert_window_rect`
/// must cause `prepare_window_resources` to recreate the uniform buffer
/// with the new opacity value baked in.
#[tokio::test]
#[serial_test::serial]
async fn test_prepare_window_resources_opacity_recreation() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // First prepare — seeds the cached buffers
    renderer.prepare_window_resources();
    assert_eq!(renderer.windows[0].cached_opacity, 1.0, "initial opacity");

    // Mutate opacity — should leave cache stale
    renderer.upsert_window_rect(1, (100.0, 200.0), (400.0, 300.0), 0.5);
    assert!((renderer.windows[0].opacity - 0.5).abs() < f32::EPSILON);
    assert!(
        (renderer.windows[0].cached_opacity - 1.0).abs() < f32::EPSILON,
        "cached_opacity should be stale (1.0, not 0.5)"
    );

    // Second prepare — should recreate uniform buffer (opacity changed),
    // but vertex buffer should survive (position + size unchanged)
    renderer.prepare_window_resources();

    {
        let w = &renderer.windows[0];
        assert!(w.cached_uniform_buffer.is_some(), "uniform buffer recreated");
        assert!(
            (w.cached_opacity - 0.5).abs() < f32::EPSILON,
            "cached_opacity should sync to 0.5"
        );
        // Vertex buffer: position and size unchanged → still present
        assert!(
            w.cached_vertex_buffer.is_some(),
            "vertex buffer should survive opacity change"
        );
        assert_eq!(w.cached_uniform_size, (400.0, 300.0));
        assert_eq!(w.cached_position, (100.0, 200.0));
        assert_eq!(w.cached_size, (400.0, 300.0));
    }

    Ok(())
}

/// Integration test: changing a window's size must invalidate both the
/// uniform buffer (size baked into `WindowUniforms`) and the vertex
/// buffer (vertex positions change).
#[tokio::test]
#[serial_test::serial]
async fn test_prepare_window_resources_size_recreation() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // First prepare — seeds the cached buffers
    renderer.prepare_window_resources();

    // Change size — position stays the same
    renderer.upsert_window_rect(1, (100.0, 200.0), (600.0, 450.0), 1.0);

    // Caches should be stale
    assert_eq!(renderer.windows[0].size, (600.0, 450.0));
    assert_eq!(renderer.windows[0].cached_uniform_size, (400.0, 300.0),
        "cached_uniform_size stale");
    assert_eq!(renderer.windows[0].cached_size, (400.0, 300.0),
        "cached_size stale");

    // Second prepare — both buffers regenerated (size flows into both)
    renderer.prepare_window_resources();

    {
        let w = &renderer.windows[0];
        assert!(w.cached_uniform_buffer.is_some(), "uniform buffer recreated after size change");
        assert!(w.cached_vertex_buffer.is_some(), "vertex buffer recreated after size change");
        assert_eq!(w.cached_uniform_size, (600.0, 450.0));
        assert_eq!(w.cached_size, (600.0, 450.0));
    }

    Ok(())
}

/// Integration test: changing a window's position must invalidate the
/// vertex buffer (vertex coordinates shift) but NOT the uniform buffer
/// (position isn't part of the uniform data).
#[tokio::test]
#[serial_test::serial]
async fn test_prepare_window_resources_position_recreation() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // First prepare
    renderer.prepare_window_resources();

    // Move the window — position changes, size stays the same
    renderer.upsert_window_rect(1, (300.0, 500.0), (400.0, 300.0), 1.0);

    assert_eq!(renderer.windows[0].position, (300.0, 500.0));
    assert_eq!(renderer.windows[0].cached_position, (100.0, 200.0),
        "cached_position stale");

    // Second prepare — vertex buffer regenerated (coords shift),
    // uniform buffer survives (opacity + size unchanged)
    renderer.prepare_window_resources();

    {
        let w = &renderer.windows[0];
        assert!(w.cached_uniform_buffer.is_some(), "uniform buffer survives position change");
        assert!(w.cached_vertex_buffer.is_some(), "vertex buffer recreated after position change");
        assert_eq!(w.cached_position, (300.0, 500.0));
        assert_eq!(w.cached_uniform_size, (400.0, 300.0));
        assert_eq!(w.cached_size, (400.0, 300.0));
    }

    Ok(())
}

/// Integration test: idempotent `prepare_window_resources` — calling it
/// twice with no state changes in between must leave cached buffers
/// intact (no unnecessary GPU allocation churn).
#[tokio::test]
#[serial_test::serial]
async fn test_prepare_window_resources_idempotent() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // First prepare
    renderer.prepare_window_resources();

    // Second prepare with no mutations → buffers intact (idempotent)
    renderer.prepare_window_resources();

    {
        let w = &renderer.windows[0];
        assert!(
            w.cached_uniform_buffer.is_some(),
            "uniform buffer still present after idempotent prepare"
        );
        assert!(
            w.cached_vertex_buffer.is_some(),
            "vertex buffer still present after idempotent prepare"
        );
        assert_eq!(w.cached_opacity, 1.0);
        assert_eq!(w.cached_uniform_size, (400.0, 300.0));
        assert_eq!(w.cached_position, (100.0, 200.0));
        assert_eq!(w.cached_size, (400.0, 300.0));
    }

    Ok(())
}

/// Integration test: `prepare_window_resources` on a window without a
/// texture (no `texture_view`) must skip buffer creation entirely.
/// A plain `upsert_window_rect` adds a window entry without a texture.
#[tokio::test]
#[serial_test::serial]
async fn test_prepare_window_resources_skips_textureless_window() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    // upsert_window_rect does NOT set a texture — only geometry
    renderer.upsert_window_rect(1, (100.0, 200.0), (400.0, 300.0), 1.0);

    assert!(
        renderer.windows[0].texture_view.is_none(),
        "no texture after upsert_window_rect"
    );

    // prepare should skip this window — no buffers created
    renderer.prepare_window_resources();

    let w = &renderer.windows[0];
    assert!(
        w.cached_uniform_buffer.is_none(),
        "no uniform buffer for textureless window"
    );
    assert!(
        w.cached_vertex_buffer.is_none(),
        "no vertex buffer for textureless window"
    );

    Ok(())
}

/// Integration test: full headless GPU round-trip — clear a render
/// target to a known color and read it back. Exercises texture creation,
/// render pass, GPU→CPU copy, and async buffer mapping through the
/// headless WGPU pipeline without requiring a surface.
#[tokio::test]
#[serial_test::serial]
async fn test_headless_clear_readback() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let renderer = AxiomRenderer::new_headless().await?;

    // Use 64×64 so bytes_per_row (256) satisfies COPY_BYTES_PER_ROW_ALIGNMENT.
    // Smaller widths would fail validation on D3D12 / Metal backends.
    let width = 64u32;
    let height = 64u32;
    let pixels = renderer.render_headless_clear_readback(width, height, 64, 128, 192, 255)?;

    // Should get width × height × 4 bytes back
    assert_eq!(pixels.len(), (width * height * 4) as usize, "correct byte count");

    // Verify every pixel matches the clear color
    for chunk in pixels.chunks_exact(4) {
        assert_eq!(
            chunk,
            &[64, 128, 192, 255],
            "every pixel should match the clear color (r=64, g=128, b=192, a=255)"
        );
    }

    Ok(())
}

/// Integration test: `remove_window` must drop cached GPU buffers along
/// with the `RenderedWindow` entry, freeing GPU memory. A fresh window
/// with the same ID added afterward gets clean (None) cache slots.
#[tokio::test]
#[serial_test::serial]
async fn test_remove_window_drops_cached_buffers() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // Populate cached GPU buffers
    renderer.prepare_window_resources();
    assert!(renderer.windows[0].cached_uniform_buffer.is_some());
    assert!(renderer.windows[0].cached_vertex_buffer.is_some());
    assert_eq!(renderer.window_count(), 1);

    // ── Remove the window ────────────────────────────────────────
    assert!(
        renderer.remove_window(1),
        "remove_window should return true for existing window"
    );

    // The entry and its GPU buffers are gone from the vec
    assert_eq!(renderer.window_count(), 0, "window entry removed");

    // ── Re-add a window with the same ID ─────────────────────────
    renderer.add_window(1, (50.0, 50.0), (200.0, 150.0));
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // Fresh entry — cached buffers start clean (None)
    assert!(
        renderer.windows[0].cached_uniform_buffer.is_none(),
        "fresh window starts with no uniform buffer"
    );
    assert!(
        renderer.windows[0].cached_vertex_buffer.is_none(),
        "fresh window starts with no vertex buffer"
    );

    // After prepare, the new window gets its own fresh GPU buffers
    renderer.prepare_window_resources();
    assert!(renderer.windows[0].cached_uniform_buffer.is_some());
    assert!(renderer.windows[0].cached_vertex_buffer.is_some());

    Ok(())
}

/// Integration test: `render_to_headless_target` must reuse the cached
/// projection buffer when called twice with the same dimensions.
#[tokio::test]
#[serial_test::serial]
async fn test_headless_projection_buffer_cache_reuse() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    let tex = make_solid_texture_rgba(128, 64, 32, 255);
    renderer.add_window(1, (0.0, 0.0), (64.0, 64.0));
    renderer.update_window_texture(1, 32, 32, &tex);

    // Cache starts empty
    assert!(!renderer.has_cached_projection(), "cache starts cold");

    // First call populates the cache
    let _pixels = renderer.render_to_headless_target(128, 128)?;
    assert!(
        renderer.has_cached_projection(),
        "cache populated after first call"
    );
    let dims_after_first = renderer.cached_projection_dims();
    assert_eq!(dims_after_first, (128, 128), "cached dims match first call");

    // Second call with same dimensions — cache reused
    let _pixels = renderer.render_to_headless_target(128, 128)?;
    assert!(
        renderer.has_cached_projection(),
        "cache still populated after second call"
    );
    assert_eq!(
        renderer.cached_projection_dims(),
        (128, 128),
        "cached dims unchanged when dimensions match"
    );

    // Third call with different dimensions — cache invalidated and repopulated
    let _pixels = renderer.render_to_headless_target(256, 256)?;
    assert!(
        renderer.has_cached_projection(),
        "cache repopulated after resize"
    );
    assert_eq!(
        renderer.cached_projection_dims(),
        (256, 256),
        "cached dims updated to new dimensions"
    );

    Ok(())
}

/// Integration test: `remove_window` must clear queued shadow and blur
/// entries for the removed window from the internal `window_shadows` and
/// `window_blurs` hashmaps.
#[tokio::test]
#[serial_test::serial]
async fn test_remove_window_clears_shadow_blur_queues() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));

    // Queue shadow and blur effects for window 1
    renderer.queue_shadow(1, (100.0, 200.0), (400.0, 300.0), Default::default());
    renderer.queue_blur(
        1,
        (100.0, 200.0),
        (400.0, 300.0),
        axiom::effects::BlurParams {
            enabled: true,
            radius: 5.0,
            intensity: 0.8,
            background_blur: true,
            window_blur: false,
        },
    );

    // Verify they are queued
    assert!(renderer.has_window_shadow(1), "shadow should be queued");
    assert!(renderer.has_window_blur(1), "blur should be queued");

    // ── Remove the window ────────────────────────────────────────
    assert!(renderer.remove_window(1), "should successfully remove window 1");

    // Both shadow and blur entries must be cleared
    assert!(
        !renderer.has_window_shadow(1),
        "shadow entry should be removed along with window"
    );
    assert!(
        !renderer.has_window_blur(1),
        "blur entry should be removed along with window"
    );

    Ok(())
}

/// Integration test: creates 3 windows, queues shadows for all three,
/// removes the middle one, and verifies only that window's shadow is cleared
/// while the other two windows' shadows remain intact.
#[tokio::test]
#[serial_test::serial]
async fn test_remove_middle_window_clears_only_its_shadow() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;

    // ── Add three windows with distinct positions ────────────────
    renderer.add_window(1, (0.0, 0.0), (200.0, 150.0));
    renderer.add_window(2, (0.0, 160.0), (200.0, 150.0));
    renderer.add_window(3, (0.0, 320.0), (200.0, 150.0));

    // ── Queue shadows for all three ─────────────────────────────
    renderer.queue_shadow(1, (0.0, 0.0), (200.0, 150.0), Default::default());
    renderer.queue_shadow(2, (0.0, 160.0), (200.0, 150.0), Default::default());
    renderer.queue_shadow(3, (0.0, 320.0), (200.0, 150.0), Default::default());

    // ── Queue blurs for all three ────────────────────────────────
    let blur_params = axiom::effects::BlurParams {
        enabled: true,
        radius: 5.0,
        intensity: 0.8,
        background_blur: true,
        window_blur: false,
    };
    renderer.queue_blur(1, (0.0, 0.0), (200.0, 150.0), blur_params.clone());
    renderer.queue_blur(2, (0.0, 160.0), (200.0, 150.0), blur_params.clone());
    renderer.queue_blur(3, (0.0, 320.0), (200.0, 150.0), blur_params);

    // ── Pre-condition: exactly 3 windows ───────────────────────
    assert_eq!(renderer.window_count(), 3, "three windows before removal");

    // ── Verify all three are queued ──────────────────────────────
    assert!(renderer.has_window_shadow(1), "window 1 shadow queued");
    assert!(renderer.has_window_shadow(2), "window 2 shadow queued");
    assert!(renderer.has_window_shadow(3), "window 3 shadow queued");
    assert!(renderer.has_window_blur(1), "window 1 blur queued");
    assert!(renderer.has_window_blur(2), "window 2 blur queued");
    assert!(renderer.has_window_blur(3), "window 3 blur queued");

    // ── Remove the middle window (ID 2) ─────────────────────────
    assert!(renderer.remove_window(2), "should successfully remove window 2");

    // ── Only window 2's shadow and blur entries should be cleared ──
    assert!(
        !renderer.has_window_shadow(2),
        "window 2 shadow entry should be removed"
    );
    assert!(
        !renderer.has_window_blur(2),
        "window 2 blur entry should be removed"
    );

    // ── Windows 1 and 3 shadows remain intact ───────────────────
    assert!(
        renderer.has_window_shadow(1),
        "window 1 shadow should survive removal of window 2"
    );
    assert!(
        renderer.has_window_shadow(3),
        "window 3 shadow should survive removal of window 2"
    );
    assert!(
        renderer.has_window_blur(1),
        "window 1 blur should survive removal of window 2"
    );
    assert!(
        renderer.has_window_blur(3),
        "window 3 blur should survive removal of window 2"
    );

    // ── Window count: 3 - 1 = 2 ─────────────────────────────────
    assert_eq!(renderer.window_count(), 2, "two windows remaining after removal");

    Ok(())
}

/// Integration test: `remove_window` on a completely empty renderer
/// must not panic and must return `false` (no-op).
#[tokio::test]
#[serial_test::serial]
async fn test_remove_window_from_empty_renderer_no_panic() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;

    // Renderer starts with zero windows
    assert_eq!(renderer.window_count(), 0, "renderer starts empty");

    // ── Remove from empty renderer ───────────────────────────────
    // Must not panic and must return false.
    assert!(
        !renderer.remove_window(1),
        "remove_window on empty renderer should return false"
    );
    assert!(
        !renderer.remove_window(999),
        "remove_window on empty renderer should return false for any ID"
    );

    // State unchanged
    assert_eq!(renderer.window_count(), 0, "window_count still zero");
    assert!(
        renderer.windows.is_empty(),
        "windows vec still empty"
    );

    Ok(())
}

/// Integration test: `remove_window` with a non-existent ID must not panic.
/// Verify it's a no-op: `window_count` stays the same and previously-added
/// windows are completely unaffected (their cache slots remain intact).
#[tokio::test]
#[serial_test::serial]
async fn test_remove_window_nonexistent_id_noop() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;
    renderer.add_window(1, (100.0, 200.0), (400.0, 300.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);
    renderer.prepare_window_resources();

    // Snapshot state before the no-op remove
    let count_before = renderer.window_count();
    assert_eq!(count_before, 1);
    assert!(renderer.windows[0].cached_uniform_buffer.is_some());
    assert!(renderer.windows[0].cached_vertex_buffer.is_some());

    // ── Remove a non-existent ID (999) ───────────────────────────
    // Must not panic and must return false (no-op).
    assert!(
        !renderer.remove_window(999),
        "remove_window should return false for non-existent ID"
    );

    // No change whatsoever
    assert_eq!(
        renderer.window_count(),
        count_before,
        "window_count unchanged after removing non-existent ID"
    );
    assert!(
        renderer.windows[0].cached_uniform_buffer.is_some(),
        "uniform buffer untouched after no-op remove"
    );
    assert!(
        renderer.windows[0].cached_vertex_buffer.is_some(),
        "vertex buffer untouched after no-op remove"
    );
    assert_eq!(renderer.windows[0].id, 1, "window id unchanged");
    assert_eq!(renderer.windows[0].position, (100.0, 200.0));
    assert_eq!(renderer.windows[0].size, (400.0, 300.0));

    Ok(())
}

/// Helper: build an N×M RGBA texture where every pixel is the same solid color.
fn make_solid_texture_rgba(r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
    let w = 32usize;
    let h = 32usize;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for _ in 0..(w * h) {
        pixels.push(r);
        pixels.push(g);
        pixels.push(b);
        pixels.push(a);
    }
    pixels
}

/// Integration test: two overlapping textured windows must produce
/// alpha-blended pixels in the overlap region (not just window A's
/// color and not just window B's color), proving that the GPU
/// compositor blend pipeline is active and correct.
///
/// Layout (128×128 target, BGRA sRGB; width=128 so bytes_per_row=512
/// satisfies COPY_BYTES_PER_ROW_ALIGNMENT=256):
///   Window 1 (red, opaque):   pos (0, 0)  size 64×64
///   Window 2 (green, α=0.5):  pos (32,32) size 64×64
///
/// Regions after composite:
///   - (8,8)    → red-only, high R, low G
///   - (72,72)  → green-only (below window 1, inside window 2), high G, low R
///   - (48,48)  → overlap, both R and G elevated (blend)
#[tokio::test]
#[serial_test::serial]
async fn test_two_overlapping_windows_alpha_blend() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;

    // ── Window 1: solid red, fully opaque, at top-left ────────────
    let red_tex = make_solid_texture_rgba(255, 0, 0, 255);
    renderer.add_window(1, (0.0, 0.0), (64.0, 64.0));
    renderer.update_window_texture(1, 32, 32, &red_tex);

    // ── Window 2: solid green, half-opacity, offset to overlap ────
    let green_tex = make_solid_texture_rgba(0, 255, 0, 255);
    // Use upsert for window 2 so we can set opacity=0.5 directly.
    // add_window would default to opacity=1.0.
    renderer.upsert_window_rect(2, (32.0, 32.0), (64.0, 64.0), 0.5);
    renderer.update_window_texture(2, 32, 32, &green_tex);

    // ── Composite via render pipeline ─────────────────────────────
    let width = 128u32;
    let height = 128u32;
    let pixels = renderer.render_to_headless_target(width, height)?;
    assert_eq!(pixels.len(), (width * height * 4) as usize, "correct byte count");

    // Helper: fetch (R, G, B, A) at (x, y).
    // BGRA sRGB target → bytes at idx+0=B, idx+1=G, idx+2=R, idx+3=A.
    let sample = |x: u32, y: u32| -> (u8, u8, u8, u8) {
        let idx = ((y * width + x) * 4) as usize;
        (
            pixels[idx + 2], // R
            pixels[idx + 1], // G
            pixels[idx + 0], // B
            pixels[idx + 3], // A
        )
    };

    // ── Red-only region (8, 8) ────────────────────────────────────
    let (r, g, b, a) = sample(8, 8);
    assert!(r > 200, "red-only: R should be high, got {}", r);
    assert!(g < 10, "red-only: G should be low, got {}", g);
    assert!(b < 10, "red-only: B should be low, got {}", b);
    assert!(a > 200, "red-only: A should be opaque, got {}", a);

    // ── Green-only region (72, 72) ────────────────────────────────
    // y=72 is below window 1's bottom edge (64), inside window 2.
    // Semi-transparent green over transparent black → dim green.
    let (r, g, b, a) = sample(72, 72);
    assert!(r < 10, "green-only: R should be low, got {}", r);
    assert!(g > 50, "green-only: G should be elevated even with opacity=0.5, got {}", g);
    assert!(b < 10, "green-only: B should be low, got {}", b);
    // With alpha blending: a = 1.0*src.a + (1-src.a)*dst.a = 0.5 + 0.5·0 = 0.5 → byte 128
    assert!(a < 200, "green-only: A should reflect partial opacity, got {}", a);

    // ── Overlap region (48, 48) ───────────────────────────────────
    // Red drawn first, then semi-transparent green on top.
    // Blend = 0.5·green + 0.5·red → both R and G channels non-trivial.
    let (r, g, b, a) = sample(48, 48);
    assert!(
        r > 50,
        "overlap: R should be non-trivial (blend includes red channel), got {}", r
    );
    assert!(
        g > 50,
        "overlap: G should be non-trivial (blend includes green channel), got {}", g
    );
    assert!(
        b < 10,
        "overlap: B should stay low (neither texture contributes blue), got {}", b
    );
    // Overlap alpha: 1.0*0.5 + 0.5*1.0 = 1.0 → byte 255
    assert!(
        a > 200,
        "overlap: A should be nearly opaque, got {}", a
    );

    // ── Proof of blending ─────────────────────────────────────────
    // Overlap is neither pure-red (r>200,g<10) nor pure-green (r<10,g>50).
    // Both channels are elevated → GPU alpha blending is active.
    assert!(
        r > 10 || g > 10,
        "sanity: overlap pixel is not transparent black"
    );

    Ok(())
}

/// Integration test: reverse draw order from the alpha-blend test —
/// green drawn FIRST (bottom), red SECOND (top, opaque). The overlap
/// region at (48,48) must show red-dominant pixels (high R, low G)
/// because the opaque red window completely covers the underlying
/// green, proving the GPU blend pipeline correctly handles draw order.
///
/// Layout (128×128 target, BGRA sRGB):
///   Window 1 (green, α=0.5): pos (0, 0)  size 64×64 — drawn first
///   Window 2 (red, opaque):   pos (32,32) size 64×64 — drawn on top
///
/// Regions after composite:
///   - (8,8)    → green-only (below window 2), dim green (G elevated, R low)
///   - (72,72)  → red-only (below window 1, inside window 2), high R, low G
///   - (48,48)  → overlap, red covers green → high R, low G (red-dominant)
#[tokio::test]
#[serial_test::serial]
async fn test_reverse_draw_order_red_dominant_blend() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;

    // ── Window 1: solid green, half-opacity, at top-left (bottom) ──
    // Use upsert for window 1 with opacity=0.5.
    let green_tex = make_solid_texture_rgba(0, 255, 0, 255);
    renderer.upsert_window_rect(1, (0.0, 0.0), (64.0, 64.0), 0.5);
    renderer.update_window_texture(1, 32, 32, &green_tex);

    // ── Window 2: solid red, fully opaque, offset to overlap (top) ──
    // add_window defaults to opacity=1.0 — drawn second, covers green.
    let red_tex = make_solid_texture_rgba(255, 0, 0, 255);
    renderer.add_window(2, (32.0, 32.0), (64.0, 64.0));
    renderer.update_window_texture(2, 32, 32, &red_tex);

    // ── Composite via render pipeline ─────────────────────────────
    let width = 128u32;
    let height = 128u32;
    let pixels = renderer.render_to_headless_target(width, height)?;
    assert_eq!(pixels.len(), (width * height * 4) as usize, "correct byte count");

    // Helper: fetch (R, G, B, A) at (x, y).
    // BGRA sRGB target → bytes at idx+0=B, idx+1=G, idx+2=R, idx+3=A.
    let sample = |x: u32, y: u32| -> (u8, u8, u8, u8) {
        let idx = ((y * width + x) * 4) as usize;
        (
            pixels[idx + 2], // R
            pixels[idx + 1], // G
            pixels[idx + 0], // B
            pixels[idx + 3], // A
        )
    };

    // ── Green-only region (8, 8) ──────────────────────────────────
    // Window 1 (green, α=0.5) drawn first, no red overlap here.
    // Semi-transparent green over transparent black → dim green.
    let (r, g, b, a) = sample(8, 8);
    assert!(r < 10, "green-only: R should be low, got {}", r);
    assert!(g > 50, "green-only: G should be elevated (even with α=0.5), got {}", g);
    assert!(b < 10, "green-only: B should be low, got {}", b);
    assert!(a < 200, "green-only: A should reflect partial opacity, got {}", a);

    // ── Red-only region (72, 72) ───────────────────────────────────
    // y=72 is below green window 1's bottom edge (64), inside red window 2.
    // Opaque red over transparent black → pure red.
    let (r, g, b, a) = sample(72, 72);
    assert!(r > 200, "red-only: R should be high (opaque red), got {}", r);
    assert!(g < 10, "red-only: G should be low, got {}", g);
    assert!(b < 10, "red-only: B should be low, got {}", b);
    assert!(a > 200, "red-only: A should be opaque, got {}", a);

    // ── Overlap region (48, 48) ───────────────────────────────────
    // Green drawn first, then opaque red on top.
    // Blend = 1.0·red + 0.0·green → pure red → red-dominant.
    let (r, g, b, a) = sample(48, 48);
    assert!(
        r > 200,
        "overlap: R should be high (red dominates), got {}", r
    );
    assert!(
        g < 30,
        "overlap: G should be low (green covered by opaque red), got {}", g
    );
    assert!(
        b < 10,
        "overlap: B should stay low, got {}", b
    );
    assert!(
        a > 200,
        "overlap: A should be opaque (red is opaque), got {}", a
    );

    // ── Proof of red-dominant blending via draw order ─────────────
    // Compare with the original alpha_blend test: there, red was bottom
    // and green (α=0.5) blended on top → both R and G elevated.
    // Here, red is on top (opaque) → overlap is pure red, proving draw
    // order matters and the GPU blend pipeline handles it correctly.
    assert!(
        r > g * 5,
        "overlap: R should be >5x G (red dominates, not a blend), R={} G={}", r, g
    );

    Ok(())
}

/// Integration test: full textured-quad composite to a headless target.
/// Uploads a real RGBA window texture, runs the compositor render pipeline
/// (projection → vertex shader → textured quad fragment shader with alpha
/// blending) to a headless target, reads back, and verifies the output
/// contains non-zero pixels where the quad was drawn.
#[tokio::test]
#[serial_test::serial]
async fn test_render_textured_quad_to_headless() -> Result<()> {
    use axiom::renderer::AxiomRenderer;

    let mut renderer = AxiomRenderer::new_headless().await?;

    // Add a window at the top-left and give it a real 64×64 texture
    renderer.add_window(1, (0.0, 0.0), (64.0, 64.0));
    let texture_data = make_test_texture_rgba();
    renderer.update_window_texture(1, 64, 64, &texture_data);

    // Composite to a 128×128 headless target (window fills top-left quadrant)
    let pixels = renderer.render_to_headless_target(128, 128)?;

    assert_eq!(
        pixels.len(),
        128 * 128 * 4,
        "correct byte count for 128×128 RGBA"
    );

    // The top-left 64×64 should contain non-zero (colored) pixels from the
    // textured quad. The rest should be transparent black (clear color).
    let mut any_nonzero = false;
    let mut any_zero = false;
    for y in 0..128u32 {
        for x in 0..128u32 {
            let idx = ((y * 128 + x) * 4) as usize;
            let is_nonzero =
                pixels[idx] != 0 || pixels[idx + 1] != 0 || pixels[idx + 2] != 0;
            if x < 64 && y < 64 {
                if is_nonzero {
                    any_nonzero = true;
                }
            } else {
                if !is_nonzero {
                    any_zero = true;
                }
            }
        }
    }

    assert!(
        any_nonzero,
        "top-left quadrant should contain non-zero pixels from the textured quad"
    );
    assert!(
        any_zero,
        "outside the window bounds should be transparent black (clear color)"
    );

    Ok(())
}

// ============================================================================
// Effects Stress & Multi-Anim Tests
// ============================================================================

/// Test multiple animation types running concurrently
#[tokio::test]
async fn test_effects_multiple_animation_types() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    let config = EffectsConfig::default();
    let mut effects = EffectsEngine::new(&config)?;

    // Window open animation
    effects.animate_window_open(2001);
    // Window close animation
    effects.animate_window_close(2002);
    // Window move animation
    effects.animate_window_move(2003, (0.0, 0.0), (500.0, 300.0));

    // Run several update cycles to let animations progress
    for _ in 0..10 {
        effects.update()?;
    }

    // Verify effects are tracked for all three windows
    assert!(effects.get_window_effects(2001).is_some());
    assert!(effects.get_window_effects(2002).is_some());
    assert!(effects.get_window_effects(2003).is_some());

    let (_frame_time, _quality, active_count) = effects.get_performance_stats();
    assert!(active_count > 0, "should have active effects");

    effects.shutdown();

    Ok(())
}

/// Test effects config update propagates correctly
#[tokio::test]
async fn test_effects_config_propagation() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    let config = EffectsConfig::default();
    let mut effects = EffectsEngine::new(&config)?;

    // Get baseline
    let (_, quality_before, _) = effects.get_performance_stats();

    // Update config with new values
    let mut new_config = EffectsConfig::default();
    new_config.blur.radius = 12;
    new_config.blur.intensity = 0.5;
    effects.update_config(new_config);

    // Should not panic and quality should remain valid
    let (_, quality_after, _) = effects.get_performance_stats();
    assert!((0.0..=1.0).contains(&quality_before));
    assert!((0.0..=1.0).contains(&quality_after));

    effects.shutdown();

    Ok(())
}

fn run_spring_trajectory_assertions(
    effects: &mut axiom::effects::EffectsEngine,
    wid: u64,
    frames: usize,
    frame_dt: Duration,
) -> Result<()> {
    // 1. Register a window via animate_window_open. This seeds:
    //    - WindowEffectState.scale = 0.8, opacity = 0.0
    //    - WindowEffectState.opened_at = Some(Instant::now())
    //    - AnimationController starts two springs:
    //        "scale"    : k=250, d=25, m=1, current=0.8 → target=1.0
    //        "opacity"  : k=300, d=28, m=1, current=0.0 → target=1.0
    effects.animate_window_open(wid);
    // Copy scalars out of `initial` so the immutable borrow ends before
    // the simulation loop calls `effects.update()` (which needs &mut self).
    let initial = effects
        .get_window_effects(wid)
        .expect("animate_window_open must create an effect entry");
    let initial_opened_at_some = initial.opened_at.is_some();
    let initial_scale = initial.scale;
    let initial_opacity = initial.opacity;
    assert!(
        initial_opened_at_some,
        "opened_at should be Some(Instant) after animate_window_open",
    );
    assert!(
        (initial_scale - 0.8).abs() < 1e-4,
        "initial scale should be 0.8, got {}",
        initial_scale,
    );
    assert!(
        initial_opacity.abs() < 1e-4,
        "initial opacity should be 0.0, got {}",
        initial_opacity,
    );

    // 2. Drive `frames` ticks at `frame_dt` each. With stiffness=250
    //    the spring envelope decays as exp(-ζ·ω·t) and ζ≈0.79 →
    //    envelope·e^(-12.5·t), so at this test's wall-clock-equivalent
    //    simulated time the spring is well past its first peak
    //    (peak_time = π/(ω·√(1-ζ²)) ≈ 0.32s) and well within the
    //    final-state tolerance. The controller clamps dt ≤ 50ms
    //    so frames wider than that still produce a steady
    //    simulation.
    let mut scales = vec![initial_scale];
    let mut alphas = vec![initial_opacity];
    let mut peak_scale = initial_scale;
    let mut peak_alpha = initial_opacity;
    for _ in 0..frames {
        // Real-time tick: the controller's dt is measured wall-clock
        // between calls. std::thread::sleep blocks the tokio runtime
        // thread but is fine here — this test owns the engine and
        // has no concurrent awaiters.
        std::thread::sleep(frame_dt);
        effects.update()?;
        // Scope the immutable borrow so it ends before the next
        // iteration's `effects.update()` (mirrors the `initial`
        // fix above).
        let (s, a) = {
            let st = effects
                .get_window_effects(wid)
                .expect("entry persists across updates");
            (st.scale, st.opacity)
        };
        peak_scale = peak_scale.max(s);
        peak_alpha = peak_alpha.max(a);
        scales.push(s);
        alphas.push(a);
    }

    let final_scale = *scales.last().unwrap();
    let final_opacity = *alphas.last().unwrap();

    // 3. Final-state convergence: scale & opacity near 1.0 after
    //    `frames`. Tolerance of ±0.05 absorbs scheduler-induced
    //    dt jitter on slow CI runners while catching moderate
    //    tuning regressions that don't fully settle.
    assert!(
        (final_scale - 1.0).abs() < 0.05,
        "after {} frames scale should converge to ~1.0 (got {}, trajectory={:?})",
        frames,
        final_scale,
        scales,
    );
    assert!(
        (final_opacity - 1.0).abs() < 0.05,
        "after {} frames opacity should converge to ~1.0 (got {}, trajectory={:?})",
        frames,
        final_opacity,
        alphas,
    );

    // 4. Bounded overshoot: scale CAN overshoot (engine only clamps
    //    opacity, not scale); hard ceiling so any future tuning
    //    that pushes the spring past 15% over-target fails loudly.
    const OVERSHOOT_CEILING: f32 = 1.15;
    assert!(
        peak_scale <= OVERSHOOT_CEILING,
        "peak scale {} exceeds ceiling {} (trajectory={:?})",
        peak_scale,
        OVERSHOOT_CEILING,
        scales,
    );
    // Opacity is engine-clamped to [0.0, 1.0] on every write, so
    // peak_alpha is bounded above at 1.0 trivially; the invariant
    // we want here is that the engine actually drives opacity up
    // toward 1.0 (i.e. the spring didn't stall at 0).
    assert!(
        peak_alpha >= 0.95,
        "peak alpha {} never reached within 5% of full opacity — \
         spring animation stalled or `EffectsEngine::update()` did \
         not propagate (trajectory={:?})",
        peak_alpha,
        alphas,
    );

    // 5. Rendered rect/alpha match the spring trajectory: replicate
    //    the centered-scale + offset math from the GL render path
    //    on CPU. For animate_window_open, position_offset stays at
    //    (0, 0) so the render output matches the unscaled rect
    //    exactly once the spring settles.
    let rect = TestRect { x: 100, y: 200, width: 800, height: 600 };
    let (rendered_w, rendered_h, rendered_x, rendered_y, rendered_alpha) =
        render_rect_from_state(&rect, final_scale, (0.0, 0.0), final_opacity);
    let expected_w = (rect.width as f32) * final_scale;
    let expected_h = (rect.height as f32) * final_scale;
    assert!(
        (rendered_w - expected_w).abs() < 1e-3,
        "rendered width should equal rect.width * scale ({} vs {})",
        rendered_w,
        expected_w,
    );
    assert!(
        (rendered_h - expected_h).abs() < 1e-3,
        "rendered height should equal rect.height * scale ({} vs {})",
        rendered_h,
        expected_h,
    );
    assert!(
        (rendered_x - rect.x as f32).abs() < 1e-3,
        "rendered x should equal rect.x when scale=1 and offset=0 ({} vs {})",
        rendered_x,
        rect.x,
    );
    assert!(
        (rendered_y - rect.y as f32).abs() < 1e-3,
        "rendered y should equal rect.y when scale=1 and offset=0 ({} vs {})",
        rendered_y,
        rect.y,
    );
    // rendered_alpha is engine-clamped to [0, 1]; after the spring
    // settles it's effectively 1.0 modulo FP rounding, so a tighter
    // ±0.05 is appropriate.
    assert!(
        (rendered_alpha - 1.0).abs() < 0.05,
        "rendered alpha (u_alpha) should be ~1.0, got {}",
        rendered_alpha,
    );

    // 6. Mid-trajectory shape: a few frames in, the spring is still
    //    rising toward 1.0; the midpoint should already be in the
    //    upper half of the trajectory.
    let early_idx = 3.min(scales.len() - 1);
    let mid_idx = scales.len() / 2;
    assert!(
        scales[early_idx] >= initial_scale - 1e-4,
        "early-trajectory scale {} should be ≥ initial scale {} (spring rising)",
        scales[early_idx], initial_scale,
    );
    assert!(
        (scales[mid_idx] - 1.0).abs() < 0.20,
        "mid-trajectory scale should be roughly tracking 1.0, got {}",
        scales[mid_idx],
    );

    // Trajectory dump is debug-level so CI stdout stays clean by
    // default; opt in with `RUST_LOG=axiom=debug cargo test ...` to
    // see the full trajectory for tuning regressions. The
    // module-level `init_test_logger()` ensures env_logger is bound
    // so the env-var gate actually works (without the binding,
    // `log::debug!` is silently dropped in test binaries).
    log::debug!(
        "📊 Spring trajectory ({} frames @ {}ms): \
         scale {:.4} → {:.4} (peak {:.4}); \
         alpha {:.4} → {:.4} (peak {:.4})",
        frames, frame_dt.as_millis(),
        initial_scale, final_scale, peak_scale,
        initial_opacity, final_opacity, peak_alpha,
    );

    Ok(())
}

/// One-time env_logger binding for the integration-test binary.
///
/// Without this, `log::debug!` calls in the spring-trajectory tests
/// are silently dropped because `main.rs`'s `env_logger::init()`
/// only runs for the `axiom` binary, not for the test harness.
/// `init_test_logger()` is idempotent (guarded by `Once`) and uses
/// `default_filter_or("off")` so CI stays silent unless the user
/// opts in with e.g. `RUST_LOG=axiom=debug cargo test ...`.
fn init_test_logger() {
    use std::sync::Once;
    static LOGGER_INIT: Once = Once::new();
    LOGGER_INIT.call_once(|| {
        let _ = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("off"),
        )
        .try_init();
    });
}

/// Thorough test: 60 frames × 16ms ≈ 960ms simulated. Spring
/// envelope at end ≈ exp(-12·t) ≈ 6e-6, so final-state convergence
/// is essentially exact and the catch surface is "anything that
/// doesn't fully settle over the long run".
#[tokio::test]
async fn test_animation_spring_trajectory_bounds() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    init_test_logger();
    let config = EffectsConfig::default();
    let mut effects = EffectsEngine::new(&config)?;
    run_spring_trajectory_assertions(&mut effects, 9001, 60, Duration::from_millis(16))?;
    effects.shutdown();
    Ok(())
}

/// Fast sibling: 30 frames × 16ms ≈ 480ms simulated for CI speed,
/// same assertion surface as the bounds test. Spring envelope
/// exp(-6·t) ≈ 2.5e-3 at t=0.5s, so final_scale still settles
/// within ±0.05 and the peak overshoot is well below the 1.15
/// ceiling. Pair the two tests to catch both "fast-path" and
/// "long-run" regressions.
#[tokio::test]
async fn test_animation_spring_trajectory_quick() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    init_test_logger();
    let config = EffectsConfig::default();
    let mut effects = EffectsEngine::new(&config)?;
    run_spring_trajectory_assertions(&mut effects, 9002, 30, Duration::from_millis(16))?;
    effects.shutdown();
    Ok(())
}

/// CPU mirror of `src/backend/mod.rs::render()`'s per-window
/// centered-scale + offset transform. Returns the (width, height,
/// x, y, alpha) the GL render path would feed into the textured
/// quad draw for the given input rect and live `WindowEffectState`
/// scalars — without any GPU/shader machinery so the integration
/// test can verify the math.
fn render_rect_from_state(
    rect: &TestRect,
    scale: f32,
    offset: (f32, f32),
    opacity: f32,
) -> (f32, f32, f32, f32, f32) {
    let scale = scale.max(0.0);
    let rw = (rect.width as f32) * scale;
    let rh = (rect.height as f32) * scale;
    // Clamp to ≥0 BEFORE recomputing x′/y′ so a negative-season
    // spring collapse cleanly hides the window instead of producing
    // a flipped rect, mirroring the production render path.
    let rw = rw.max(0.0);
    let rh = rh.max(0.0);
    let cx = (rect.x as f32) + (rect.width as f32) / 2.0 + offset.0;
    let cy = (rect.y as f32) + (rect.height as f32) / 2.0 + offset.1;
    let x = cx - rw / 2.0;
    let y = cy - rh / 2.0;
    // Alpha (shader u_alpha) is fed to GL_BLEND; for opaque
    // rendering it sits in [0.0, 1.0].
    let alpha = opacity.clamp(0.0, 1.0);
    (rw, rh, x, y, alpha)
}

/// Rect shape used by [`render_rect_from_state`] — a CPU-side
/// mirror of the GPU-fed rects in the real render path.
struct TestRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
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
        sender.send(cmd).unwrap();
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

/// Test IPC EffectsControl message flow
#[tokio::test]
#[serial_test::serial]
async fn test_ipc_effects_control_flow() -> Result<()> {
    use axiom::config::AxiomConfig;
    use axiom::ipc::LazyUIMessage;

    let mut config = AxiomConfig::default();
    let mut ipc_server = AxiomIPCServer::new();

    ipc_server.start()?;

    let cmd = LazyUIMessage::EffectsControl {
        enabled: Some(false),
        blur_radius: Some(8.0),
        animation_speed: None,
    };

    if let Some(sender) = ipc_server.command_sender_for_test() {
        sender.send(cmd).unwrap();
    }

    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    // EffectsControl is forwarded to pending_actions
    assert!(!changed);
    assert_eq!(actions.len(), 1);

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

    let original_blur = config.effects.blur.radius;

    let mut changes = HashMap::new();
    changes.insert("effects.blur.radius".into(), serde_json::json!(18.0));
    let cmd = LazyUIMessage::OptimizeConfig {
        changes,
        reason: "test".into(),
    };

    if let Some(sender) = ipc_server.command_sender_for_test() {
        sender.send(cmd).unwrap();
    }

    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    assert!(changed, "OptimizeConfig should change config");
    assert!(actions.is_empty(), "no pending actions for config changes");
    assert_eq!(
        config.effects.blur.radius, 18,
        "blur radius should be updated"
    );
    assert_ne!(config.effects.blur.radius, original_blur);

    ipc_server.shutdown().await?;

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
    let (mut compositor, ..) = make_test_compositor(AxiomConfig::default()).await?;

    // Clean tick with 0 errors should succeed
    assert!(
        compositor.tick_for_test().await.is_ok(),
        "clean tick should return Ok"
    );
    assert!(
        compositor.is_running(),
        "compositor should be running after clean tick"
    );

    // Simulate 3 consecutive error ticks, then a clean tick.
    // The clean tick should reset the count (3 < 5).
    for _ in 0..3 {
        compositor.force_next_tick_error();
        compositor.tick_for_test().await?;
    }
    // Clean tick — count was 3, now resets to 0
    compositor.tick_for_test().await?;
    assert!(
        compositor.is_running(),
        "compositor should survive after error reset"
    );

    // Now simulate exactly 5 consecutive errors — should trigger shutdown.
    // The reset proves the count started from 0, not a residual value.
    for _ in 0..4 {
        compositor.force_next_tick_error();
        compositor.tick_for_test().await?; // First 4 should succeed
    }
    assert!(
        compositor.is_running(),
        "should still be running after 4 errors"
    );
    // 5th error triggers emergency shutdown — must return Err
    compositor.force_next_tick_error();
    let result = compositor.tick_for_test().await;
    assert!(
        result.is_err(),
        "5th consecutive error should trigger shutdown"
    );
    assert!(
        !compositor.is_running(),
        "compositor should stop after 5 consecutive errors"
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
        sender.send(LazyUIMessage::HealthCheck).unwrap();
        sender.send(LazyUIMessage::GetPerformanceReport).unwrap();
    }

    let (changed, actions) = ipc_server.process_messages(&mut config)?;

    assert!(!changed, "HealthCheck should not change config");
    assert!(actions.is_empty(), "HealthCheck produces no actions");

    // Config should be unchanged
    assert_eq!(config.effects.blur.radius, config_clone.effects.blur.radius);

    ipc_server.shutdown().await?;

    Ok(())
}
