#![allow(dead_code)]
//! XWayland integration for X11 app compatibility
//! Provides seamless integration of X11 applications in Wayland

use crate::config::XWaylandConfig;
use anyhow::Result;
use log::{info, warn};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// XWayland manager for X11 application support
pub struct XWaylandManager {
    /// XWayland configuration
    config: XWaylandConfig,

    /// XWayland server process
    xwayland_process: Option<Child>,

    /// X11 display number
    display_number: Option<u32>,

    /// X11 windows managed by XWayland (metadata placeholder)
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
    /// Create a new XWayland manager (synchronous)
    pub fn new(config: &XWaylandConfig) -> Result<Self> {
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

    /// Start the XWayland server if enabled. Pass the Wayland display name (e.g. "wayland-1").
    pub fn start_server(&mut self, wayland_display: &str) -> Result<()> {
        if !self.config.enabled {
            warn!("XWayland is disabled in config");
            return Ok(());
        }
        if self.xwayland_process.is_some() {
            return Ok(());
        }

        // Select a display number
        let display = if let Some(d) = self.config.display {
            d
        } else {
            find_free_x_display().unwrap_or(1)
        };

        // Spawn Xwayland in rootless mode
        let mut cmd = Command::new("Xwayland");
        cmd.arg(format!(":{}", display))
            .arg("-rootless")
            .arg("-terminate")
            .arg("-nolisten")
            .arg("tcp")
            .env("WAYLAND_DISPLAY", wayland_display)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                self.xwayland_process = Some(child);
                self.display_number = Some(display);
                self.server_state = XWaylandServerState::Running;
                std::env::set_var("DISPLAY", format!(":{}", display));
                info!(
                    "🗔 XWayland started on DISPLAY=:{} (WAYLAND_DISPLAY={})",
                    display, wayland_display
                );
                Ok(())
            }
            Err(e) => {
                self.server_state = XWaylandServerState::Error(format!("spawn failed: {}", e));
                warn!("Failed to start XWayland: {}", e);
                Err(e.into())
            }
        }
    }

    /// Update stats (call periodically if desired)
    pub fn tick(&mut self) {
        self.stats.uptime = self.start_time.elapsed();
    }

    /// Stop the XWayland server
    pub fn stop_server(&mut self) -> Result<()> {
        if let Some(mut child) = self.xwayland_process.take() {
            info!("🛑 Stopping XWayland server");
            let _ = child.kill();
            let _ = child.wait();
            self.server_state = XWaylandServerState::Stopped;
            self.display_number = None;
            info!("✅ XWayland server stopped");
        }
        Ok(())
    }

    /// Graceful shutdown
    pub fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down XWayland manager");
        self.stop_server()?;
        self.x11_windows.clear();
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

fn find_free_x_display() -> Option<u32> {
    // Scan /tmp/.X11-unix for sockets and pick the first unused :N
    let dir = Path::new("/tmp/.X11-unix");
    let mut used = std::collections::BTreeSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(num) = name.strip_prefix('X').and_then(|s| s.parse::<u32>().ok()) {
                    used.insert(num);
                }
            }
        }
    }
    (0..256u32).find(|n| !used.contains(n))
}
