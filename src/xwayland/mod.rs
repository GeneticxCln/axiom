//! XWayland integration for X11 app compatibility
//! Provides seamless integration of X11 applications in Wayland

use crate::config::XWaylandConfig;
use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use std::{
    collections::HashMap,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    process::{Child as TokioChild, Command as TokioCommand},
    time::sleep,
};

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

    /// Start the XWayland server
    pub async fn start_server(&mut self) -> Result<()> {
        if !self.config.enabled {
            info!("🚫 XWayland is disabled in configuration");
            return Ok(());
        }

        info!("🚀 Starting XWayland server");
        self.server_state = XWaylandServerState::Starting;

        // Find available display number
        let display_num = self.find_available_display().await?;
        self.display_number = Some(display_num);

        // Set up environment variables
        let display_name = format!(":{}", display_num);

        // Start XWayland process
        let mut cmd = TokioCommand::new(&self.config.xwayland_path);
        cmd.arg(display_name.clone())
            .arg("-rootless")
            .arg("-terminate")
            .arg("-wm")
            .arg(format!("{}", std::process::id()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Add additional arguments from config
        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        let child = cmd.spawn().context("Failed to spawn XWayland process")?;

        self.xwayland_process = Some(child);

        // Wait for server to be ready
        self.wait_for_server_ready(display_num).await?;

        self.server_state = XWaylandServerState::Running;

        info!(
            "✅ XWayland server started successfully on display {}",
            display_name
        );
        info!(
            "🔧 X11 applications can now connect via DISPLAY={}",
            display_name
        );

        // Set environment variable for child processes
        std::env::set_var("DISPLAY", display_name);

        Ok(())
    }

    /// Find an available X11 display number
    async fn find_available_display(&self) -> Result<u32> {
        for display_num in 0..100 {
            let lock_file = format!("/tmp/.X{}-lock", display_num);
            if !std::path::Path::new(&lock_file).exists() {
                debug!("🔍 Found available display: :{}", display_num);
                return Ok(display_num);
            }
        }

        Err(anyhow::anyhow!("No available X11 display numbers found"))
    }

    /// Wait for XWayland server to be ready
    async fn wait_for_server_ready(&self, display_num: u32) -> Result<()> {
        let socket_path = format!("/tmp/.X11-unix/X{}", display_num);
        let max_attempts = 50;
        let delay = Duration::from_millis(100);

        for attempt in 0..max_attempts {
            if std::path::Path::new(&socket_path).exists() {
                debug!(
                    "✅ XWayland server socket ready after {} attempts",
                    attempt + 1
                );
                return Ok(());
            }

            debug!(
                "⏳ Waiting for XWayland server socket... ({}/{})",
                attempt + 1,
                max_attempts
            );
            sleep(delay).await;
        }

        Err(anyhow::anyhow!(
            "XWayland server failed to start within timeout"
        ))
    }

    /// Handle new X11 window
    pub fn handle_x11_window_created(&mut self, window_id: u32, title: String, class: String) {
        let window_info = X11WindowInfo {
            window_id,
            title: title.clone(),
            class: class.clone(),
            geometry: (0, 0, 800, 600), // Default geometry
            mapped: false,
            created_at: Instant::now(),
        };

        self.x11_windows.insert(window_id, window_info);
        self.stats.x11_windows_created += 1;
        self.stats.active_x11_windows = self.x11_windows.len();

        info!(
            "🪟 New X11 window: {} ({}), ID: {}",
            title, class, window_id
        );
    }

    /// Handle X11 window destroyed
    pub fn handle_x11_window_destroyed(&mut self, window_id: u32) {
        if let Some(window_info) = self.x11_windows.remove(&window_id) {
            self.stats.active_x11_windows = self.x11_windows.len();

            info!(
                "🗑️ X11 window destroyed: {} ({}), ID: {} (lived for {:.1}s)",
                window_info.title,
                window_info.class,
                window_id,
                window_info.created_at.elapsed().as_secs_f32()
            );
        }
    }

    /// Handle X11 window mapped
    pub fn handle_x11_window_mapped(&mut self, window_id: u32) {
        if let Some(window_info) = self.x11_windows.get_mut(&window_id) {
            window_info.mapped = true;
            debug!(
                "👁️ X11 window mapped: {} ({})",
                window_info.title, window_info.class
            );
        }
    }

    /// Handle X11 window unmapped
    pub fn handle_x11_window_unmapped(&mut self, window_id: u32) {
        if let Some(window_info) = self.x11_windows.get_mut(&window_id) {
            window_info.mapped = false;
            debug!(
                "👁️‍🗨️ X11 window unmapped: {} ({})",
                window_info.title, window_info.class
            );
        }
    }

    /// Update X11 window geometry
    pub fn update_x11_window_geometry(
        &mut self,
        window_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) {
        if let Some(window_info) = self.x11_windows.get_mut(&window_id) {
            window_info.geometry = (x, y, width, height);
            debug!(
                "📐 X11 window geometry updated: {} -> {}x{} at ({}, {})",
                window_info.title, width, height, x, y
            );
        }
    }

    /// Get X11 window by ID
    pub fn get_x11_window(&self, window_id: u32) -> Option<&X11WindowInfo> {
        self.x11_windows.get(&window_id)
    }

    /// Get all X11 windows
    pub fn x11_windows(&self) -> impl Iterator<Item = &X11WindowInfo> {
        self.x11_windows.values()
    }

    /// Get mapped X11 windows
    pub fn mapped_x11_windows(&self) -> impl Iterator<Item = &X11WindowInfo> {
        self.x11_windows.values().filter(|w| w.mapped)
    }

    /// Check if XWayland server is running
    pub fn is_server_running(&self) -> bool {
        matches!(self.server_state, XWaylandServerState::Running)
    }

    /// Get server state
    pub fn server_state(&self) -> &XWaylandServerState {
        &self.server_state
    }

    /// Get display number
    pub fn display_number(&self) -> Option<u32> {
        self.display_number
    }

    /// Monitor XWayland process health
    pub async fn monitor_process(&mut self) -> Result<()> {
        if let Some(ref mut process) = self.xwayland_process {
            // Check if process is still running
            match process.try_wait() {
                Ok(Some(status)) => {
                    warn!("⚠️ XWayland process exited with status: {:?}", status);
                    self.server_state =
                        XWaylandServerState::Error(format!("Process exited: {:?}", status));
                    self.xwayland_process = None;

                    // Attempt restart if enabled
                    if self.config.auto_restart {
                        warn!("🔄 Attempting to restart XWayland server...");
                        self.stats.server_restarts += 1;
                        sleep(Duration::from_secs(1)).await;
                        self.start_server().await?;
                    }
                }
                Ok(None) => {
                    // Process is still running
                }
                Err(e) => {
                    error!("❌ Error checking XWayland process status: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Update statistics
    fn update_stats(&mut self) {
        self.stats.uptime = self.start_time.elapsed();
        self.stats.active_x11_windows = self.x11_windows.len();

        // Update memory usage (simplified)
        if let Some(ref _process) = self.xwayland_process {
            // In a real implementation, we would query process memory usage
            self.stats.memory_usage = Some(0); // Placeholder
        }
    }

    /// Get statistics
    pub fn get_stats(&mut self) -> &XWaylandStats {
        self.update_stats();
        &self.stats
    }

    /// Restart XWayland server
    pub async fn restart_server(&mut self) -> Result<()> {
        info!("🔄 Restarting XWayland server");

        // Stop current server
        self.stop_server().await?;

        // Wait a moment
        sleep(Duration::from_secs(1)).await;

        // Start new server
        self.start_server().await?;

        self.stats.server_restarts += 1;
        info!("✅ XWayland server restarted successfully");

        Ok(())
    }

    /// Stop XWayland server
    pub async fn stop_server(&mut self) -> Result<()> {
        if let Some(mut process) = self.xwayland_process.take() {
            info!("🛑 Stopping XWayland server");

            // Try graceful shutdown first
            if let Err(e) = process.kill().await {
                warn!("⚠️ Error killing XWayland process: {}", e);
            }

            // Wait for process to exit
            if let Err(e) = process.wait().await {
                warn!("⚠️ Error waiting for XWayland process: {}", e);
            }

            self.server_state = XWaylandServerState::Stopped;
            info!("✅ XWayland server stopped");
        }

        Ok(())
    }

    /// Shutdown XWayland manager
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
}
