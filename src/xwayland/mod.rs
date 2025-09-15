#![allow(dead_code)]
//! XWayland integration for X11 app compatibility
//! Provides seamless integration of X11 applications in Wayland

use crate::config::XWaylandConfig;
use anyhow::Result;
use log::info;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::process::Child as TokioChild;

/// XWayland manager for X11 application support
pub struct XWaylandManager {
    /// XWayland configuration
    config: XWaylandConfig,

    /// XWayland server process
    xwayland_process: Option<TokioChild>,

    /// X11 display number
    display_number: Option<u32>,

    /// X11 windows managed by XWayland
    x11_windows: HashMap<u32, X11WindowInfo>,

    /// XWayland server state
    server_state: XWaylandServerState,

    /// Statistics
    stats: XWaylandStats,

    /// Start time for uptime tracking
    start_time: Instant,
}

/// Information about an X11 window
#[derive(Debug, Clone)]
pub struct X11WindowInfo {
    /// X11 window ID
    pub window_id: u32,

    /// Window title
    pub title: String,

    /// Window class (application name)
    pub class: String,

    /// Window geometry
    pub geometry: (i32, i32, u32, u32), // x, y, width, height

    /// Whether window is mapped
    pub mapped: bool,

    /// Created timestamp
    pub created_at: Instant,
}

/// XWayland server state
#[derive(Debug, Clone, PartialEq)]
pub enum XWaylandServerState {
    /// Server is stopped
    Stopped,

    /// Server is starting up
    Starting,

    /// Server is running normally
    Running,

    /// Server encountered an error
    Error(String),
}

/// XWayland statistics
#[derive(Debug, Clone, Default)]
pub struct XWaylandStats {
    pub server_restarts: u32,
    pub x11_windows_created: u64,
    pub active_x11_windows: usize,
    pub uptime: Duration,
    pub memory_usage: Option<u64>,
}

impl XWaylandManager {
    /// Create a new XWayland manager
    pub async fn new(config: &XWaylandConfig) -> Result<Self> {
        info!("🔗 Initializing XWayland manager");

        Ok(Self {
            config: config.clone(),
            xwayland_process: None,
            display_number: None,
            x11_windows: HashMap::new(),
            server_state: XWaylandServerState::Stopped,
            stats: XWaylandStats::default(),
            start_time: Instant::now(),
        })
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down XWayland manager");

        // Stop server if running
        self.stop_server().await?;

        // Clear all X11 windows
        self.x11_windows.clear();

        // Unset environment variable
        std::env::remove_var("DISPLAY");

        info!(
            "📊 XWayland final stats: {} windows created, {} server restarts, {:.1}s uptime",
            self.stats.x11_windows_created,
            self.stats.server_restarts,
            self.stats.uptime.as_secs_f32()
        );

        info!("✅ XWayland manager shutdown complete");
        Ok(())
    }

    /// Stop the XWayland server
    pub async fn stop_server(&mut self) -> Result<()> {
        if let Some(mut process) = self.xwayland_process.take() {
            info!("🛑 Stopping XWayland server");

            // Try graceful shutdown first
            if let Err(e) = process.kill().await {
                log::warn!("Failed to kill XWayland process: {}", e);
            }

            // Wait for process to exit
            let _ = process.wait().await;

            self.server_state = XWaylandServerState::Stopped;
            self.display_number = None;

            info!("✅ XWayland server stopped");
        }

        Ok(())
    }
}
