//! Core compositor implementation
//!
//! This module contains the main AxiomCompositor struct and event loop.
//! It coordinates between all subsystems: workspaces, input, etc.
//!
//! Uses Smithay 0.7 for Wayland compositor functionality including
//! window management, surface handling, and protocol support.
//!
//! ## Event loop model
//!
//! Uses a calloop `EventLoop` with two event sources:
//! - A `Signals` source for SIGTERM/SIGINT handling
//! - A `Timer` source for frame pacing (drives `tick()` at the configured FPS)
//!
//! All subsystems are shared via `parking_lot::RwLock`. Window-correlated
//! locks are conventionally taken in the order `workspace` → `window_manager`
//! → `decoration_manager` (see `remove_window`); the reverse is avoided to
//! prevent cross-subsystem inversions.

use anyhow::{Context, Result};
use calloop::signals::{Signal, Signals};
use calloop::timer::{TimeoutAction, Timer};
use calloop::EventLoop;
use log::{debug, info, warn};
use std::time::Duration;

use crate::backend::AxiomSmithayBackendReal;
use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::input::InputManager;
use crate::ipc::{AxiomIPCServer, LazyUIMessage, LiveMetrics};
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;

use std::sync::Arc;

/// Main compositor struct that orchestrates all subsystems
pub struct AxiomCompositor {
    config: AxiomConfig,
    running: bool,
    _windowed: bool,

    // Subsystems
    workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
    window_manager: Arc<parking_lot::RwLock<WindowManager>>,
    input_manager: Arc<parking_lot::RwLock<InputManager>>,
    ipc_server: AxiomIPCServer,
    consecutive_error_count: u32,
    /// When true, the next `tick()` will record an error regardless of
    /// actual subsystem behavior. Used by integration tests to simulate
    /// consecutive errors without requiring real failures.
    force_next_tick_error: bool,

    // Server-side decoration manager for titlebar/button rendering
    decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,

    // Smithay Backend
    smithay_backend: AxiomSmithayBackendReal,
}

impl AxiomCompositor {
    /// Create a new Axiom compositor instance
    pub fn new(
        config: AxiomConfig,
        windowed: bool,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        mut ipc_server: AxiomIPCServer,
    ) -> Result<Self> {
        // Initialize IPC server for Lazy UI integration. Wire the live config
        // handle so `GetConfig` queries resolve against the real config tree
        // rather than the previous hard-coded default placeholder.
        debug!("Initializing IPC server...");
        ipc_server.set_config_handle(Arc::new(parking_lot::RwLock::new(config.clone())));
        ipc_server.start().context("Failed to start IPC server")?;

        info!("All subsystems initialized successfully");

        // Initialize server-side decoration manager (must be created before
        // the Smithay backend so it can receive a clone).
        let minimize_enabled = config.features.enable_minimize;
        let decoration_manager = Arc::new(parking_lot::RwLock::new(DecorationManager::new(
            &config.window,
            minimize_enabled,
        )));

        let smithay_backend = {
            info!("Initializing Axiom compositor with Smithay backend...");
            debug!("Initializing Smithay Wayland backend...");
            let mut backend = AxiomSmithayBackendReal::new(
                config.clone(),
                window_manager.clone(),
                workspace_manager.clone(),
                input_manager.clone(),
                decoration_manager.clone(),
            )?;
            backend
                .initialize()
                .context("Failed to initialize Smithay backend")?;
            backend
        };

        Ok(Self {
            config,
            _windowed: windowed,
            workspace_manager,
            window_manager,
            input_manager,
            ipc_server,
            smithay_backend,
            consecutive_error_count: 0,
            force_next_tick_error: false,
            decoration_manager,
            running: true,
        })
    }

