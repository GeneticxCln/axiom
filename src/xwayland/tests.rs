use super::*;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_xwayland_manager_lifecycle() {
    // Skip if XWayland is not installed
    match tokio::process::Command::new("which")
        .arg("Xwayland")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {}
        _ => {
            println!("Skipping XWayland test: Xwayland not found");
            return;
        }
    }

    let config = XWaylandConfig {
        enabled: true,
        display: None,
    };

    log::info!("Starting XWayland manager test...");
    let mut manager = XWaylandManager::new(&config)
        .await
        .expect("Failed to create manager");

    // Give it a moment to start
    sleep(Duration::from_millis(500)).await;

    // Verify state
    // Since we are in a submodule, we can access private fields of XWaylandManager
    match manager.server_state {
        XWaylandServerState::Running => {
            assert!(manager.xwayland_process.is_some());
            assert!(manager.display_number.is_some());
            assert!(std::env::var("DISPLAY").is_ok());
            if let Some(display) = manager.display_number {
                log::info!("XWayland started successfully on :{}", display);
            }
        }
        _ => {
            log::warn!("XWayland did not start (State: {:?})", manager.server_state);
        }
    }

    // Test shutdown
    manager.shutdown().await.expect("Failed to shutdown");

    assert_eq!(manager.server_state, XWaylandServerState::Stopped);
    assert!(manager.xwayland_process.is_none());
    assert!(manager.display_number.is_none());
    assert!(std::env::var("DISPLAY").is_err());
}
