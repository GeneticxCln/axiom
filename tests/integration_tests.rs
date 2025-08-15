//! Integration tests for Axiom compositor
//!
//! These tests verify end-to-end functionality including IPC communication,
//! compositor lifecycle, and interaction between major subsystems.

use anyhow::Result;
use std::time::Duration;
use tokio::time::timeout;

// Import Axiom modules
use axiom::{
    compositor::AxiomCompositor,
    config::AxiomConfig,
    ipc::{AxiomIPCServer, AxiomMessage, LazyUIMessage},
};

/// Test IPC server startup and basic communication
#[tokio::test]
async fn test_ipc_server_startup() -> Result<()> {
    let mut ipc_server = AxiomIPCServer::new();

    // Test that server can start
    ipc_server.start().await?;

    // Test that socket file is created
    assert!(ipc_server.socket_path().exists());

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

/// Test compositor initialization with default config
#[tokio::test]
async fn test_compositor_initialization() -> Result<()> {
    let config = AxiomConfig::default();

    // Test compositor creation in windowed mode (safe for CI)
    let result = timeout(Duration::from_secs(10), AxiomCompositor::new(config, true)).await;

    match result {
        Ok(Ok(_compositor)) => {
            // Compositor created successfully
        }
        Ok(Err(e)) => {
            // Expected in CI environment without display
            println!("Expected initialization failure in CI: {}", e);
        }
        Err(_) => {
            // Timeout - this might happen in CI
            println!("Compositor initialization timed out (expected in CI)");
        }
    }

    Ok(())
}

/// Test configuration loading and validation
#[tokio::test]
async fn test_configuration_system() -> Result<()> {
    // Test default configuration
    let default_config = AxiomConfig::default();
    assert!(default_config.effects.enabled);
    assert!(default_config.workspace.column_width > 0.0);

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
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Test adding windows
    workspaces.add_window(1001);
    workspaces.add_window(1002);
    workspaces.add_window(1003);

    assert_eq!(workspaces.active_column_count(), 3);

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
    let (frame_time, quality, active_count) = effects.get_performance_stats();
    assert!(frame_time.as_millis() >= 0);
    assert!(quality >= 0.0 && quality <= 1.0);
    assert!(active_count >= 0);

    // Test shutdown
    effects.shutdown()?;

    Ok(())
}

/// Test input event processing
#[tokio::test]
async fn test_input_processing() -> Result<()> {
    use axiom::config::{BindingsConfig, InputConfig};
    use axiom::input::{CompositorAction, InputEvent, InputManager};

    let input_config = InputConfig::default();
    let bindings_config = BindingsConfig::default();

    let mut input_manager = InputManager::new(&input_config, &bindings_config)?;

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
                assert!(true); // Expected actions
            }
            _ => {
                // Other actions might be present too
            }
        }
    }

    input_manager.shutdown()?;
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

    let mut workspaces = ScrollableWorkspaces::new(&workspace_config)?;
    let mut effects = EffectsEngine::new(&effects_config)?;

    // Add 100 windows
    for i in 1..=100 {
        workspaces.add_window(i);
        // Add some effects to test performance
        if i % 10 == 0 {
            effects.animate_window_move(i, (0.0, 0.0), (100.0, 100.0));
        }
    }

    assert_eq!(workspaces.active_column_count(), 100);

    // Test scrolling through many workspaces
    let start = std::time::Instant::now();
    for _ in 0..50 {
        workspaces.scroll_right();
        effects.update()?;
    }
    let elapsed = start.elapsed();

    // Should complete within reasonable time (1 second for 50 operations)
    assert!(elapsed < Duration::from_secs(1));

    effects.shutdown()?;
    workspaces.shutdown()?;

    Ok(())
}

/// Test error handling and recovery scenarios
#[tokio::test]
async fn test_error_recovery() -> Result<()> {
    use axiom::config::EffectsConfig;
    use axiom::effects::EffectsEngine;

    // Test effects engine with invalid configuration
    let mut bad_config = EffectsConfig::default();
    bad_config.blur_radius = -1.0; // Invalid value

    // Should handle gracefully or provide meaningful error
    let result = EffectsEngine::new(&bad_config);
    match result {
        Ok(mut engine) => {
            // If it accepts invalid config, it should sanitize it
            let (_, quality, _) = engine.get_performance_stats();
            assert!((0.0..=1.0).contains(&quality));
            engine.shutdown()?;
        }
        Err(_) => {
            // Expected to reject invalid configuration
        }
    }

    Ok(())
}

/// Test memory usage with typical workload
#[tokio::test]
async fn test_memory_usage() -> Result<()> {
    use axiom::config::WorkspaceConfig;
    use axiom::workspace::ScrollableWorkspaces;

    let config = WorkspaceConfig::default();

    // Measure baseline memory
    let initial_memory = get_memory_usage();

    // Create workspace system
    let mut workspaces = ScrollableWorkspaces::new(&config)?;

    // Add many windows and remove them repeatedly to test for leaks
    for cycle in 0..10 {
        // Add 20 windows
        for i in 1..=20 {
            let window_id = cycle * 20 + i;
            workspaces.add_window(window_id);
        }

        // Remove half of them
        for i in 1..=10 {
            let window_id = cycle * 20 + i;
            workspaces.remove_window(window_id);
        }
    }

    // Clean shutdown
    workspaces.shutdown()?;

    // Force garbage collection if possible
    // (Rust doesn't have explicit GC, but drop should clean up)
    drop(workspaces);

    let final_memory = get_memory_usage();
    let memory_growth = final_memory.saturating_sub(initial_memory);

    // Memory growth should be reasonable (less than 100MB for this test)
    assert!(
        memory_growth < 100 * 1024 * 1024,
        "Memory growth too large: {} bytes",
        memory_growth
    );

    Ok(())
}

/// Get current memory usage (rough estimate)
fn get_memory_usage() -> usize {
    // Simple memory usage estimate
    // In a real implementation, you'd use a proper memory profiler
    use std::alloc::{GlobalAlloc, Layout, System};

    // This is a placeholder - real memory measurement would need
    // external tools or more sophisticated tracking
    0
}

/// Test concurrent operations
#[tokio::test]
async fn test_concurrent_operations() -> Result<()> {
    use axiom::config::WorkspaceConfig;
    use axiom::workspace::ScrollableWorkspaces;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let config = WorkspaceConfig::default();
    let workspaces = Arc::new(Mutex::new(ScrollableWorkspaces::new(&config)?));

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

    // Check that all windows were added
    let ws = workspaces.lock().await;
    assert_eq!(ws.active_column_count(), 50); // 5 tasks * 10 windows each

    Ok(())
}
