// Resource lifecycle integration tests for Axiom compositor
//
// Tests the creation and destruction patterns for core compositor resources
// including windows, buffers, and output configurations.

use axiom::window::WindowManager;
use axiom::config::WindowConfig;
use std::sync::{Arc, RwLock};

#[test]
fn test_window_creation_and_destruction() {
    // Initialize window manager
    let config = WindowConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&config).unwrap()));
    
    // Create multiple windows
    let id1 = {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window("Test Window 1".to_string())
    };
    
    let id2 = {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window("Test Window 2".to_string())
    };
    
    let id3 = {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window("Test Window 3".to_string())
    };
    
    // Verify all windows exist
    {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.get_window(id1).is_some());
        assert!(wm_guard.get_window(id2).is_some());
        assert!(wm_guard.get_window(id3).is_some());
    }
    
    // Remove middle window
    {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id2);
    }
    
    // Verify correct windows exist
    {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.get_window(id1).is_some());
        assert!(wm_guard.get_window(id2).is_none()); // removed
        assert!(wm_guard.get_window(id3).is_some());
    }
    
    // Remove remaining windows
    {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id1);
        wm_guard.remove_window(id3);
    }
    
    // Verify all removed
    {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.get_window(id1).is_none());
        assert!(wm_guard.get_window(id2).is_none());
        assert!(wm_guard.get_window(id3).is_none());
    }
}

#[test]
fn test_window_state_transitions() {
    let config = WindowConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&config).unwrap()));
    
    // Create a window
    let id = {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window("State Test Window".to_string())
    };
    
    // Test minimize
    {
        let mut wm_guard = wm.write().unwrap();
        assert!(wm_guard.minimize_window(id).is_ok());
    }
    
    {
        let wm_guard = wm.read().unwrap();
        let window = wm_guard.get_window(id).unwrap();
        assert!(window.properties.minimized);
    }
    
    // Test restore from minimized
    {
        let mut wm_guard = wm.write().unwrap();
        assert!(wm_guard.restore_window(id).is_ok());
    }
    
    {
        let wm_guard = wm.read().unwrap();
        let window = wm_guard.get_window(id).unwrap();
        assert!(!window.properties.minimized);
        assert!(!window.properties.maximized);
    }
    
    // Test maximize
    {
        let mut wm_guard = wm.write().unwrap();
        assert!(wm_guard.maximize_window(id).is_ok());
    }
    
    {
        let wm_guard = wm.read().unwrap();
        let window = wm_guard.get_window(id).unwrap();
        assert!(window.properties.maximized);
        assert!(!window.properties.minimized);
    }
    
    // Test fullscreen
    {
        let mut wm_guard = wm.write().unwrap();
        assert!(wm_guard.toggle_fullscreen(id).is_ok());
    }
    
    {
        let wm_guard = wm.read().unwrap();
        let window = wm_guard.get_window(id).unwrap();
        assert!(window.properties.fullscreen);
    }
    
    // Cleanup
    {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id);
    }
}

#[test]
fn test_window_focus_management() {
    let config = WindowConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&config).unwrap()));
    
    // Create multiple windows
    let ids: Vec<u64> = (0..5).map(|i| {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window(format!("Window {}", i))
    }).collect();
    
    // First window should be focused automatically
    {
        let wm_guard = wm.read().unwrap();
        assert_eq!(wm_guard.focused_window_id(), Some(ids[0]));
    }
    
    // Focus each window in sequence
    for &id in &ids {
        {
            let mut wm_guard = wm.write().unwrap();
            assert!(wm_guard.focus_window(id).is_ok());
        }
        
        {
            let wm_guard = wm.read().unwrap();
            assert_eq!(wm_guard.focused_window_id(), Some(id));
        }
    }
    
    // Remove focused window - focus should clear
    let focused_id = {
        let wm_guard = wm.read().unwrap();
        wm_guard.focused_window_id().unwrap()
    };
    
    {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(focused_id);
    }
    
    {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.focused_window_id().is_none());
    }
    
    // Cleanup remaining
    for &id in &ids {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id);
    }
}

#[test]
fn test_window_properties_persistence() {
    let config = WindowConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&config).unwrap()));
    
    let id = {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window("Properties Test".to_string())
    };
    
    // Set custom properties
    {
        let mut wm_guard = wm.write().unwrap();
        if let Some(window) = wm_guard.get_window_mut(id) {
            window.properties.floating = true;
            window.properties.always_on_top = true;
        }
    }
    
    // Verify properties persisted
    {
        let wm_guard = wm.read().unwrap();
        let window = wm_guard.get_window(id).unwrap();
        assert!(window.properties.floating);
        assert!(window.properties.always_on_top);
        assert!(!window.properties.minimized);
        assert!(!window.properties.maximized);
    }
    
    // Modify state
    {
        let mut wm_guard = wm.write().unwrap();
        let _ = wm_guard.maximize_window(id);
    }
    
    // Verify state change and property persistence
    {
        let wm_guard = wm.read().unwrap();
        let window = wm_guard.get_window(id).unwrap();
        assert!(window.properties.floating); // still set
        assert!(window.properties.always_on_top); // still set
        assert!(window.properties.maximized); // newly set
    }
    
    // Cleanup
    {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id);
    }
}

