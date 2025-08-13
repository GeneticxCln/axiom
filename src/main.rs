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
mod smithay_backend;
mod workspace;
mod effects;
mod window;
mod input;
mod config;
mod xwayland;
mod ipc;
mod demo_workspace;

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
    
    /// Run scrollable workspace demo
    #[arg(long)]
    demo: bool,
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
    
    let mut compositor = AxiomCompositor::new(config, cli.windowed).await?;
    
    info!("✨ Axiom is ready! Where productivity meets beauty.");
    
    // Run demo if requested
    if cli.demo {
        info!("🎭 Running Phase 3 scrollable workspace demo...");
        demo_workspace::run_comprehensive_test(&mut compositor).await?;
        info!("🎆 Demo completed! Continuing with normal compositor operation...");
    }
    
    // Main event loop
    compositor.run().await?;
    
    info!("👋 Axiom compositor shutting down");
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
