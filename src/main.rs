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

// Use jemalloc for better memory profiling when enabled
#[cfg(all(not(target_env = "msvc"), feature = "memory-profiling"))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(not(target_env = "msvc"), feature = "memory-profiling"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use anyhow::Result;
use clap::Parser;
use log::{error, info};

mod compositor;
mod config;
mod demo_phase4_effects;
mod demo_workspace;
mod effects;
mod input;
mod ipc;
#[cfg(feature = "real-compositor")]
mod multi_output; // Multi-output support for multiple monitors
#[cfg(feature = "real-compositor")]
mod real_input; // Real input handling from Smithay
#[cfg(feature = "real-compositor")]
mod real_smithay; // Real Smithay compositor implementation (Phase 5)
#[cfg(feature = "real-compositor")]
mod real_window; // Real window management with Wayland surfaces
mod renderer; // GPU rendering pipeline
mod smithay_backend;
mod smithay_enhanced; // Enhanced Smithay with Wayland socket support
mod wayland_protocols; // Real Wayland protocol implementation
mod window;
mod workspace;
mod xwayland;

use compositor::AxiomCompositor;
use config::AxiomConfig;

#[derive(Parser)]
#[command(name = "axiom")]
#[command(
    about = "A hybrid Wayland compositor combining scrollable workspaces with visual effects"
)]
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

    /// Use real Smithay backend with full Wayland protocol support (Phase 5)
    #[arg(long)]
    real_smithay: bool,

    /// Use completely real Smithay compositor with proper protocols (Phase 5)
    #[arg(long)]
    real_compositor: bool,
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
        }
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

    // Check if we should use completely real Smithay compositor
    if cli.real_compositor {
        info!("🚀 Using completely real Smithay compositor with full protocol support");
        info!("🌊 This is Phase 5: Production-ready Wayland compositor with proper protocols");

        // Run with real Smithay compositor
        #[cfg(feature = "real-compositor")]
        {
            return real_smithay::run_real_compositor(config);
        }
        #[cfg(not(feature = "real-compositor"))]
        {
            error!("❌ real-compositor feature not enabled at compile time. Rebuild with `--features real-compositor`.");
            return Ok(());
        }
    }

    // Check if we should use enhanced Smithay backend
    if cli.real_smithay {
        info!("🔧 Using enhanced Smithay backend with Wayland socket support");
        info!("🌊 This is Phase 5: Production-ready Wayland compositor");

        // Run with enhanced Smithay backend
        smithay_enhanced::run_enhanced_compositor(config, cli.windowed).await?;
        return Ok(());
    }

    // Initialize and run compositor with simulated backend
    info!("🏗️  Initializing Axiom compositor...");

    let mut compositor = AxiomCompositor::new(config, cli.windowed).await?;

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

    if cli.demo || cli.effects_demo {
        info!("🎆 All demos completed! Continuing with normal compositor operation...");
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
