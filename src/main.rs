//! # Axiom - Hybrid Wayland Compositor
//!
//! The first Wayland compositor combining niri's scrollable workspace innovation 
//! with Hyprland's visual effects system.
//!
//! ## Architecture Overview
//!
//! Axiom is built on a modular architecture:
//! - `compositor`: Core compositor logic and event loop
//! - `workspace`: Scrollable workspace management (niri-inspired)
//! - `effects`: Visual effects engine (Hyprland-inspired) 
//! - `window`: Window management and layout algorithms
//! - `input`: Keyboard, mouse, and gesture input handling
//! - `config`: Configuration parsing and management
//! - `xwayland`: X11 compatibility layer

use anyhow::Result;
use clap::Parser;
use log::{info, error};

mod compositor;
mod decoration;
mod smithay_backend_simple;
mod smithay_backend_real;  // Real Smithay implementation
// mod smithay_backend_production;  // Phase 6: Production Smithay backend (disabled for now)
mod smithay_backend_minimal;  // Phase 6.1: Minimal working backend
mod smithay_backend_phase6;  // Phase 6.1: WORKING Smithay backend
mod smithay_backend_phase6_2;  // Phase 6.2: Full protocol implementation
mod workspace;
mod effects;
mod window;
mod input;
mod config;
mod xwayland;
mod ipc;
mod demo_workspace;
mod demo_phase4_effects;
mod demo_phase6_minimal;
mod demo_phase6_working;

use compositor::AxiomCompositor;
use config::AxiomConfig;

#[derive(Parser)]
#[command(name = "axiom")]
#[command(about = "A hybrid Wayland compositor combining scrollable workspaces with visual effects")]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "~/.config/axiom/axiom.toml")]
    config: String,
    
    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
    
    /// Run in windowed mode (for development)
    #[arg(short, long)]
    windowed: bool,
    
    /// Disable visual effects (performance mode)
    #[arg(long)]
    no_effects: bool,
    
    /// Run scrollable workspace demo (Phase 3)
    #[arg(long)]
    demo: bool,
    
    /// Run visual effects demo (Phase 4)
    #[arg(long)]
    effects_demo: bool,
    
    /// Run Phase 6.2 Smithay backend demo with protocol simulation
    #[arg(long)]
    phase6_2_demo: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize logging
    if cli.debug {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }
    
    info!("🚀 Starting Axiom - Hybrid Wayland Compositor");
    info!("📄 Version: {}", env!("CARGO_PKG_VERSION"));
    
    // Load configuration
    let config = match AxiomConfig::load(&cli.config) {
        Ok(config) => {
            info!("✅ Configuration loaded from: {}", cli.config);
            config
        },
        Err(e) => {
            error!("❌ Failed to load configuration: {}", e);
            info!("📝 Using default configuration");
            AxiomConfig::default()
        }
    };
    
    // Override config with CLI flags
    let mut config = config;
    if cli.no_effects {
        config.effects.enabled = false;
        info!("🚫 Visual effects disabled via CLI flag");
    }
    
    // Initialize and run compositor
    info!("🏗️  Initializing Axiom compositor...");
    
    let mut compositor = AxiomCompositor::new(config.clone(), cli.windowed).await?;
    
    info!("✨ Axiom is ready! Where productivity meets beauty.");
    
    // Run demos if requested
    if cli.demo {
        info!("🎭 Running Phase 3 scrollable workspace demo...");
        demo_workspace::run_comprehensive_test(&mut compositor).await?;
        info!("🎆 Phase 3 demo completed!");
    }
    
    if cli.effects_demo {
        info!("🎨 Running Phase 4 visual effects demo...");
        demo_phase4_effects::display_effects_capabilities(&compositor);
        demo_phase4_effects::run_phase4_effects_demo(&mut compositor).await?;
        info!("🎆 Phase 4 effects demo completed!");
    }
    
    if cli.phase6_2_demo {
        info!("🌊 Running Phase 6.2 Smithay backend demo with protocol simulation...");
        run_phase6_2_demo(config.clone(), cli.windowed).await?;
        info!("🎆 Phase 6.2 demo completed!");
        return Ok(()); // Exit after demo
    }
    
    if cli.demo || cli.effects_demo || cli.phase6_2_demo {
        info!("🎆 All demos completed! Continuing with normal compositor operation...");
    }
    
    // Main event loop
    compositor.run().await?;
    
    info!("👋 Axiom compositor shutting down");
    Ok(())
}

/// Run Phase 6.2 Smithay backend demo with protocol simulation
async fn run_phase6_2_demo(config: AxiomConfig, windowed: bool) -> Result<()> {
    use crate::smithay_backend_phase6_2::AxiomSmithayBackendPhase6_2;
    use crate::workspace::ScrollableWorkspaces;
    use crate::window::WindowManager;
    use crate::effects::EffectsEngine;
    use crate::decoration::DecorationManager;
    use crate::input::InputManager;
    use parking_lot::RwLock;
    use std::sync::Arc;
    
    info!("🌊 Initializing Phase 6.2 Enhanced Protocol Simulation Backend...");
    info!("🔧 Creating required manager components...");
    
    // Create all required manager components
    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)?));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)?));
    let effects_engine = Arc::new(RwLock::new(EffectsEngine::new(&config.effects)?));
    let decoration_manager = Arc::new(RwLock::new(DecorationManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(&config.input, &config.bindings)?));
    
    let mut backend = AxiomSmithayBackendPhase6_2::new(
        config,
        windowed,
        workspace_manager,
        window_manager,
        effects_engine,
        decoration_manager,
        input_manager,
    )?;
    
    backend.initialize().await?;
    
    info!("✨ Phase 6.2 backend initialized successfully!");
    info!("🔌 Socket: {:?}", backend.socket_name());
    
    // Run the comprehensive demonstration
    backend.demonstrate_protocol_simulation().await?;
    
    // Clean up demonstration
    backend.demonstrate_client_cleanup().await?;
    
    info!("📊 Final status report:");
    backend.report_status();
    
    // Shutdown cleanly
    backend.shutdown().await?;
    
    info!("🎯 Phase 6.2 demo completed successfully!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        // Test basic CLI parsing
        let cli = Cli::try_parse_from(&["axiom"]).unwrap();
        assert!(!cli.debug);
        assert!(!cli.windowed);
        assert!(!cli.no_effects);
    }
    
    #[test] 
    fn test_cli_flags() {
        let cli = Cli::try_parse_from(&["axiom", "--debug", "--windowed", "--no-effects"]).unwrap();
        assert!(cli.debug);
        assert!(cli.windowed);  
        assert!(cli.no_effects);
    }
}
