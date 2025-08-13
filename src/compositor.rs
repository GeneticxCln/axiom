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
}

// TODO: Future versions will integrate deeply with Smithay for full Wayland compositor functionality
// For now, we focus on getting the basic architecture working and communicating with Lazy UI
