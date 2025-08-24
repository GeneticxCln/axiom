//! Demo/test module for scrollable workspaces
//!
//! This module provides functionality to demonstrate and test
//! the scrollable workspace system in action.

use anyhow::Result;
use log::{debug, info};
use std::time::Duration;
use tokio::time;

use crate::compositor::AxiomCompositor;

/// Demo the scrollable workspace functionality
pub async fn demo_scrollable_workspaces(compositor: &mut AxiomCompositor) -> Result<()> {
    info!("🎭 Starting scrollable workspace demonstration...");

    // Set a reasonable viewport size for demo
    compositor.set_viewport_size(1920, 1080);

    // Demo 1: Add some windows to the initial workspace
    info!("📋 Demo 1: Adding windows to initial workspace");
    let _window1 = compositor.add_window("Terminal".to_string());
    let window2 = compositor.add_window("Browser".to_string());
    let _window3 = compositor.add_window("Editor".to_string());

    time::sleep(Duration::from_millis(500)).await;

    // Show current workspace info
    let (column, position, count, scrolling) = compositor.get_workspace_info();

    info!(
        "🎯 Current: Column {}, Position {:.1}, {} active columns, Scrolling: {}",
        column, position, count, scrolling
    );

    // Demo 2: Scroll to the right and add more windows
    info!("📋 Demo 2: Scrolling right and adding windows");
    compositor.scroll_workspace_right();

    // Wait for scroll animation
    time::sleep(Duration::from_millis(300)).await;

    let window4 = compositor.add_window("Calculator".to_string());
    let window5 = compositor.add_window("Files".to_string());

    let (column, position, count, scrolling) = compositor.get_workspace_info();
    info!(
        "🎯 After scroll right: Column {}, Position {:.1}, {} active columns",
        column, position, count
    );

    // Demo 3: Scroll further right
    info!("📋 Demo 3: Scrolling to column 2");
    compositor.scroll_workspace_right();
    time::sleep(Duration::from_millis(300)).await;

    let window6 = compositor.add_window("Music Player".to_string());

    // Demo 4: Move windows between workspaces
    info!("📋 Demo 4: Moving windows between workspaces");
    compositor.move_window_left(window6); // Move music player to column 1
    time::sleep(Duration::from_millis(200)).await;

    compositor.move_window_left(window4); // Move calculator to column 0
    time::sleep(Duration::from_millis(200)).await;

    // Demo 5: Scroll back to see all workspaces
    info!("📋 Demo 5: Touring all workspaces");
    for _i in 0..3 {
        compositor.scroll_workspace_left();
        time::sleep(Duration::from_millis(400)).await;

        let (column, position, count, _scrolling) = compositor.get_workspace_info();
        info!(
            "🎯 Scrolled to: Column {}, Position {:.1}, {} active columns",
            column, position, count
        );
    }

    // Demo 6: Clean up some windows
    info!("📋 Demo 6: Removing some windows");
    compositor.remove_window(window2);
    compositor.remove_window(window5);

    time::sleep(Duration::from_millis(200)).await;

    let (column, position, count, scrolling) = compositor.get_workspace_info();
    info!(
        "🎯 After cleanup: Column {}, Position {:.1}, {} active columns",
        column, position, count
    );

    info!("✅ Scrollable workspace demonstration completed!");

    Ok(())
}

/// Demo momentum scrolling (simulated)
pub async fn demo_momentum_scrolling(compositor: &mut AxiomCompositor) -> Result<()> {
    info!("🎭 Demonstrating momentum scrolling simulation...");

    // Add some windows across multiple workspaces for visual effect
    let _w1 = compositor.add_window("App 1".to_string());

    compositor.scroll_workspace_right();
    time::sleep(Duration::from_millis(100)).await;
    let _w2 = compositor.add_window("App 2".to_string());

    compositor.scroll_workspace_right();
    time::sleep(Duration::from_millis(100)).await;
    let _w3 = compositor.add_window("App 3".to_string());

    compositor.scroll_workspace_right();
    time::sleep(Duration::from_millis(100)).await;
    let _w4 = compositor.add_window("App 4".to_string());

    // Now demonstrate scrolling back through them smoothly
    info!("🏃 Simulating smooth momentum scroll through workspaces...");

    for i in 0..4 {
        compositor.scroll_workspace_left();
        // Shorter delays to simulate momentum
        time::sleep(Duration::from_millis(150)).await;

        let (column, position, _count, _) = compositor.get_workspace_info();
        debug!(
            "📍 Momentum step {}: Column {}, Position {:.1}",
            i + 1,
            column,
            position
        );
    }

    info!("✅ Momentum scrolling demonstration completed!");

    Ok(())
}

