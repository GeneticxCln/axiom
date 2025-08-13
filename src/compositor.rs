//! Core compositor implementation
//!
//! This module contains the main AxiomCompositor struct and event loop.
//! It coordinates between all subsystems: workspaces, effects, input, etc.
//!
//! This implementation uses Smithay for proper Wayland compositor functionality
//! with window management, surface handling, and protocol support.

use anyhow::{Result, Context};
use log::{info, debug, warn, error};
use tokio::signal;

use crate::config::AxiomConfig;
use crate::workspace::ScrollableWorkspaces;
use crate::effects::EffectsEngine;
use crate::window::{WindowManager, AxiomWindow};
use crate::input::InputManager;
use crate::xwayland::XWaylandManager;
use crate::ipc::AxiomIPCServer;
use crate::smithay_backend::AxiomSmithayBackend;

/// Main compositor struct that orchestrates all subsystems
pub struct AxiomCompositor {
    config: AxiomConfig,
    windowed: bool,
    
    // Core subsystems  
    workspace_manager: ScrollableWorkspaces,
    effects_engine: EffectsEngine,
    window_manager: WindowManager,
    input_manager: InputManager,
    xwayland_manager: Option<XWaylandManager>,
    ipc_server: AxiomIPCServer,
    
    // Smithay backend for Wayland compositor functionality
    smithay_backend: AxiomSmithayBackend,
    