    /// Start the compositor main event loop
    pub fn run(&mut self) -> Result<()> {
        info!("Starting Axiom compositor event loop with calloop");
        self.running = true;

        let mut event_loop = EventLoop::try_new()?;
        let handle = event_loop.handle();

        // LoopSignal to stop the event loop from callbacks
        let loop_signal = event_loop.get_signal();
        let sig_for_signals = loop_signal.clone();
        let sig_for_timer = loop_signal.clone();

        // Signal handling (SIGTERM, SIGINT)
        let signals = Signals::new(&[Signal::SIGTERM, Signal::SIGINT])
            .map_err(|e| anyhow::anyhow!("Failed to create signal source: {}", e))?;
        handle
            .insert_source(
                signals,
                move |_event, _metadata, compositor: &mut AxiomCompositor| {
                    info!("Received signal, shutting down gracefully");
                    let _ = compositor.shutdown();
                    sig_for_signals.stop();
                },
            )
            .map_err(|e| anyhow::anyhow!("Failed to insert signal source: {}", e))?;

        // Frame pacing timer — fires every `interval` and calls tick()
        let interval = if self.config.general.max_fps == 0 {
            Duration::from_millis(16) // unbounded → default ~60fps
        } else {
            let clamped = self.config.general.max_fps.clamp(1, 1000);
            Duration::from_secs_f64(1.0 / f64::from(clamped))
        };
        let timer = Timer::from_duration(interval);
        handle
            .insert_source(
                timer,
                move |_event, _metadata, compositor: &mut AxiomCompositor| {
                    if compositor.running {
                        if compositor.tick().is_err() {
                            // tick returned error (threshold exceeded) — stop
                            compositor.running = false;
                            sig_for_timer.stop();
                            return TimeoutAction::Drop;
                        }
                        // Re-arm timer for next frame
                        TimeoutAction::ToDuration(interval)
                    } else {
                        sig_for_timer.stop();
                        TimeoutAction::Drop
                    }
                },
            )
            .map_err(|e| anyhow::anyhow!("Failed to insert timer source: {}", e))?;

        // Run the event loop — dispatches events, calls timer and signal callbacks
        event_loop.run(None, &mut *self, |_| {})?;

        info!("Axiom compositor event loop finished");
        Ok(())
    }

