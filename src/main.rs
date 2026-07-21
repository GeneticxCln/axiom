//! # Axiom - Scrollable Workspaces Wayland Compositor
//!
//! A winit-only Wayland compositor (GLES rendering) featuring niri-inspired
//! scrollable workspaces.
//!
//! ## Architecture Overview
//!
//! Axiom is built on a modular architecture:
//! - `compositor`: Core compositor logic and event loop
//! - `workspace`: Scrollable workspace management (niri-inspired)
//! - `window`: Window management and layout algorithms
//! - `input`: Keyboard, mouse, and gesture input handling
//! - `config`: Configuration parsing and management

use anyhow::Result;
use clap::Parser;
use log::{debug, error, info};

use axiom::compositor::AxiomCompositor;
use axiom::config::AxiomConfig;
use axiom::input::InputManager;
use axiom::ipc::AxiomIPCServer;
use axiom::window::WindowManager;
use axiom::workspace::ScrollableWorkspaces;
use parking_lot::RwLock;
use std::sync::Arc;
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

    /// Run in nested/windowed mode (recommended alpha target)
    #[arg(short, long)]
    windowed: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging — CLI flag or config can enable debug.
    // Config is not loaded yet at this point, so we defer a possible
    // re-init below. The CLI flag always takes priority.
    let log_level = if cli.debug { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

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

    // Load configuration (AxiomConfig::load handles ~ expansion)
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

    // Honor config.general.debug (in addition to the CLI flag).
    // `log::set_max_level` works after env_logger has been initialized.
    if config.general.debug {
        log::set_max_level(log::LevelFilter::Debug);
        debug!("Debug logging enabled via config");
    }

    // Initialize and run compositor
    info!("🏗️  Initializing Axiom compositor...");

    // Create shared managers
    #[allow(clippy::arc_with_non_send_sync)]
    let workspace_manager = std::sync::Arc::new(parking_lot::RwLock::new(
        ScrollableWorkspaces::new(&config.workspace),
    ));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));

    let ipc_server = AxiomIPCServer::new();

    let compositor = AxiomCompositor::new(
        config.clone(),
        cli.windowed,
        workspace_manager.clone(),
        window_manager.clone(),
        input_manager.clone(),
        ipc_server,
    )
    .await?;

    info!("✨ Axiom is ready! Where productivity meets beauty.");

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
        let cli = Cli::try_parse_from(["axiom"]).expect("CLI parse should succeed");
        assert!(!cli.debug);
        assert!(!cli.windowed);
    }

    #[test]
    fn test_cli_flags() {
        let cli = Cli::try_parse_from(["axiom", "--debug", "--windowed"])
            .expect("CLI parse should succeed");
        assert!(cli.debug);
        assert!(cli.windowed);
    }
}
