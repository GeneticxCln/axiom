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
use log::{debug, error, info, warn};

use axiom::compositor::AxiomCompositor;
use axiom::config::AxiomConfig;
use axiom::decoration::DecorationManager;
use axiom::effects::EffectsEngine;
use axiom::input::InputManager;
use axiom::ipc::AxiomIPCServer;
use axiom::renderer::AxiomRenderer;
use axiom::window::WindowManager;
use axiom::workspace::ScrollableWorkspaces;
use axiom::xwayland::XWaylandManager;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::RwLock as AsyncRwLock;
// use axiom::generate_default_config;

#[derive(Parser)]
#[command(name = "axiom")]
#[command(
    about = "A hybrid Wayland compositor combining scrollable workspaces with visual effects"
)]
#[command(version)]
#[allow(clippy::struct_excessive_bools)]
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

    // Set global panic handler
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .unwrap_or_else(|| std::panic::Location::caller());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "Box<Any>"
        };
        error!("🚨 COMPOSITOR PANIC [{}]: {}", location, payload);
    }));

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

    // Initialize and run compositor
    info!("🏗️  Initializing Axiom compositor...");

    // Create shared managers
    // use parking_lot::RwLock; // Use top level
    // use tokio::sync::RwLock as AsyncRwLock; // Use top level
    // use std::sync::Arc; // Use top level

    let workspace_manager = std::sync::Arc::new(parking_lot::RwLock::new(
        ScrollableWorkspaces::new(&config.workspace)?,
    ));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)?));
    let effects_engine = Arc::new(RwLock::new(EffectsEngine::new(&config.effects)?));
    let decoration_manager = Arc::new(RwLock::new(DecorationManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )?));

    // XWayland
    let xwayland_manager = if config.xwayland.enabled {
        debug!("🔗 Initializing XWayland...");
        Some(Arc::new(AsyncRwLock::new(
            XWaylandManager::new(&config.xwayland).await?,
        )))
    } else {
        debug!("🔗 XWayland disabled by config");
        None
    };

    let ipc_server = AxiomIPCServer::new();

    // Renderer
    let renderer = match AxiomRenderer::new_headless().await {
        Ok(r) => Arc::new(RwLock::new(r)),
        Err(e) => {
            warn!("⚠️ Failed to initialize headless renderer: {}", e);
            // This is problematic if new() expects it.
            anyhow::bail!("Failed to initialize renderer");
        }
    };

    let mut compositor = AxiomCompositor::new(
        config.clone(),
        cli.windowed,
        workspace_manager.clone(),
        effects_engine.clone(),
        window_manager.clone(),
        decoration_manager.clone(),
        input_manager.clone(),
        xwayland_manager.clone(),
        ipc_server,
        renderer.clone(),
    )
    .await?;

    info!("✨ Axiom is ready! Where productivity meets beauty.");

    // Run demos if requested
    if cli.demo {
        info!("🎭 Running Phase 3 scrollable workspace demo...");
        axiom::demo_workspace::run_comprehensive_test(&mut compositor).await?;
        info!("🎆 Phase 3 demo completed!");
    }

    if cli.effects_demo {
        info!("🎨 Running Phase 4 visual effects demo...");
        axiom::demo_phase4_effects::display_effects_capabilities(&compositor);
        axiom::demo_phase4_effects::run_phase4_effects_demo(&mut compositor).await?;
        info!("🎆 Phase 4 effects demo completed!");
    }

    if cli.demo || cli.effects_demo {
        info!("🎆 All demos completed! Continuing with normal compositor operation...");
    }

    // Main event loop
    Box::pin(compositor.run()).await?;

    info!("👋 Axiom compositor shutting down");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        // Test basic CLI parsing
        let cli = Cli::try_parse_from(&["axiom"]).expect("CLI parse should succeed");
        assert!(!cli.debug);
        assert!(!cli.windowed);
        assert!(!cli.no_effects);
    }

    #[test]
    fn test_cli_flags() {
        let cli = Cli::try_parse_from(&["axiom", "--debug", "--windowed", "--no-effects"]).expect("CLI parse should succeed");
        assert!(cli.debug);
        assert!(cli.windowed);
        assert!(cli.no_effects);
    }
}