    /// Process all pending compositor events with real input handling
    fn process_events(&mut self) -> Result<()> {
        // Process backend events (Wayland, input devices)
        self.smithay_backend.process_events()?;

        // Poll IPC server: accept connections, read/write, idle timeout
        self.ipc_server.poll();

        // Process IPC messages from Lazy UI.
        match self.ipc_server.process_messages(&mut self.config) {
            Ok((config_changed, pending_actions)) => {
                if config_changed {
                    self.update_subsystems_config();
                    // Refresh the IPC server's config handle so `GetConfig`
                    // queries see the same values the compositor just
                    // applied through `process_messages`. Without this the
                    // handle remains frozen at its `new()`-time clone and
                    // returns stale data when Lazy UI re-queries. Matches
                    // the project's push-based config propagation model
                    // (see `update_subsystems_config`).
                    self.ipc_server
                        .set_config_handle(Arc::new(parking_lot::RwLock::new(self.config.clone())));
                }
                for action in pending_actions {
                    match action {
                        LazyUIMessage::WorkspaceCommand { action, parameters } => {
                            self.dispatch_workspace_command(&action, &parameters);
                        }
                        LazyUIMessage::SetClipboard { text } => {
                            self.set_clipboard(text);
                        }
                        LazyUIMessage::SetWindowBlur { window_id, radius } => {
                            debug!("Set blur radius {} for window {}", radius, window_id);
                            self.smithay_backend.state.needs_redraw = true;
                        }
                        _ => {
                            warn!("Unexpected pending action variant from IPC queue");
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Error processing IPC messages: {}", e);
            }
        }

        Ok(())
    }

    /// Apply a validated `LazyUIMessage::WorkspaceCommand { action, parameters }`
    /// to the live workspace / window subsystems. Each high-level composer
    /// method (`scroll_workspace_left`, `move_window_left`, …) takes and
    /// drops its own lock internally, so calling them in sequence avoids
    /// any cross-subsystem inversion.
    fn dispatch_workspace_command(&mut self, action: &str, parameters: &serde_json::Value) {
        match action {
            "scroll_left" => self.scroll_workspace_left(),
            "scroll_right" => self.scroll_workspace_right(),
            "add_window" => {
                // A window created via IPC has no backing Wayland surface, so it
                // would be a phantom that never renders — a footgun for any local
                // IPC client. Windows are created by real Wayland clients, not by
                // IPC, so ignore this request. (The `add_window` method itself
                // remains available for direct/test use.)
                warn!("WorkspaceCommand::add_window ignored: IPC cannot create a window with a real Wayland surface");
            }
            "remove_window" => match parameters.get("window_id").and_then(|v| v.as_u64()) {
                Some(id) => {
                    self.remove_window(id);
                }
                None => {
                    warn!("WorkspaceCommand remove_window missing 'window_id' parameter — no-op")
                }
            },
            "move_focus_left" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => self.move_window_left(id),
                    None => debug!("WorkspaceCommand move_focus_left: no focused window, no-op"),
                }
            }
            "move_focus_right" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => self.move_window_right(id),
                    None => {
                        debug!("WorkspaceCommand move_focus_right: no focused window, no-op")
                    }
                }
            }
            "toggle_floating" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => {
                        self.window_manager.write().toggle_floating(id);
                    }
                    None => debug!("WorkspaceCommand toggle_floating: no focused window, no-op"),
                }
                self.smithay_backend.state.needs_redraw = true;
            }
            "minimize_window" => match parameters.get("window_id").and_then(|v| v.as_u64()) {
                Some(id) => {
                    let _ = self.minimize_window(id);
                }
                None => {
                    warn!("WorkspaceCommand minimize_window missing 'window_id' parameter — no-op")
                }
            },
            "restore_window" => match parameters.get("window_id").and_then(|v| v.as_u64()) {
                Some(id) => {
                    let _ = self.restore_window(id);
                }
                None => {
                    warn!("WorkspaceCommand restore_window missing 'window_id' parameter — no-op")
                }
            },
            "toggle_fullscreen" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => {
                        self.toggle_fullscreen(id);
                    }
                    None => {
                        debug!("WorkspaceCommand toggle_fullscreen: no focused window, no-op")
                    }
                }
            }
            // Defensive catch-all. The IPC layer's whitelist already rejects
            // unknown actions, so reaching here means a future handler or
            // schema change introduced a mismatch — surface it loudly.
            unknown => warn!(
                "WorkspaceCommand '{}' reached dispatch despite whitelist validation",
                unknown
            ),
        }
    }

    /// Post-render phase: placeholder for monitoring.
    fn render_frame(&mut self) -> Result<()> {
        debug!(
            "Frame rendered - position: {:.1}, column: {}",
            self.workspace_manager.read().current_position(),
            self.workspace_manager.read().focused_column_index(),
        );

        Ok(())
    }

    /// Gracefully shutdown the compositor (with Smithay backend)
    fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down Axiom compositor...");

        self.running = false;

        // Broadcast shutdown state change before backend teardown so
        // IPC clients can react before the broadcast channel closes.
        self
            .ipc_server
            .broadcast_state_change("compositor", "running", "shutdown");

        // Clean up Smithay backend
        debug!("Shutting down Smithay backend...");
        self.smithay_backend.shutdown()?;

        // Clean up other subsystems
        debug!("Cleaning up compositor subsystems...");
        // IPC server shutdown is sync for now (Phase 2 will fully migrate IPC)
        self.ipc_server.shutdown_sync();
        self.input_manager.write().shutdown();
        self.workspace_manager.write().shutdown();
        self.window_manager.write().shutdown();

        info!("Axiom compositor shutdown complete");
        Ok(())
    }

    /// Get current configuration
    pub fn config(&self) -> &AxiomConfig {
        &self.config
    }

    /// Check if compositor is running in windowed mode
    pub fn is_windowed(&self) -> bool {
        self._windowed
    }

    /// Get the Wayland display socket name.
    pub fn socket_name(&self) -> &str {
        &self.smithay_backend.socket_name
    }

    /// Single tick of the compositor (event processing + rendering).
    ///
    /// Frame pacing is handled by the calloop timer in `run()`, so this
    /// method is purely synchronous: process events, render, update metrics.
    fn tick(&mut self) -> Result<()> {
        use std::time::Instant;
        let frame_start = Instant::now();
        let mut tick_error = false;

        // Process events (calls backend.process_events → run_one_cycle → render)
        if self.force_next_tick_error {
            tick_error = true;
            self.force_next_tick_error = false;
        }
        if let Err(e) = self.process_events() {
            tick_error = true;
            warn!("Error processing events: {}", e);
        }

        // Render frame — post-render monitoring.
        if let Err(e) = self.render_frame() {
            tick_error = true;
            warn!("Error rendering frame: {}", e);
        }

        // Update stability metrics
        if tick_error {
            self.consecutive_error_count += 1;
            warn!("Consecutive error count: {}", self.consecutive_error_count);
        } else if self.consecutive_error_count > 0 {
            self.consecutive_error_count = self.consecutive_error_count.saturating_sub(1);
        }

        // Broadcast IPC performance metrics and refresh snapshot
        let (frame_time_ms, active_windows, workspace_idx) = {
            let frame_time_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
            let (workspace_idx, _, _column_count, _) = self.get_workspace_info();
            let active_windows = self.window_manager.read().window_count();
            (frame_time_ms, active_windows, workspace_idx)
        };
        self.ipc_server.maybe_broadcast_performance_metrics(
            frame_time_ms,
            active_windows,
            workspace_idx,
        );
        self.ipc_server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms,
            active_windows,
            current_workspace: workspace_idx,
        });

        // Check stability threshold
        if self.consecutive_error_count >= 5 {
            log::error!(
                "CRITICAL: Too many consecutive errors ({}). Initiating emergency shutdown.",
                self.consecutive_error_count
            );
            let _ = self.shutdown();
            return Err(anyhow::anyhow!(
                "Critical stability failure: too many consecutive errors"
            ));
        }

        Ok(())
    }

    // === Public Workspace Interaction Methods ===

    /// Scroll workspace left (for input handling)
    pub fn scroll_workspace_left(&mut self) {
        info!("Scrolling workspace left");
        let mut wm = self.workspace_manager.write();
        let old_idx = wm.focused_column_index();
        wm.scroll_left();
        let new_idx = wm.focused_column_index();
        drop(wm);
        self.smithay_backend.state.needs_redraw = true;
        self.ipc_server.broadcast_state_change(
            "workspace",
            &old_idx.to_string(),
            &new_idx.to_string(),
        );
    }

    /// Scroll workspace right (for input handling)
    pub fn scroll_workspace_right(&mut self) {
        info!("Scrolling workspace right");
        let mut wm = self.workspace_manager.write();
        let old_idx = wm.focused_column_index();
        wm.scroll_right();
        let new_idx = wm.focused_column_index();
        drop(wm);
        self.smithay_backend.state.needs_redraw = true;
        self.ipc_server.broadcast_state_change(
            "workspace",
            &old_idx.to_string(),
            &new_idx.to_string(),
        );
    }

    /// Add a new window to the current workspace.
    /// Also registers the window with the server-side decoration manager so
    /// titlebar buttons are positioned from real geometry (not a placeholder).
    pub fn add_window(&mut self, title: String) -> u64 {
        // Create window in window manager (default size: 800x600)
        let window_id = self.window_manager.write().add_window(title.clone());
        // ponytail: this is a phantom window — no Wayland surface and no
        // window_map entry, reachable only via IPC for tests/debug. It will
        // never be rendered.
        log::warn!(
            "add_window: created window with no backing Wayland surface; it will not be rendered"
        );

        // Add to current workspace column
        self.workspace_manager.write().add_window(window_id);

        // Register with DecorationManager using real window geometry.
        // The default BackendWindow size is 800×600; callers can update
        // via `set_window_width` after a Wayland configure arrives.
        self.decoration_manager.write().add_window(
            window_id,
            title.clone(),
            /* prefers_server_side */ true,
            800, // default BackendWindow width
        );

        self.smithay_backend.state.needs_redraw = true;
        info!(
            "Added window '{}' (ID: {}) to current workspace",
            title, window_id
        );
        self.ipc_server.broadcast_state_change(
            "window",
            "none",
            &format!("added:{}", window_id),
        );
        window_id
    }

    /// Remove a window from the compositor.
    ///
    /// Returns `true` if the window existed (in workspace manager) and was
    /// removed from all subsystems, `false` if the ID was not found.
    pub fn remove_window(&mut self, window_id: u64) -> bool {
        let removed_from_workspace = self
            .workspace_manager
            .write()
            .remove_window(window_id)
            .is_some();
        let removed_from_windows = self
            .window_manager
            .write()
            .remove_window(window_id)
            .is_some();

        let removed = removed_from_workspace || removed_from_windows;

        if removed {
            self.smithay_backend.state.needs_redraw = true;
            info!("Removed window {}", window_id);
            self.ipc_server.broadcast_state_change(
                "window",
                &format!("active:{}", window_id),
                "none",
            );
        }

        self.decoration_manager.write().remove_window(window_id);

        removed
    }

    /// Move window to left workspace
    pub fn move_window_left(&mut self, window_id: u64) {
        if self.workspace_manager.write().move_window_left(window_id) {
            self.smithay_backend.state.needs_redraw = true;
            info!("Moved window {} to left workspace", window_id);
            self.ipc_server.broadcast_state_change(
                "window",
                &format!("workspace:{}", window_id),
                "left",
            );
        }
    }

    /// Move window to right workspace
    pub fn move_window_right(&mut self, window_id: u64) {
        if self.workspace_manager.write().move_window_right(window_id) {
            self.smithay_backend.state.needs_redraw = true;
            info!("Moved window {} to right workspace", window_id);
            self.ipc_server.broadcast_state_change(
                "window",
                &format!("workspace:{}", window_id),
                "right",
            );
        }
    }

    /// Minimize a window (remove from workspace layout and mark as iconified).
    /// Returns `true` if the window was found and minimized, `false` if the
    /// window ID did not exist or was already minimized.
    #[must_use]
    pub fn minimize_window(&mut self, window_id: u64) -> bool {
        let workspace_ok = self.workspace_manager.write().minimize_window(window_id);
        let wm_ok = self.window_manager.write().minimize_window(window_id);
        if workspace_ok || wm_ok {
            self.smithay_backend.state.needs_redraw = true;
            info!("Minimized window {}", window_id);
            self.ipc_server.broadcast_state_change(
                "window",
                &format!("active:{}", window_id),
                "minimized",
            );
        }
        workspace_ok || wm_ok
    }

    /// Restore a previously minimized window back to its originating column.
    /// Returns `true` if the window was found and restored.
    #[must_use]
    pub fn restore_window(&mut self, window_id: u64) -> bool {
        let workspace_ok = self.workspace_manager.write().restore_window(window_id);
        let wm_ok = self.window_manager.write().restore_window(window_id);
        if workspace_ok || wm_ok {
            self.smithay_backend.state.needs_redraw = true;
            info!("Restored window {}", window_id);
            self.ipc_server.broadcast_state_change(
                "window",
                "minimized",
                &format!("active:{}", window_id),
            );
        }
        workspace_ok || wm_ok
    }

    /// Toggle fullscreen on a window.
    pub fn toggle_fullscreen(&mut self, window_id: u64) {
        self.window_manager.write().toggle_fullscreen(window_id);
        self.smithay_backend.state.needs_redraw = true;
        info!("Toggled fullscreen for window {}", window_id);
        self.ipc_server.broadcast_state_change(
            "window",
            &format!("active:{}", window_id),
            "fullscreen_toggle",
        );
    }

    /// Get current workspace information
    pub fn get_workspace_info(&self) -> (i32, f64, usize, bool) {
        let wm = self.workspace_manager.read();
        (
            wm.focused_column_index(),
            wm.current_position(),
            wm.active_column_count(),
            wm.is_scrolling(),
        )
    }

    /// Set the viewport size (called when display size changes)
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.workspace_manager
            .write()
            .set_viewport_size(width as f64, height as f64);
        info!("Updated viewport size to {}x{}", width, height);
        self.smithay_backend.state.needs_redraw = true;
    }

    /// Single tick for integration testing — calls the private `tick()` method.
    /// Returns `Ok(())` on success or `Err(...)` if the error threshold is exceeded.
    pub fn tick_for_test(&mut self) -> Result<()> {
        self.tick()
    }

    /// Artificially set the consecutive error count for testing error recovery.
    /// When set >= 5, the next `tick()` will trigger an emergency shutdown.
    pub fn set_errors_for_test(&mut self, count: u32) {
        self.consecutive_error_count = count;
    }

    /// Force the next `tick()` to count as an error tick, incrementing the
    /// error count. This lets tests simulate real consecutive errors rather
    /// than just pre-setting the count. Resets after the next tick.
    pub fn force_next_tick_error(&mut self) {
        self.force_next_tick_error = true;
    }

    /// Check whether the compositor is still running.
    /// Used by integration tests to verify shutdown behavior.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Propagate configuration changes to all subsystems
    fn update_subsystems_config(&mut self) {
        info!("Propagating configuration changes to subsystems...");

        // Update Workspace Manager
        self.workspace_manager
            .write()
            .update_config(self.config.workspace.clone());

        self.smithay_backend.state.needs_redraw = true;

        // Future: Update Input Manager, etc.
    }
}