#[test]
fn test_workspace_window_lifecycle() {
    use axiom::workspace::ScrollableWorkspaces;
    use axiom::config::WorkspaceConfig;
    
    let wm_config = WindowConfig::default();
    let ws_config = WorkspaceConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&wm_config).unwrap()));
    let ws = Arc::new(RwLock::new(ScrollableWorkspaces::new(&ws_config).unwrap()));
    
    // Create windows in window manager
    let ids: Vec<u64> = (0..3).map(|i| {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window(format!("WS Window {}", i))
    }).collect();
    
    // Add windows to workspace columns
    for &id in &ids {
        let mut ws_guard = ws.write().unwrap();
        let col = ws_guard.get_focused_column_mut();
        col.add_window(id);
    }
    
    // Verify windows are in workspace
    {
        let ws_guard = ws.read().unwrap();
        for &id in &ids {
            // Windows should be present in workspace
            assert!(ws_guard.window_exists(id), "Window {} not found in workspace", id);
        }
    }
    
    // Remove window from both managers
    let remove_id = ids[1];
    {
        let mut ws_guard = ws.write().unwrap();
        ws_guard.remove_window(remove_id);
    }
    {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(remove_id);
    }
    
    // Verify removal
    {
        let ws_guard = ws.read().unwrap();
        assert!(!ws_guard.window_exists(remove_id), "Window {} should be removed from workspace", remove_id);
    }
    {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.get_window(remove_id).is_none());
    }
    
    // Cleanup remaining
    for &id in &ids {
        let mut ws_guard = ws.write().unwrap();
        ws_guard.remove_window(id);
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id);
    }
}

#[test]
fn test_buffer_texture_lifecycle() {
    use axiom::renderer;
    
    // Queue texture updates for mock windows
    let test_ids = vec![1001, 1002, 1003];
    
    for &id in &test_ids {
        let rgba = vec![255u8, 0, 0, 255]; // red pixel
        renderer::queue_texture_update(id, rgba, 1, 1);
    }
    
    // In a real scenario, these would be consumed by the renderer
    // For this test, we just verify the API doesn't panic
    
    // Remove a texture
    renderer::remove_placeholder_quad(test_ids[1]);
    
    // The API should handle missing IDs gracefully
    renderer::remove_placeholder_quad(99999);
}

#[test]
fn test_output_configuration_changes() {
    // Test output configuration changes via mock data structures
    // LogicalOutput is internal to smithay server, so we test the concept with simple structs
    #[derive(Debug, Clone)]
    struct MockOutput {
        id: u64,
        _name: String,
        width: i32,
        height: i32,
    }
    
    let mut outputs = vec![
        MockOutput { id: 1, _name: "Primary".to_string(), width: 1920, height: 1080 },
        MockOutput { id: 2, _name: "Secondary".to_string(), width: 1920, height: 1080 },
    ];
    
    assert_eq!(outputs.len(), 2);
    
    // Simulate output hotplug (add)
    outputs.push(MockOutput { id: 3, _name: "New Monitor".to_string(), width: 2560, height: 1440 });
    assert_eq!(outputs.len(), 3);
    
    // Simulate output removal (disconnect)
    outputs.retain(|o| o.id != 2);
    assert_eq!(outputs.len(), 2);
    assert!(outputs.iter().any(|o| o.id == 1));
    assert!(outputs.iter().any(|o| o.id == 3));
    assert!(!outputs.iter().any(|o| o.id == 2));
    
    // Simulate resolution change
    if let Some(primary) = outputs.iter_mut().find(|o| o.id == 1) {
        primary.width = 3840;
        primary.height = 2160;
    }
    
    let primary = outputs.iter().find(|o| o.id == 1).unwrap();
    assert_eq!(primary.width, 3840);
    assert_eq!(primary.height, 2160);
}

#[test]
fn test_concurrent_window_operations() {
    use std::thread;
    
    let config = WindowConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&config).unwrap()));
    let mut handles = vec![];
    
    // Spawn multiple threads creating windows concurrently
    for i in 0..10 {
        let wm_clone = Arc::clone(&wm);
        let handle = thread::spawn(move || {
            let mut wm_guard = wm_clone.write().unwrap();
            wm_guard.add_window(format!("Thread {} Window", i))
        });
        handles.push(handle);
    }
    
    // Collect all created IDs
    let ids: Vec<u64> = handles.into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    
    // Verify all windows were created
    {
        let wm_guard = wm.read().unwrap();
        for &id in &ids {
            assert!(wm_guard.get_window(id).is_some(), "Window {} not found", id);
        }
    }
    
    // Cleanup
    {
        let mut wm_guard = wm.write().unwrap();
        for &id in &ids {
            wm_guard.remove_window(id);
        }
    }
}

#[test]
fn test_memory_cleanup_on_window_removal() {
    let config = WindowConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&config).unwrap()));
    
    // Create many windows
    let ids: Vec<u64> = (0..100).map(|i| {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.add_window(format!("Temp Window {}", i))
    }).collect();
    
    // Verify all windows were created
    for &id in &ids {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.get_window(id).is_some());
    }
    
    // Remove all windows
    for &id in &ids {
        let mut wm_guard = wm.write().unwrap();
        wm_guard.remove_window(id);
    }
    
    // Verify all removed
    for &id in &ids {
        let wm_guard = wm.read().unwrap();
        assert!(wm_guard.get_window(id).is_none());
    }
}
