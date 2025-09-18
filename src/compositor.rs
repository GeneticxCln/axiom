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

use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::ipc::{AxiomIPCServer, RuntimeCommand};
use crate::renderer::AxiomRenderer;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use crate::xwayland::XWaylandManager;
use tokio::sync::mpsc;

/// Main compositor struct that orchestrates all subsystems
pub struct AxiomCompositor {
    #[allow(dead_code)]
    config: AxiomConfig,
    #[allow(dead_code)]
    windowed: bool,

    // Core subsystems
    workspace_manager: ScrollableWorkspaces,
    effects_engine: EffectsEngine,
    window_manager: WindowManager,
    #[allow(dead_code)]
    decoration_manager: DecorationManager,
    input_manager: InputManager,
    xwayland_manager: Option<XWaylandManager>,
    ipc_server: AxiomIPCServer,

    // Runtime command receiver from IPC
    runtime_cmd_rx: mpsc::UnboundedReceiver<RuntimeCommand>,

    // Renderer (headless for now, scaffolding real GPU path)
    renderer: Option<AxiomRenderer>,

    // Event loop state
    running: bool,
}

#[allow(dead_code)]
impl AxiomCompositor {
    /// Create a new Axiom compositor instance
    pub async fn new(config: AxiomConfig, windowed: bool) -> Result<Self> {
        info!("🏗️ Initializing Axiom compositor...");

        // Initialize our custom subsystems
        debug!("📱 Initializing scrollable workspaces...");
        let workspace_manager = ScrollableWorkspaces::new(&config.workspace)?;

        debug!("✨ Initializing effects engine...");
        let effects_engine = EffectsEngine::new(&config.effects)?;

        debug!("🪟 Initializing window manager...");
        let window_manager = WindowManager::new(&config.window)?;

        debug!("🎨 Initializing decoration manager...");
        let decoration_manager = DecorationManager::new(&config.window);

        debug!("⌨️ Initializing input manager...");
        let input_manager = InputManager::new(&config.input, &config.bindings)?;

        // Initialize XWayland (if enabled)
        let xwayland_manager = if config.xwayland.enabled {
            debug!("🔗 Initializing XWayland...");
            Some(XWaylandManager::new(&config.xwayland)?)
        } else {
            warn!("🚫 XWayland disabled - X11 apps will not work");
            None
        };

        // Initialize IPC server for Lazy UI integration
        debug!("🔗 Initializing IPC server...");
        let mut ipc_server = AxiomIPCServer::new();
        // Provide IPC with a read-only snapshot of the current configuration
        AxiomIPCServer::set_config_snapshot(config.clone());
        // Create runtime command channel and register sender with IPC module
        let (runtime_tx, runtime_rx) = mpsc::unbounded_channel();
        AxiomIPCServer::register_runtime_command_sender(runtime_tx);
        ipc_server
            .start()
            .await
            .context("Failed to start IPC server")?;

        warn!(
            "⚠️ Running in simulation mode (Smithay window server is handled by main when enabled)"
        );

        // Initialize a headless renderer as scaffolding for real GPU rendering
        let renderer = match AxiomRenderer::new_headless().await {
            Ok(r) => Some(r),
            Err(e) => {
                warn!("⚠️ Failed to initialize headless renderer: {}", e);
                None
            }
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
            renderer,
            runtime_cmd_rx: runtime_rx,
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

    /// Phase 3: Process all pending compositor events
    async fn process_events(&mut self) -> Result<()> {
        // Process IPC messages from Lazy UI
        if let Err(e) = self.ipc_server.process_messages().await {
            warn!("⚠️ Error processing IPC messages: {}", e);
        }

        // Drain runtime commands from IPC and apply changes
        while let Ok(cmd) = self.runtime_cmd_rx.try_recv() {
            self.apply_runtime_command(cmd);
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

            let actions = self.input_manager.process_input_event(event);
            for action in actions {
                self.handle_compositor_action(action).await?;
            }
        }

        Ok(())
    }

    /// Apply runtime command received from IPC
    fn apply_runtime_command(&mut self, cmd: RuntimeCommand) {
        match cmd {
            RuntimeCommand::SetConfig { key, value } => {
                // Only support a safe subset of keys for live updates
                match key.as_str() {
                    "effects.enabled" => {
                        if let Some(b) = value.as_bool() {
                            self.effects_engine.set_effects_enabled(b);
                            self.config.effects.enabled = b;
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    "effects.blur.radius" => {
                        if let Some(r) = value.as_f64() {
                            let r = r as f32;
                            self.effects_engine.set_blur_radius(r);
                            self.config.effects.blur.radius = r as u32;
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    "effects.animations.duration" => {
                        if let Some(ms) = value.as_u64() {
                            let ms = ms as u32;
                            self.effects_engine.set_animation_duration(ms);
                            // config updated inside setter
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        } else if let Some(f) = value.as_f64() {
                            // allow float too
                            let ms = f.max(0.0) as u32;
                            self.effects_engine.set_animation_duration(ms);
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    "workspace.scroll_speed" => {
                        if let Some(s) = value.as_f64() {
                            let s = s.clamp(0.01, 10.0);
                            self.workspace_manager.set_scroll_speed(s);
                            self.config.workspace.scroll_speed = s;
                            if let Err(e) = self.config.validate() {
                                warn!("Invalid runtime config for workspace.scroll_speed: {}", e);
                            }
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    "input.pan_threshold" => {
                        if let Some(v) = value.as_f64() {
                            let v = v.clamp(0.0, 1000.0);
                            self.input_manager.update_thresholds(Some(v), None, None);
                            self.config.input.pan_threshold = v;
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    "input.scroll_threshold" => {
                        if let Some(v) = value.as_f64() {
                            let v = v.clamp(0.0, 1000.0);
                            self.input_manager.update_thresholds(None, Some(v), None);
                            self.config.input.scroll_threshold = v;
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    "input.swipe_threshold" => {
                        if let Some(v) = value.as_f64() {
                            let v = v.clamp(0.0, 5000.0);
                            self.input_manager.update_thresholds(None, None, Some(v));
                            self.config.input.swipe_threshold = v;
                            AxiomIPCServer::set_config_snapshot(self.config.clone());
                        }
                    }
                    _ => {
                        debug!("Unsupported live SetConfig key: {}", key);
                    }
                }
            }
            RuntimeCommand::EffectsControl {
                enabled,
                blur_radius,
                animation_speed,
            } => {
                if let Some(b) = enabled {
                    self.effects_engine.set_effects_enabled(b);
                    self.config.effects.enabled = b;
                }
                if let Some(r) = blur_radius {
                    self.effects_engine.set_blur_radius(r);
                    self.config.effects.blur.radius = r as u32;
                }
                if let Some(speed) = animation_speed {
                    self.effects_engine.set_animation_speed(speed);
                }
                AxiomIPCServer::set_config_snapshot(self.config.clone());
            }
            RuntimeCommand::ClipboardSet { .. } | RuntimeCommand::ClipboardGet => {
                // Clipboard handled directly in IPC layer for now
            }
            RuntimeCommand::Workspace { action, parameters } => {
                let act = action.as_str();
                match act {
                    "scroll_left" => self.scroll_workspace_left(),
                    "scroll_right" => self.scroll_workspace_right(),
                    "scroll_to_column" => {
                        if let Some(idx) = parameters.get("index").and_then(|v| v.as_i64()) {
                            self.workspace_manager.scroll_to_column(idx as i32);
                        }
                    }
                    "move_focused_left" => {
                        if let Some(&window_id) =
                            self.workspace_manager.get_focused_column_windows().first()
                        {
                            self.move_window_left(window_id);
                        }
                    }
                    "move_focused_right" => {
                        if let Some(&window_id) =
                            self.workspace_manager.get_focused_column_windows().first()
                        {
                            self.move_window_right(window_id);
                        }
                    }
                    "close_focused" => {
                        if let Some(focused_id) = self.window_manager.focused_window_id() {
                            self.remove_window(focused_id);
                            self.effects_engine.animate_window_close(focused_id);
                        }
                    }
                    "toggle_fullscreen" => {
                        if let Some(focused_id) = self.window_manager.focused_window_id() {
                            let _ = self.window_manager.toggle_fullscreen(focused_id);
                        }
                    }
                    "add_window" => {
                        let title = parameters
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("IPC Window")
                            .to_string();
                        self.add_window(title);
                    }
                    "set_viewport_size" => {
                        if let (Some(w), Some(h)) = (
                            parameters.get("width").and_then(|v| v.as_u64()),
                            parameters.get("height").and_then(|v| v.as_u64()),
                        ) {
                            self.set_viewport_size(w as u32, h as u32);
                        }
                    }
                    _ => {
                        debug!("Unsupported workspace action: {}", action);
                    }
                }
            }
        }
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
                    let windows = self.workspace_manager.get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.move_window_left(window_id);
                    }
                }
            }
            CompositorAction::MoveWindowRight => {
                debug!("🎨 Input triggered: Move window right");
                if let Some((_window_id, _, _, _)) = self.get_workspace_info().into() {
                    let windows = self.workspace_manager.get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.move_window_right(window_id);
                    }
                }
            }
            CompositorAction::CloseWindow => {
                debug!("🎨 Input triggered: Close window");
                if let Some(focused_id) = self.window_manager.focused_window_id() {
                    self.remove_window(focused_id);
                    self.effects_engine.animate_window_close(focused_id);
                }
            }
            CompositorAction::ToggleFullscreen => {
                debug!("🎨 Input triggered: Toggle fullscreen");
                if let Some(focused_id) = self.window_manager.focused_window_id() {
                    let _ = self.window_manager.toggle_fullscreen(focused_id);
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
    async fn render_frame(&mut self) -> Result<()> {
        // 1. Update workspace positions/animations
        self.workspace_manager.update_animations()?;

        // 2. Update visual effects (animations, blur, shadows)
        self.effects_engine.update()?;

        // 3. Calculate workspace layouts for all visible windows
        let workspace_layouts = self.workspace_manager.calculate_workspace_layouts();

        // 4. Update window positions, apply effects, and push to renderer
        for (window_id, layout_rect) in workspace_layouts {
            if let Some(window) = self.window_manager.get_window_mut(window_id) {
                // Check if window position changed (for move animations)
                let old_pos = window.window.position;
                let new_pos = (layout_rect.x, layout_rect.y);

                if old_pos != new_pos {
                    // Trigger move animation
                    self.effects_engine.animate_window_move(
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

                // Determine render-time transforms
                let mut scale = 1.0_f32;
                let mut opacity = 1.0_f32;
                let mut offset = (0.0_f32, 0.0_f32);

                if let Some(effect_state) = self.effects_engine.get_window_effects(window_id) {
                    scale = effect_state.scale;
                    opacity = effect_state.opacity;
                    offset = effect_state.position_offset;
                    debug!(
                        "✨ Window {} effects: scale={:.2}, opacity={:.2}, corner_radius={:.1}",
                        window_id,
                        effect_state.scale,
                        effect_state.opacity,
                        effect_state.corner_radius
                    );
                }

                // Feed to renderer (headless placeholder): apply scale and offset
                if let Some(renderer) = self.renderer.as_mut() {
                    let x = layout_rect.x as f32 + offset.0;
                    let y = layout_rect.y as f32 + offset.1;
                    let w = layout_rect.width as f32 * scale;
                    let h = layout_rect.height as f32 * scale;
                    renderer.upsert_window_rect(window_id, (x, y), (w, h), opacity);
                }
            }
        }

        // Render with headless renderer for now
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(e) = renderer.render() {
                warn!("⚠️ Renderer error: {}", e);
            }
        }

        // 5. Apply global effects (workspace transitions, blur backgrounds)
        self.apply_global_effects();

        // 6. Performance monitoring for effects
        let (frame_time, effects_quality, active_effects) =
            self.effects_engine.get_performance_stats();
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
            self.workspace_manager.current_position(),
            self.workspace_manager.focused_column_index(),
            active_effects
        );

        Ok(())
    }

    /// Apply global visual effects like workspace transitions and background blur
    fn apply_global_effects(&mut self) {
        // Apply workspace transition effects
        if self.workspace_manager.is_scrolling() {
            let current_pos = self.workspace_manager.current_position();
            let progress = self.workspace_manager.scroll_progress();

            // In a real implementation, this would apply visual effects to the entire compositor
            debug!(
                "🌊 Workspace transition: position={:.1}, progress={:.2}",
                current_pos, progress
            );
        }
    }

    /// Gracefully shutdown the compositor
    async fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down Axiom compositor...");

        self.running = false;

        // Clean up XWayland first
        if let Some(ref mut xwayland) = self.xwayland_manager {
            debug!("🔗 Shutting down XWayland...");
            xwayland.shutdown()?;
        }

        // Clean up other subsystems (no direct Smithay backend owned here)
        debug!("🧩 Cleaning up compositor subsystems...");
        self.input_manager.shutdown()?;
        self.effects_engine.shutdown()?;
        self.workspace_manager.shutdown()?;
        self.window_manager.shutdown()?;

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
        // Process events
        if let Err(e) = self.process_events().await {
            warn!("⚠️ Error processing events: {}", e);
        }

        // Render only when we have damage or active animations/scrolling
        let mut should_render = true; // default for now until renderer is present
        if let Some(renderer) = self.renderer.as_ref() {
            should_render = renderer.has_dirty()
                || self.workspace_manager.is_scrolling()
                || self.effects_engine.get_animation_stats().active_animations > 0;
        }

        if should_render {
            if let Err(e) = self.render_frame().await {
                warn!("⚠️ Error rendering frame: {}", e);
            }
        } else {
            // Sleep briefly to reduce CPU while idle
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }

        Ok(())
    }

    // === Public Workspace Interaction Methods ===

    /// Scroll workspace left (for input handling)
    pub fn scroll_workspace_left(&mut self) {
        info!("⬅️ Scrolling workspace left");
        self.workspace_manager.scroll_left();
    }

    /// Scroll workspace right (for input handling)
    pub fn scroll_workspace_right(&mut self) {
        info!("➡️ Scrolling workspace right");
        self.workspace_manager.scroll_right();
    }

    /// Add a new window to the current workspace
    pub fn add_window(&mut self, title: String) -> u64 {
        // Create window in window manager
        let window_id = self.window_manager.add_window(title.clone());

        // Add to current workspace column
        self.workspace_manager.add_window(window_id);

        info!(
            "🪟 Added window '{}' (ID: {}) to current workspace",
            title, window_id
        );
        window_id
    }

    /// Remove a window from the compositor
    pub fn remove_window(&mut self, window_id: u64) {
        // Remove from workspace
        if let Some(column) = self.workspace_manager.remove_window(window_id) {
            info!(
                "🗑️ Removed window {} from workspace column {}",
                window_id, column
            );
        }

        // Remove from window manager
        self.window_manager.remove_window(window_id);
    }

    /// Move window to left workspace
    pub fn move_window_left(&mut self, window_id: u64) {
        if self.workspace_manager.move_window_left(window_id) {
            info!("⬅️ Moved window {} to left workspace", window_id);
        }
    }

    /// Move window to right workspace
    pub fn move_window_right(&mut self, window_id: u64) {
        if self.workspace_manager.move_window_right(window_id) {
            info!("➡️ Moved window {} to right workspace", window_id);
        }
    }

    /// Get current workspace information
    pub fn get_workspace_info(&self) -> (i32, f64, usize, bool) {
        (
            self.workspace_manager.focused_column_index(),
            self.workspace_manager.current_position(),
            self.workspace_manager.active_window_count(),
            self.workspace_manager.is_scrolling(),
        )
    }

    /// Set the viewport size (called when display size changes)
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.workspace_manager
            .set_viewport_size(width as f64, height as f64);
        info!("📐 Updated viewport size to {}x{}", width, height);
    }

    /// Get reference to effects engine (for demo purposes)
    pub fn effects_engine(&self) -> &crate::effects::EffectsEngine {
        &self.effects_engine
    }

    /// Get mutable reference to effects engine (for demo purposes)
    pub fn effects_engine_mut(&mut self) -> &mut crate::effects::EffectsEngine {
        &mut self.effects_engine
    }
}

// TODO: Future versions will integrate deeply with Smithay for full Wayland compositor functionality
// For now, we focus on getting the basic architecture working and communicating with Lazy UI