impl AxiomCompositor {
    /// Set the compositor clipboard content from IPC command.
    fn set_clipboard(&mut self, text: String) {
        self.smithay_backend.set_clipboard_data(text.into_bytes());
    }

    /// Get a sender for injecting IPC commands in tests.
    pub fn ipc_command_sender(&self) -> std::sync::mpsc::Sender<LazyUIMessage> {
        self.ipc_server.command_sender_for_test()
    }

    /// Test/debug accessor — see `AxiomSmithayBackendReal::debug_clipboard_cache`.
    pub fn debug_clipboard_cache(&self) -> Option<Vec<u8>> {
        self.smithay_backend.debug_clipboard_cache()
    }

    /// Test/debug helper — see backend `debug_focus_first_client_for_test`.
    pub fn debug_focus_first_client_for_test(&mut self) {
        self.smithay_backend.debug_focus_first_client_for_test();
    }

    /// Test-only constructor that skips real backend initialization.
    /// Subsystems are fully initialized. Smithay backend uses a test
    /// constructor that doesn't bind Wayland sockets.
    pub fn new_for_test(
        config: AxiomConfig,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
    ) -> Result<Self> {
        // Dummy IPC server (skip socket bind)
        let ipc_server = AxiomIPCServer::new();

        // Initialize server-side decoration manager for tests
        let minimize_enabled = config.features.enable_minimize;
        let decoration_manager = Arc::new(parking_lot::RwLock::new(DecorationManager::new(
            &config.window,
            minimize_enabled,
        )));

        // Test Smithay backend (no socket bind, no GPU init)
        let smithay_backend = AxiomSmithayBackendReal::new_for_test(
            config.clone(),
            window_manager.clone(),
            workspace_manager.clone(),
            input_manager.clone(),
            decoration_manager.clone(),
        )?;

        Ok(Self {
            config,
            _windowed: false,
            workspace_manager,
            window_manager,
            input_manager,
            ipc_server,
            smithay_backend,
            consecutive_error_count: 0,
            force_next_tick_error: false,
            decoration_manager,
            running: true, // Test compositor starts in running state
        })
    }
}

