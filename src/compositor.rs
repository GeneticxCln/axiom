//! Core compositor implementation
//!
//! This module contains the main AxiomCompositor struct and event loop.
//! It coordinates between all subsystems: workspaces, effects, input, etc.
//!
//! This implementation can optionally use Smithay for proper Wayland compositor functionality
//! with window management, surface handling, and protocol support when the
//! `experimental-smithay` feature is enabled.

use anyhow::{Context, Result};
use log::{debug, info, warn};
use tokio::signal;

use crate::backend::AxiomSmithayBackendReal;
use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::ipc::AxiomIPCServer;
use crate::renderer::AxiomRenderer;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use crate::xwayland::XWaylandManager;

use std::sync::Arc;
use tokio::sync::RwLock as AsyncRwLock;

/// Main compositor struct that orchestrates all subsystems
pub struct AxiomCompositor {
    config: AxiomConfig,
    running: bool,
    windowed: bool,

    // Subsystems
    workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
    effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
    window_manager: Arc<parking_lot::RwLock<WindowManager>>,
    decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
    input_manager: Arc<parking_lot::RwLock<InputManager>>,
    xwayland_manager: Option<Arc<AsyncRwLock<XWaylandManager>>>,
    ipc_server: AxiomIPCServer,
    consecutive_error_count: u32,

    // Renderer
    renderer: Arc<parking_lot::RwLock<AxiomRenderer>>,

    // Experimental Smithay Backend
    smithay_backend: AxiomSmithayBackendReal,

    // Performance optimization: Persistent buffers for rendering
    // Avoids re-allocating Vec per frame
    render_data_buffer: Vec<WindowRenderData>,
}

// Data structure for render pass (outside impl to be accessible)
struct WindowRenderData {
    id: u64,
    layout_rect: crate::window::Rectangle,
    opacity: f32,
}

