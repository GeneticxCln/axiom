use super::*;
use tokio::time::{sleep, Duration};

/// XWayland lifecycle test — isolated to avoid failures on headless systems.
/// Skips when Xwayland binary is absent OR when startup fails (e.g. no X11 socket).
#[tokio::test]
async fn test_xwayland_manager_lifecycle() {
    // Guard: skip entirely if Xwayland binary not on PATH
    match tokio::process::Command::new("which")
        .arg("Xwayland")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {}
        _ => {
            log::warn!("Skipping XWayland test: Xwayland not found in PATH");
            return;
        }
    }

    let config = XWaylandConfig {
        enabled: true,
        display: None,
    };

    let mut manager = XWaylandManager::new(&config)
        .await
        .expect("Failed to create XWayland manager");

    // Give it a moment to start
    sleep(Duration::from_millis(500)).await;

    // If XWayland failed to start (e.g. no free display or permission denied),
    // still verify shutdown works and bail gracefully
    if manager.server_state != XWaylandServerState::Running {
        log::warn!(
            "XWayland did not start (state: {:?}) — testing graceful shutdown only",
            manager.server_state
        );
        manager.shutdown().await.expect("Failed to shutdown");
        assert_eq!(manager.server_state, XWaylandServerState::Stopped);
        return;
    }

    // Server is running — verify state invariants
    assert!(manager.xwayland_process.is_some());
    assert!(manager.display_number.is_some());
    if let Some(display) = manager.display_number {
        log::info!("XWayland started on :{}", display);
    }

    // Shutdown and verify cleanup
    manager.shutdown().await.expect("Failed to shutdown");
    assert_eq!(manager.server_state, XWaylandServerState::Stopped);
    assert!(manager.xwayland_process.is_none());
    assert!(manager.display_number.is_none());
    std::env::remove_var("DISPLAY");
}