// Phase 1.A4: any rename of `state` / `winit_backend` /
// `winit_event_loop` fails the build. Order is locked structurally by
// Rust's drop semantics + the SAFETY comment at
// `backend/mod.rs::AxiomSmithayBackendReal::initialize_winit`. Lives at
// file scope (not inside `#[cfg(test)] mod tests`) so the assertion
// fires on every `cargo build` invocation, not just `cargo test`.
#[allow(dead_code)]
const _: () = {
    static_assertions::assert_fields!(
        crate::backend::AxiomSmithayBackendReal: state, winit_backend, winit_event_loop
    );
};

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
mod tests {
    use super::*;
    use parking_lot::RwLock;
    use serial_test::serial;
    use std::sync::Arc;

    /// Create subsystems and a test compositor for unit testing public API methods.
    ///
    /// `Arc<parking_lot::RwLock<ScrollableWorkspaces>>` is flagged as
    /// non-`Sync` by clippy because `ScrollableWorkspaces` contains a
    /// `RefCell` for its layout cache. This is intentional (single-threaded
    /// interior mutability on the hot path) and the `Arc` here is only
    /// ever held within this test's task — it never crosses thread
    /// boundaries, so the absence of `Sync` is harmless for tests.
    #[allow(clippy::arc_with_non_send_sync)]
    fn make_test_compositor() -> AxiomCompositor {
        let config = AxiomConfig::default();
        let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
        let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
        let input_manager = Arc::new(RwLock::new(InputManager::new(
            &config.input,
            &config.bindings,
        )));

        AxiomCompositor::new_for_test(config, workspace_manager, window_manager, input_manager)
            .expect("compositor init")
    }