/// Phase 3: Enhanced comprehensive test with input processing
pub async fn run_comprehensive_test(compositor: &mut AxiomCompositor) -> Result<()> {
    info!("🧪 Phase 3: Running comprehensive scrollable workspace test with input processing...");

    // Test 1: Basic scrollable workspace functionality
    demo_scrollable_workspaces(compositor).await?;

    time::sleep(Duration::from_millis(1000)).await;

    // Test 2: Momentum scrolling
    demo_momentum_scrolling(compositor).await?;

    time::sleep(Duration::from_millis(1000)).await;

    // Phase 3: Test input processing
    demo_input_processing(compositor).await?;

    time::sleep(Duration::from_millis(1000)).await;

    // Phase 3: Test enhanced workspace interactions
    demo_enhanced_workspace_features(compositor).await?;

    info!("🎉 All Phase 3 scrollable workspace tests completed successfully!");

    Ok(())
}

/// Phase 3: Demo input processing capabilities
pub async fn demo_input_processing(compositor: &mut AxiomCompositor) -> Result<()> {
    info!("🎭 Phase 3: Demonstrating input processing...");

    // Set up some windows for testing
    let window1 = compositor.add_window("Input Test Window 1".to_string());
    let window2 = compositor.add_window("Input Test Window 2".to_string());

    time::sleep(Duration::from_millis(300)).await;

    // Test input simulation (normally these would come from real input devices)
    info!("📋 Testing simulated keyboard input...");

    // Move to the right workspace with simulated input
    // Note: In a real compositor, these would come from actual keyboard events
    info!("⌨️ Simulating Super+Right key press (scroll right)");
    compositor.scroll_workspace_right();
    time::sleep(Duration::from_millis(400)).await;

    let (_column, position, _count, _scrolling) = compositor.get_workspace_info();
    info!("🎯 Position after simulated input: {:.1}", position);

    // Test window movement
    info!("⌨️ Simulating Super+Shift+Left (move window left)");
    compositor.move_window_left(window1);
    time::sleep(Duration::from_millis(300)).await;

    // Test scroll events (trackpad-like)
    info!("📜 Simulating trackpad scroll gesture...");
    compositor.scroll_workspace_left();
    time::sleep(Duration::from_millis(400)).await;

    let (_column, position, _count, _scrolling) = compositor.get_workspace_info();
    info!("🎯 Position after scroll gesture: {:.1}", position);

    // Clean up
    compositor.remove_window(window1);
    compositor.remove_window(window2);

    info!("✅ Input processing demonstration completed!");
    Ok(())
}

/// Phase 3: Demo enhanced workspace features
pub async fn demo_enhanced_workspace_features(compositor: &mut AxiomCompositor) -> Result<()> {
    info!("🎭 Phase 3: Demonstrating enhanced workspace features...");

    // Test multiple viewport sizes (responsive design)
    info!("📐 Testing responsive workspace layout...");
    compositor.set_viewport_size(1366, 768); // Smaller screen
    time::sleep(Duration::from_millis(200)).await;

    let w1 = compositor.add_window("Small Screen App".to_string());

    compositor.set_viewport_size(2560, 1440); // Larger screen
    time::sleep(Duration::from_millis(200)).await;

    let w2 = compositor.add_window("Large Screen App".to_string());

    // Test rapid workspace navigation
    info!("🏃 Testing rapid workspace navigation...");
    for i in 0..5 {
        compositor.scroll_workspace_right();
        time::sleep(Duration::from_millis(100)).await; // Faster scrolling

        let (_column, position, _count, _scrolling) = compositor.get_workspace_info();
        debug!("⚡ Rapid scroll {}: Position {:.1}", i + 1, position);
    }

    // Test workspace with many windows
    info!("🪟 Testing workspace with multiple windows...");
    compositor.scroll_workspace_right();
    time::sleep(Duration::from_millis(200)).await;

    let windows: Vec<u64> = (1..=6)
        .map(|i| compositor.add_window(format!("Multi-Window App {}", i)))
        .collect();

    let (_column, position, count, _scrolling) = compositor.get_workspace_info();
    info!(
        "🎯 Multi-window workspace: {} columns, position {:.1}",
        count, position
    );

    // Test window movement between multiple workspaces
    info!("🔀 Testing complex window movements...");

    // Move some windows to different workspaces
    for (i, &window_id) in windows.iter().enumerate() {
        if i % 2 == 0 {
            compositor.move_window_left(window_id);
            time::sleep(Duration::from_millis(150)).await;
        }
    }

    // Tour the final result
    info!("🎪 Final workspace tour...");
    for i in 0..3 {
        compositor.scroll_workspace_left();
        time::sleep(Duration::from_millis(300)).await;

        let (column, position, count, _scrolling) = compositor.get_workspace_info();
        info!(
            "🌟 Final tour {}: Column {}, Position {:.1}, {} total columns",
            i + 1,
            column,
            position,
            count
        );
    }

    // Cleanup
    for &window_id in &windows {
        compositor.remove_window(window_id);
    }
    compositor.remove_window(w1);
    compositor.remove_window(w2);

    info!("✅ Enhanced workspace features demonstration completed!");
    Ok(())
}