impl AxiomCompositor {
    /// Create a new Axiom compositor instance
    /// Create a new Axiom compositor instance
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: AxiomConfig,
        windowed: bool,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        xwayland_manager: Option<Arc<AsyncRwLock<XWaylandManager>>>,
        mut ipc_server: AxiomIPCServer,
        renderer: Arc<parking_lot::RwLock<AxiomRenderer>>,
    ) -> Result<Self> {
        // Initialize IPC server for Lazy UI integration
        debug!("🔗 Initializing IPC server...");
        ipc_server
            .start()
            .await
            .context("Failed to start IPC server")?;

        let smithay_backend = {
            info!("🏗️ Initializing Axiom compositor with Smithay backend...");
            debug!("🚀 Initializing Smithay Wayland backend...");
            let mut backend = AxiomSmithayBackendReal::new(
                config.clone(),
                window_manager.clone(),
                workspace_manager.clone(),
                effects_engine.clone(),
                input_manager.clone(),
                renderer.clone(),
            )?;
            backend
                .initialize()
                .context("Failed to initialize Smithay backend")?;
            backend
        };

        info!("✅ All subsystems initialized successfully");

        Ok(Self {
            config,
            windowed,
            workspace_manager,
            effects_engine,
            window_manager,
            decoration_manager,
            input_manager,
            xwayland_manager,
            ipc_server,
            smithay_backend,
            render_data_buffer: Vec::with_capacity(64), // Pre-allocate for typical window count
            consecutive_error_count: 0,
            renderer,
            running: false,
        })
    }

    /// Start the compositor main event loop
    pub async fn run(mut self) -> Result<()> {
        info!("🎬 Starting Axiom compositor event loop");

        self.running = true;

        // Set up signal handling
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())?;
        let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())?;

        // Main event loop
        while self.running {
            tokio::select! {
                // Handle system signals
                _ = sigterm.recv() => {
                    info!("📨 Received SIGTERM, shutting down gracefully");
                    self.shutdown().await?;
                }
                _ = sigint.recv() => {
                    info!("📨 Received SIGINT (Ctrl+C), shutting down gracefully");
                    self.shutdown().await?;
                }

                // Combined event processing and rendering
                _ = self.tick() => {}
            }
        }

        info!("🛑 Axiom compositor event loop finished");
        Ok(())
    }

    /// Phase 3: Process all pending compositor events with real input handling
    /// Process all pending compositor events with real input handling
    async fn process_events(&mut self) -> Result<()> {
        // Process backend events (Wayland, input devices)
        self.smithay_backend.process_events().await?;

        // Process IPC messages from Lazy UI
        match self.ipc_server.process_messages(&mut self.config).await {
            Ok(config_changed) => {
                if config_changed {
                    self.update_subsystems_config();
                }
            }
            Err(e) => {
                warn!("⚠️ Error processing IPC messages: {}", e);
            }
        }

        // Phase 3: Simulate input processing for demonstration
        // In a real implementation, this would receive events from Smithay
        self.process_simulated_input_events().await?;

        Ok(())
    }

    /// Phase 3: Simulate input events for testing (until real Smithay integration)
    async fn process_simulated_input_events(&mut self) -> Result<()> {
        // This is a placeholder that simulates occasional input events
        // for testing purposes. Real implementation would receive these from Smithay.

        use crate::input::InputEvent;

        // Simulate a scroll event occasionally (for demo purposes)
        if rand::random::<f32>() < 0.001 {
            // Very low probability
            let event = InputEvent::Scroll {
                x: 100.0,
                y: 100.0,
                delta_x: if rand::random::<bool>() { 10.0 } else { -10.0 },
                delta_y: 0.0,
            };

            let actions = self.input_manager.write().process_input_event(event);
            for action in actions {
                self.handle_compositor_action(action).await?;
            }
        }

        Ok(())
    }

    /// Phase 3: Handle compositor actions triggered by input events
    async fn handle_compositor_action(
        &mut self,
        action: crate::input::CompositorAction,
    ) -> Result<()> {
        use crate::input::CompositorAction;

        match action {
            CompositorAction::ScrollWorkspaceLeft => {
                debug!("🎨 Input triggered: Scroll workspace left");
                self.scroll_workspace_left();
            }
            CompositorAction::ScrollWorkspaceRight => {
                debug!("🎨 Input triggered: Scroll workspace right");
                self.scroll_workspace_right();
            }
            CompositorAction::MoveWindowLeft => {
                debug!("🎨 Input triggered: Move window left");
                if let Some((_window_id, _, _, _)) = self.get_workspace_info().into() {
                    // Get first window in current workspace for demo
                    // We need to lock workspace manager
                    let windows = self.workspace_manager.read().get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.move_window_left(window_id);
                    }
                }
            }
            CompositorAction::MoveWindowRight => {
                debug!("🎨 Input triggered: Move window right");
                if let Some((_window_id, _, _, _)) = self.get_workspace_info().into() {
                    let windows = self.workspace_manager.read().get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.move_window_right(window_id);
                    }
                }
            }
            CompositorAction::CloseWindow => {
                debug!("🎨 Input triggered: Close window");
                // Need window manager lock
                let focused_id = self.window_manager.read().focused_window_id();

                if let Some(focused_id) = focused_id {
                    self.remove_window(focused_id);
                    self.effects_engine.write().animate_window_close(focused_id);
                }
            }
            CompositorAction::ToggleFullscreen => {
                debug!("🎨 Input triggered: Toggle fullscreen");
                let focused_id = self.window_manager.read().focused_window_id();
                if let Some(focused_id) = focused_id {
                    let _ = self.window_manager.write().toggle_fullscreen(focused_id);
                }
            }
            CompositorAction::Quit => {
                info!("💼 Input triggered: Quit compositor");
                self.shutdown().await?;
            }
            CompositorAction::Custom(command) => {
                debug!("🎨 Input triggered custom command: {}", command);
                // TODO: Handle custom commands
            }
        }

        Ok(())
    }

    /// Phase 4: Enhanced frame rendering with visual effects
    #[allow(clippy::unused_async)]
    async fn render_frame(&mut self) -> Result<()> {
        // 1. Update workspace positions/animations
        self.workspace_manager.write().update_animations()?;

        // 2. Update visual effects (animations, blur, shadows)
        self.effects_engine.write().update()?;

        // 3. Calculate workspace layouts for all visible windows
        let workspace_layouts = self.workspace_manager.read().calculate_workspace_layouts();

        // 4. Update window positions and collect render data
        // Split into two passes to avoid holding WindowManager lock while calling other subsystems

        // Clear previous frame data but keep capacity
        self.render_data_buffer.clear();

        {
            let mut wm = self.window_manager.write();
            for (window_id, layout_rect) in workspace_layouts {
                if let Some(window) = wm.get_window_mut(window_id) {
                    // Check if window position changed (for move animations)
                    let old_pos = window.window.position;
                    let new_pos = (layout_rect.x, layout_rect.y);

                    if old_pos != new_pos {
                        // Trigger move animation
                        // Note: We still hold WM lock here, but triggering animation is usually fast
                        // and doesn't lock WM recursively.
                        // Ideally we'd queue this too, but for now let's keep it simple.
                        self.effects_engine.write().animate_window_move(
                            window_id,
                            (old_pos.0 as f32, old_pos.1 as f32),
                            (new_pos.0 as f32, new_pos.1 as f32),
                        );
                    }

                    // Update the backend window position and size
                    window.window.set_position(layout_rect.x, layout_rect.y);
                    window
                        .window
                        .set_size(layout_rect.width, layout_rect.height);

                    self.render_data_buffer.push(WindowRenderData {
                        id: window_id,
                        layout_rect: layout_rect.clone(),
                        opacity: window.properties.opacity,
                    });
                }
            }
        } // Drop WM lock

        // 5. Apply effects and push to renderer (without holding WM lock)
        for win_data in &self.render_data_buffer {
            // Determine render-time properties
            let mut scale = 1.0_f32;
            let mut opacity = win_data.opacity;
            let mut offset = (0.0_f32, 0.0_f32);

            {
                let effects = self.effects_engine.read();
                if let Some(effect_state) = effects.get_window_effects(win_data.id) {
                    scale = effect_state.scale;
                    opacity = effect_state.opacity;
                    offset = effect_state.position_offset;
                }
            } // Drop effects lock

            // Feed to renderer: apply scale and offset
            // Safely use write() instead of try_write() now that we don't hold other locks
            {
                let mut renderer = self.renderer.write();
                let x = win_data.layout_rect.x as f32 + offset.0;
                let y = win_data.layout_rect.y as f32 + offset.1;
                let w = win_data.layout_rect.width as f32 * scale;
                let h = win_data.layout_rect.height as f32 * scale;
                renderer.upsert_window_rect(win_data.id, (x, y), (w, h), opacity);
            }
        }

        // Render with headless renderer for now
        if let Some(mut renderer) = self.renderer.try_write() {
            if let Err(e) = renderer.render() {
                warn!("⚠️ Renderer error: {}", e);
            }
        }

        // 5. Apply global effects (workspace transitions, blur backgrounds)
        self.apply_global_effects();

        // 6. Performance monitoring for effects
        let (frame_time, effects_quality, active_effects) =
            self.effects_engine.read().get_performance_stats();
        if frame_time.as_millis() > 20 {
            // More than ~50 FPS
            debug!(
                "⚡ Frame time: {:.1}ms, effects quality: {:.1}, active effects: {}",
                frame_time.as_secs_f64() * 1000.0,
                effects_quality,
                active_effects
            );
        }

        debug!(
            "🎨 Frame rendered - position: {:.1}, column: {}, effects: {}",
            self.workspace_manager.read().current_position(),
            self.workspace_manager.read().focused_column_index(),
            active_effects
        );

        Ok(())
    }

    /// Apply global visual effects like workspace transitions and background blur
    fn apply_global_effects(&mut self) {
        // Apply workspace transition effects
        let wm = self.workspace_manager.read();
        if wm.is_scrolling() {
            let current_pos = wm.current_position();
            let progress = wm.scroll_progress();

            // In a real implementation, this would apply visual effects to the entire compositor
            debug!(
                "🌊 Workspace transition: position={:.1}, progress={:.2}",
                current_pos, progress
            );
        }
    }

    /// Gracefully shutdown the compositor (with Smithay backend)
    /// Gracefully shutdown the compositor (with Smithay backend)
    async fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down Axiom compositor...");

        self.running = false;

        // Clean up XWayland first
        if let Some(ref mut xwayland) = self.xwayland_manager {
            debug!("🔗 Shutting down XWayland...");
            xwayland.write().await.shutdown().await?;
        }

        // Clean up Smithay backend
        debug!("🚀 Shutting down Smithay backend...");
        self.smithay_backend.shutdown()?;

        // Clean up other subsystems
        debug!("🧩 Cleaning up compositor subsystems...");
        self.ipc_server.shutdown().await?;
        self.input_manager.write().shutdown()?;
        self.effects_engine.write().shutdown()?;
        self.workspace_manager.write().shutdown()?;
        self.window_manager.write().shutdown()?;

        info!("✅ Axiom compositor shutdown complete");
        Ok(())
    }

    /// Get current configuration
    pub fn config(&self) -> &AxiomConfig {
        &self.config
    }

    /// Check if compositor is running in windowed mode
    pub fn is_windowed(&self) -> bool {
        self.windowed
    }

    /// Single tick of the compositor (event processing + rendering)
    async fn tick(&mut self) -> Result<()> {
        use std::time::{Duration, Instant};
        let frame_start = Instant::now();
        let target_frame_time = Duration::from_micros(16667); // ~60 FPS

        let mut tick_error = false;

        // Process events
        if let Err(e) = self.process_events().await {
            tick_error = true;
            warn!("⚠️ Error processing events: {}", e);
        }

        // Render frame
        if let Err(e) = self.render_frame().await {
            tick_error = true;
            warn!("⚠️ Error rendering frame: {}", e);
        }

        // Update stability metrics
        if tick_error {
            self.consecutive_error_count += 1;
            warn!(
                "⚠️ Consecutive error count: {}",
                self.consecutive_error_count
            );
        } else {
            // Stable tick, reset error count
            self.consecutive_error_count = 0;
        }

        // Frame pacing: sleep for remaining time to target ~60 FPS
        let elapsed = frame_start.elapsed();
        if elapsed < target_frame_time {
            if let Some(sleep_duration) = target_frame_time.checked_sub(elapsed) {
                tokio::time::sleep(sleep_duration).await;
            }
        }

        // Check stability threshold
        if self.consecutive_error_count >= 5 {
            log::error!(
                "🚨 CRITICAL: Too many consecutive errors ({}). Initiating emergency shutdown.",
                self.consecutive_error_count
            );
            let _ = self.shutdown().await;
            return Err(anyhow::anyhow!(
                "Critical stability failure: too many consecutive errors"
            ));
        }

        Ok(())
    }

    // === Public Workspace Interaction Methods ===

    /// Scroll workspace left (for input handling)
    pub fn scroll_workspace_left(&mut self) {
        info!("⬅️ Scrolling workspace left");
        self.workspace_manager.write().scroll_left();
    }

    /// Scroll workspace right (for input handling)
    pub fn scroll_workspace_right(&mut self) {
        info!("➡️ Scrolling workspace right");
        self.workspace_manager.write().scroll_right();
    }

    /// Add a new window to the current workspace
    pub fn add_window(&mut self, title: String) -> u64 {
        // Create window in window manager
        let window_id = self.window_manager.write().add_window(title.clone());

        // Add to current workspace column
        self.workspace_manager.write().add_window(window_id);

        info!(
            "🪟 Added window '{}' (ID: {}) to current workspace",
            title, window_id
        );
        window_id
    }

    /// Remove a window from the compositor
    pub fn remove_window(&mut self, window_id: u64) {
        // Remove from workspace
        if let Some(column) = self.workspace_manager.write().remove_window(window_id) {
            info!(
                "🗑️ Removed window {} from workspace column {}",
                window_id, column
            );
        }

        // Remove from window manager
        self.window_manager.write().remove_window(window_id);
    }

    /// Move window to left workspace
    pub fn move_window_left(&mut self, window_id: u64) {
        if self.workspace_manager.write().move_window_left(window_id) {
            info!("⬅️ Moved window {} to left workspace", window_id);
        }
    }

    /// Move window to right workspace
    pub fn move_window_right(&mut self, window_id: u64) {
        if self.workspace_manager.write().move_window_right(window_id) {
            info!("➡️ Moved window {} to right workspace", window_id);
        }
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
        info!("📐 Updated viewport size to {}x{}", width, height);
    }

    /// Get reference to effects engine (for demo purposes)
    pub fn effects_engine(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, crate::effects::EffectsEngine> {
        self.effects_engine.read()
    }

    /// Get mutable reference to effects engine (for demo purposes)
    pub fn effects_engine_mut(
        &self,
    ) -> parking_lot::RwLockWriteGuard<'_, crate::effects::EffectsEngine> {
        self.effects_engine.write()
    }

    /// Propagate configuration changes to all subsystems
    fn update_subsystems_config(&mut self) {
        info!("🔄 Propagating configuration changes to subsystems...");

        // Update Effects Engine
        self.effects_engine
            .write()
            .update_config(self.config.effects.clone());

        // Update Workspace Manager
        self.workspace_manager
            .write()
            .update_config(self.config.workspace.clone());

        // Future: Update Input Manager, etc.
    }
}

// TODO: Future versions will integrate deeply with Smithay for full Wayland compositor functionality
// For now, we focus on getting the basic architecture working and communicating with Lazy UI