    #[test]
    #[serial]
    fn test_compositor_initialization() {
        let comp = make_test_compositor();
        assert!(!comp.is_windowed());
        // DecorationManager should be initialized
        assert!(comp.decoration_manager.read().get_decoration(1).is_none());
    }

    #[test]
    #[serial]
    fn test_add_and_remove_window() {
        let mut comp = make_test_compositor();

        let id = comp.add_window("Test Window".into());
        assert_eq!(id, 1);

        // Window should be registered with DecorationManager using real geometry
        {
            let deco = comp.decoration_manager.read();
            let d = deco
                .get_decoration(id)
                .expect("decoration should exist after add_window");
            assert_eq!(d.title, "Test Window");
            assert_eq!(
                d.window_width, 800,
                "should use default BackendWindow width"
            );
        }

        let (column, _pos, _count, _scrolling) = comp.get_workspace_info();
        assert!(column >= 0);

        comp.remove_window(id);
        // Window should be removed from DecorationManager too
        assert!(comp.decoration_manager.read().get_decoration(id).is_none());
    }

    #[test]
    #[serial]
    fn test_workspace_scrolling() {
        let mut comp = make_test_compositor();

        let _initial = comp.get_workspace_info();
        // Verify scrolling doesn't panic
        comp.scroll_workspace_right();
        let _after_right = comp.get_workspace_info();
        comp.scroll_workspace_left();
        let _after_left = comp.get_workspace_info();
    }