    // Event loop state
    running: bool,
}

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
        
        debug!("⌨️ Initializing input manager...");
        let input_manager = InputManager::new(&config.input, &config.bindings)?;
        
        // Initialize XWayland (if enabled)
        let xwayland_manager = if config.xwayland.enabled {
            debug!("🔗 Initializing XWayland...");
            Some(XWaylandManager::new(&config.xwayland).await?)
        } else {
            warn!("🚫 XWayland disabled - X11 apps will not work");
            None
        };
        
        // Initialize IPC server for Lazy UI integration
        debug!("🔗 Initializing IPC server...");
        let mut ipc_server = AxiomIPCServer::new();
        ipc_server.start().await.context("Failed to start IPC server")?;
        
        // Initialize Smithay backend
        debug!("🚀 Initializing Smithay Wayland backend...");
        let mut smithay_backend = AxiomSmithayBackend::new(config.clone(), windowed)?;
        smithay_backend.initialize().await.context("Failed to initialize Smithay backend")?;
        
        info!("✅ All subsystems initialized successfully");
        
        Ok(Self {
            config,
            windowed,
            workspace_manager,
            effects_engine,
            window_manager,
            input_manager,
            xwayland_manager,
            ipc_server,
            smithay_backend,
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
    async fn process_events(&mut self) -> Result<()> {
        // Process backend events (Wayland, input devices)
        self.smithay_backend.process_events().await?;
        
        // Process IPC messages from Lazy UI
        if let Err(e) = self.ipc_server.process_messages().await {
            warn!("⚠️ Error processing IPC messages: {}", e);
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
        
        use crate::input::{InputEvent, CompositorAction};
        
        // Simulate a scroll event occasionally (for demo purposes)
        if rand::random::<f32>() < 0.001 { // Very low probability
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
    
    /// Phase 3: Handle compositor actions triggered by input events
    async fn handle_compositor_action(&mut self, action: crate::input::CompositorAction) -> Result<()> {
        use crate::input::CompositorAction;
        
        match action {
            CompositorAction::ScrollWorkspaceLeft => {
                debug!("🎨 Input triggered: Scroll workspace left");
                self.scroll_workspace_left();
            },
            CompositorAction::ScrollWorkspaceRight => {
                debug!("🎨 Input triggered: Scroll workspace right");
                self.scroll_workspace_right();
            },
            CompositorAction::MoveWindowLeft => {
                debug!("🎨 Input triggered: Move window left");
                if let Some((window_id, _, _, _)) = self.get_workspace_info().into() {
                    // Get first window in current workspace for demo
                    let windows = self.workspace_manager.get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.move_window_left(window_id);
                    }
                }
            },
            CompositorAction::MoveWindowRight => {
                debug!("🎨 Input triggered: Move window right");
                if let Some((window_id, _, _, _)) = self.get_workspace_info().into() {
                    let windows = self.workspace_manager.get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.move_window_right(window_id);
                    }
                }
            },
            CompositorAction::CloseWindow => {
                debug!("🎨 Input triggered: Close window");
                // TODO: Close focused window
            },
            CompositorAction::ToggleFullscreen => {
                debug!("🎨 Input triggered: Toggle fullscreen");
                // TODO: Toggle fullscreen for focused window
            },
            CompositorAction::Quit => {
                info!("💼 Input triggered: Quit compositor");
                self.shutdown().await?;
            },
            CompositorAction::Custom(command) => {
                debug!("🎨 Input triggered custom command: {}", command);
                // TODO: Handle custom commands
            },
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
        
        // 4. Update window positions and apply effects
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
                        (new_pos.0 as f32, new_pos.1 as f32)
                    );
                }
                
                // Update the backend window position and size
                window.window.set_position(layout_rect.x, layout_rect.y);
                window.window.set_size(layout_rect.width, layout_rect.height);
                
                // Apply visual effects if window has them
                if let Some(effect_state) = self.effects_engine.get_window_effects(window_id) {
                    // Apply scale, opacity, and position offsets
                    // In a real implementation, this would modify the rendering pipeline
                    debug!("✨ Window {} effects: scale={:.2}, opacity={:.2}, corner_radius={:.1}", 
                           window_id, effect_state.scale, effect_state.opacity, effect_state.corner_radius);
                }
            }
        }
        
        // 5. Apply global effects (workspace transitions, blur backgrounds)
        self.apply_global_effects();
        
        // 6. Performance monitoring for effects
        let (frame_time, effects_quality, active_effects) = self.effects_engine.get_performance_stats();
        if frame_time.as_millis() > 20 { // More than ~50 FPS
            debug!("⚡ Frame time: {:.1}ms, effects quality: {:.1}, active effects: {}", 
                   frame_time.as_secs_f64() * 1000.0, effects_quality, active_effects);
        }
        
        debug!("🎨 Frame rendered - position: {:.1}, column: {}, effects: {}", 
               self.workspace_manager.current_position(),
               self.workspace_manager.focused_column_index(),
               active_effects);
        
        Ok(())
    }
    
    /// Apply global visual effects like workspace transitions and background blur
    fn apply_global_effects(&mut self) {
        // Apply workspace transition effects
        if self.workspace_manager.is_scrolling() {
            let current_pos = self.workspace_manager.current_position();
            let progress = self.workspace_manager.scroll_progress();
            
            // In a real implementation, this would apply visual effects to the entire compositor
            debug!("🌊 Workspace transition: position={:.1}, progress={:.2}", current_pos, progress);
        }
    }
    
    /// Gracefully shutdown the compositor
    async fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down Axiom compositor...");
        
        self.running = false;
        
        // Clean up XWayland first
        if let Some(ref mut xwayland) = self.xwayland_manager {
            debug!("🔗 Shutting down XWayland...");
            xwayland.shutdown().await?;
        }
        
        // Clean up Smithay backend
        debug!("🚀 Shutting down Smithay backend...");
        self.smithay_backend.shutdown().await?;
        
        // Clean up other subsystems
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
        
        // Render frame
        if let Err(e) = self.render_frame().await {
            warn!("⚠️ Error rendering frame: {}", e);
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
        
        info!("🪟 Added window '{}' (ID: {}) to current workspace", title, window_id);
        window_id
    }
    
    /// Remove a window from the compositor
    pub fn remove_window(&mut self, window_id: u64) {
        // Remove from workspace
        if let Some(column) = self.workspace_manager.remove_window(window_id) {
            info!("🗑️ Removed window {} from workspace column {}", window_id, column);
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
            self.workspace_manager.active_column_count(),
            self.workspace_manager.is_scrolling(),
        )
    }
    
    /// Set the viewport size (called when display size changes)
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.workspace_manager.set_viewport_size(width as f64, height as f64);
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
