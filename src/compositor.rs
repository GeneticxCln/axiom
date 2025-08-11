//! Core compositor implementation
//!
//! This module contains the main AxiomCompositor struct and event loop.
//! It coordinates between all subsystems: workspaces, effects, input, etc.

use anyhow::Result;
use log::{info, debug, warn};
use tokio::signal;

use crate::config::AxiomConfig;
use crate::workspace::ScrollableWorkspaces;
use crate::effects::EffectsEngine;
use crate::window::WindowManager;
use crate::input::InputManager;
use crate::xwayland::XWaylandManager;

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
    
    // Event loop state
    running: bool,
}

impl AxiomCompositor {
    /// Create a new Axiom compositor instance
    pub async fn new(config: AxiomConfig, windowed: bool) -> Result<Self> {
        info!("🏗️ Initializing Axiom compositor subsystems...");
        
        // Initialize workspace management (niri-inspired)
        debug!("📱 Initializing scrollable workspaces...");
        let workspace_manager = ScrollableWorkspaces::new(&config.workspace)?;
        
        // Initialize effects engine (hyprland-inspired) 
        debug!("✨ Initializing effects engine...");
        let effects_engine = EffectsEngine::new(&config.effects)?;
        
        // Initialize window management
        debug!("🪟 Initializing window manager...");
        let window_manager = WindowManager::new(&config.window)?;
        
        // Initialize input handling
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
        
        info!("✅ All subsystems initialized successfully");
        
        Ok(Self {
            config,
            windowed,
            workspace_manager,
            effects_engine,
            window_manager,
            input_manager,
            xwayland_manager,
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
                
                // Process compositor events
                result = self.process_events() => {
                    if let Err(e) = result {
                        warn!("⚠️ Error processing events: {}", e);
                        // Continue running unless it's a critical error
                    }
                }
                
                // Render frame
                result = self.render_frame() => {
                    if let Err(e) = result {
                        warn!("⚠️ Error rendering frame: {}", e);
                    }
                }
            }
        }
        
        info!("🛑 Axiom compositor event loop finished");
        Ok(())
    }
    
    /// Process all pending compositor events
    async fn process_events(&mut self) -> Result<()> {
        // TODO: Implement actual event processing
        // This will handle:
        // - Wayland client requests
        // - Input events (keyboard, mouse, gestures)
        // - Window state changes
        // - XWayland events
        
        debug!("🔄 Processing compositor events");
        
        // Placeholder implementation
        tokio::time::sleep(tokio::time::Duration::from_millis(16)).await; // ~60fps
        
        Ok(())
    }
    
    /// Render a single frame
    async fn render_frame(&mut self) -> Result<()> {
        // TODO: Implement actual rendering pipeline
        // This will:
        // 1. Update workspace positions/animations
        // 2. Apply visual effects (blur, shadows, etc.)
        // 3. Render all windows to screen
        // 4. Handle multi-monitor output
        
        debug!("🎨 Rendering frame");
        
        // Update workspace animations
        self.workspace_manager.update_animations()?;
        
        // Update effects
        self.effects_engine.update()?;
        
        // Render everything
        // TODO: Actual rendering implementation
        
        Ok(())
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
        
        // Clean up other subsystems
        debug!("🧹 Cleaning up compositor subsystems...");
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
}
