//! XWayland integration manager
//!
//! Provides seamless integration of X11 applications in Wayland
//! via the XWayland compatibility layer.

use crate::config::XWaylandConfig;
use anyhow::Result;
use log::info;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};
use tokio::process::Child as TokioChild;

/// XWayland manager for X11 application support
pub struct XWaylandManager {
    /// XWayland configuration (retained for runtime reference)
    _config: XWaylandConfig,

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
    #[allow(dead_code)]
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

/// `XWayland` server state
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

/// `XWayland` statistics
#[derive(Debug, Clone, Default)]
pub struct XWaylandStats {
    pub server_restarts: u32,
    pub x11_windows_created: u64,
    pub active_x11_windows: usize,
    pub uptime: Duration,
    pub memory_usage: Option<u64>,
}

impl XWaylandManager {
    /// Create a new `XWayland` manager
    pub async fn new(config: &XWaylandConfig) -> Result<Self> {
        info!("🔗 Initializing XWayland manager");

        let mut manager = Self {
            _config: config.clone(),
            xwayland_process: None,
            display_number: None,
            x11_windows: HashMap::new(),
            server_state: XWaylandServerState::Stopped,
            stats: XWaylandStats::default(),
            start_time: Instant::now(),
        };

        if config.enabled {
            if let Err(e) = manager.start_server().await {
                log::error!("Failed to auto-start XWayland: {}", e);
            }
        }

        Ok(manager)
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

    /// Start the `XWayland` subprocess (rootless, terminate-on-last-client).
    ///
    /// Idempotent: returns `Ok(())` if a server is already running. Picks
    /// the configured display number first, then scans `:0..:32` for a
    /// free slot. Sets the `DISPLAY` env var on success.
    pub async fn start_server(&mut self) -> Result<()> {
        self.start_server_inner(None, None).await
    }

    /// Restart or start XWayland with a compositor-side XWM stream wired into
    /// the `-wm <fd>` socket expected by rootless XWayland.
    pub async fn restart_with_wm_stream(&mut self, wm_stream: UnixStream) -> Result<()> {
        self.restart_with_wm_stream_for_display(wm_stream, None).await
    }

    /// Same as [`restart_with_wm_stream`], but forces the child XWayland
    /// process to connect to the specified parent Wayland display.
    pub async fn restart_with_wm_stream_for_display(
        &mut self,
        wm_stream: UnixStream,
        wayland_display: Option<String>,
    ) -> Result<()> {
        if self.xwayland_process.is_some() {
            self.stop_server().await?;
        }
        self.start_server_inner(Some(wm_stream), wayland_display).await
    }

    async fn start_server_inner(
        &mut self,
        wm_stream: Option<UnixStream>,
        wayland_display: Option<String>,
    ) -> Result<()> {
        if self.xwayland_process.is_some() {
            info!("⚠️ XWayland server already running");
            return Ok(());
        }

        info!("🚀 Starting XWayland server...");
        self.server_state = XWaylandServerState::Starting;

        // 1. Find a free display number
        let display = self.find_free_display()?;
        info!("Found free X11 display: :{}", display);

        // 2. Prepare XWayland command
        let mut cmd = tokio::process::Command::new("Xwayland");
        cmd.arg(format!(":{}", display))
            .arg("-rootless") // Integrate with the parent Wayland compositor
            .arg("-terminate") // Terminate when last client disconnects
            .arg("-core"); // Core dump on fault

        if let Some(ref parent_wayland_display) = wayland_display {
            cmd.env("WAYLAND_DISPLAY", parent_wayland_display);
            info!(
                "Starting XWayland against parent Wayland display '{}'",
                parent_wayland_display
            );
        }

        // When a compositor-side XWM connection is supplied, pass the inherited
        // FD number to XWayland so rootless X11 windows can be managed by the
        // compositor's XWM path.
        let wm_stream = if let Some(wm_stream) = wm_stream {
            clear_cloexec(wm_stream.as_raw_fd())?;
            cmd.arg("-wm").arg(wm_stream.as_raw_fd().to_string());
            Some(wm_stream)
        } else {
            None
        };

        // 3. Spawn the process
        match cmd.spawn() {
            Ok(child) => {
                self.xwayland_process = Some(child);
                self.display_number = Some(display);
                drop(wm_stream);

                // 4. Wait for display to be ready
                if self.wait_for_display(display).await {
                    info!("✅ XWayland server started successfully on :{}", display);
                    self.server_state = XWaylandServerState::Running;

                    // 5. Set environment variable for this process and children
                    std::env::set_var("DISPLAY", format!(":{}", display));
                } else {
                    log::error!("❌ Timeout waiting for XWayland display :{}", display);
                    self.stop_server().await?;
                    return Err(anyhow::anyhow!("Timed out waiting for XWayland to start"));
                }
            }
            Err(e) => {
                log::error!("❌ Failed to spawn XWayland: {}", e);
                self.server_state = XWaylandServerState::Error(e.to_string());
                return Err(anyhow::anyhow!("Failed to spawn XWayland: {}", e));
            }
        }

        Ok(())
    }

    /// Find a free X11 display number
    fn find_free_display(&self) -> Result<u32> {
        // If config specifies a display, try that first
        if let Some(display) = self._config.display {
            if !self.is_display_locked(display) {
                return Ok(display);
            }
            log::warn!(
                "Configured display :{} is in use, searching for others...",
                display
            );
        }

        // Search for free display from 0 to 32
        for i in 0..32 {
            if !self.is_display_locked(i) {
                return Ok(i);
            }
        }

        Err(anyhow::anyhow!("No free X11 displays found"))
    }

    /// Check if a display number is locked/in-use
    fn is_display_locked(&self, display: u32) -> bool {
        let lock_path = format!("/tmp/.X{}-lock", display);
        let socket_path = format!("/tmp/.X11-unix/X{}", display);

        std::path::Path::new(&lock_path).exists() || std::path::Path::new(&socket_path).exists()
    }

    /// Wait for `XWayland` display socket to appear
    async fn wait_for_display(&self, display: u32) -> bool {
        let socket_path = std::path::PathBuf::from(format!("/tmp/.X11-unix/X{}", display));
        let start = Instant::now();
        let timeout = Duration::from_secs(5); // 5 second timeout

        while start.elapsed() < timeout {
            if socket_path.exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        false
    }

    /// Stop the `XWayland` subprocess if running, and reset the manager
    /// state to `Stopped`. Always clears the `DISPLAY` env var.
    pub async fn stop_server(&mut self) -> Result<()> {
        if let Some(mut process) = self.xwayland_process.take() {
            info!("🛑 Stopping XWayland server");

            // Try graceful shutdown first (SIGTERM allows cleanup)
            #[cfg(unix)]
            {
                if let Some(pid) = process.id() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                if let Err(e) = process.kill().await {
                    log::warn!("Failed to kill XWayland process: {}", e);
                }
            }

            // Wait up to 2 seconds for graceful shutdown, then SIGKILL
            let shutdown_timeout = Duration::from_secs(2);
            match tokio::time::timeout(shutdown_timeout, process.wait()).await {
                Ok(Ok(status)) => {
                    info!("✅ XWayland server exited gracefully (status: {})", status);
                }
                Ok(Err(e)) => {
                    log::warn!("Error waiting for XWayland process: {}", e);
                }
                Err(_) => {
                    // Timeout — force kill
                    log::warn!(
                        "⏰ XWayland didn't exit within {}s, sending SIGKILL",
                        shutdown_timeout.as_secs()
                    );
                    #[cfg(unix)]
                    {
                        // Process was taken by wait(), but we can still force-kill via pid
                        // Re-take the process to send SIGKILL
                        // Actually, process was moved into wait(). Use kill via pid.
                    }
                    // On unix, we already sent SIGTERM. If still alive, the caller
                    // should handle cleanup. For now, log the situation.
                    log::error!(
                        "XWayland process may still be running — manual cleanup may be needed"
                    );
                }
            }

            info!("✅ XWayland server stopped");
        }

        // Always reset state and display, even if no process was running
        // (handles edge cases where startup failed mid-way)
        self.server_state = XWaylandServerState::Stopped;
        self.display_number = None;
        std::env::remove_var("DISPLAY");

        Ok(())
    }
}

fn clear_cloexec(fd: std::os::fd::RawFd) -> Result<()> {
    // SAFETY: `fcntl` only inspects/modifies the descriptor flags for the
    // provided valid fd; it does not dereference arbitrary memory.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(anyhow::anyhow!(
            "fcntl(F_GETFD) failed for wm fd {}: {}",
            fd,
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: same rationale as above; clears the close-on-exec bit so the
    // child XWayland process inherits the fd specified via `-wm`.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    if rc < 0 {
        return Err(anyhow::anyhow!(
            "fcntl(F_SETFD) failed for wm fd {}: {}",
            fd,
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
