//! Demo/test module for scrollable workspaces
//!
//! This module provides functionality to demonstrate and test
//! the scrollable workspace system in action.

use anyhow::Result;
use log::{info, debug};
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
    let window1 = compositor.add_window("Terminal".to_string());
    let window2 = compositor.add_window("Browser".to_string());
    let window3 = compositor.add_window("Editor".to_string());
    
    time::sleep(Duration::from_millis(500)).await;
    
    // Show current workspace info
    let (column, position, count, scrolling) = compositor.get_workspace_info();
    info!("🎯 Current: Column {}, Position {:.1}, {} active columns, Scrolling: {}", 
          column, position, count, scrolling);
    
    // Demo 2: Scroll to the right and add more windows
    info!("📋 Demo 2: Scrolling right and adding windows");
    compositor.scroll_workspace_right();
    
    // Wait for scroll animation
    time::sleep(Duration::from_millis(300)).await;
    
    let window4 = compositor.add_window("Calculator".to_string());
    let window5 = compositor.add_window("Files".to_string());
    
    let (column, position, count, scrolling) = compositor.get_workspace_info();
    info!("🎯 After scroll right: Column {}, Position {:.1}, {} active columns", 
          column, position, count);
    
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
    for i in 0..3 {
        compositor.scroll_workspace_left();
        time::sleep(Duration::from_millis(400)).await;
        
        let (column, position, count, scrolling) = compositor.get_workspace_info();
        info!("🎯 Scrolled to: Column {}, Position {:.1}, {} active columns", 
              column, position, count);
    }
    
    // Demo 6: Clean up some windows
    info!("📋 Demo 6: Removing some windows");
    compositor.remove_window(window2);
    compositor.remove_window(window5);
    
    time::sleep(Duration::from_millis(200)).await;
    
    let (column, position, count, scrolling) = compositor.get_workspace_info();
    info!("🎯 After cleanup: Column {}, Position {:.1}, {} active columns", 
          column, position, count);
    
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
        
        let (column, position, count, _) = compositor.get_workspace_info();
        debug!("📍 Momentum step {}: Column {}, Position {:.1}", i + 1, column, position);
    }
    
    info!("✅ Momentum scrolling demonstration completed!");
    
    Ok(())
}

/// Run a comprehensive workspace test
pub async fn run_comprehensive_test(compositor: &mut AxiomCompositor) -> Result<()> {
    info!("🧪 Running comprehensive scrollable workspace test...");
    
    // Test 1: Basic functionality
    demo_scrollable_workspaces(compositor).await?;
    
    time::sleep(Duration::from_millis(1000)).await;
    
    // Test 2: Momentum scrolling
    demo_momentum_scrolling(compositor).await?;
    
    info!("🎉 All scrollable workspace tests completed successfully!");
    
    Ok(())
}