    #[test]
    #[serial]
    fn test_viewport_resize() {
        let mut comp = make_test_compositor();

        comp.set_viewport_size(1920, 1080);
        comp.set_viewport_size(3840, 2160);
        // No panic = success
    }

    #[test]
    #[serial]
    fn test_window_movement_between_workspaces() {
        let mut comp = make_test_compositor();

        let id = comp.add_window("movable".into());
        comp.move_window_right(id);
        comp.move_window_left(id);
        comp.remove_window(id);
    }

    #[test]
    #[serial]
    fn test_config_access() {
        let comp = make_test_compositor();
        let config = comp.config();
        assert!(config.workspace.scroll_speed > 0.0);
        assert!(!config.window.focus_follows_mouse);
    }

    #[test]
    #[serial]
    fn test_multiple_windows() {
        let mut comp = make_test_compositor();

        let ids: Vec<u64> = (0..10)
            .map(|i| comp.add_window(format!("Window {}", i)))
            .collect();

        assert_eq!(ids.len(), 10);
        assert!(ids.windows(2).all(|w| w[0] + 1 == w[1]));

        for id in ids {
            comp.remove_window(id);
        }
    }

    #[test]
    #[serial]
    fn test_shutdown_cleans_up() {
        let mut comp = make_test_compositor();
        comp.add_window("pre-shutdown".into());
        comp.shutdown().expect("shutdown should succeed");
    }

    // ─── Phase 1 migration regression test ────────────────────────────

    /// Verify that tick() runs without error and the compositor stays
    /// running after multiple ticks.
    #[test]
    #[serial]
    fn test_tick_runs_without_error() {
        let mut comp = make_test_compositor();
        comp.config.general.max_fps = 30;
        for _ in 0..8 {
            assert!(comp.tick_for_test().is_ok());
        }
        assert!(comp.is_running());
    }
}
