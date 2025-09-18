//! Minimal Wayland server entrypoint (smithay-minimal)
//! Starts a bare server that accepts clients and prints WAYLAND_DISPLAY.

use anyhow::Result;
use log::info;
use parking_lot::RwLock;
use std::sync::Arc;

// Import reexports from the axiom library crate
use axiom::clipboard::ClipboardManager;
#[cfg(feature = "smithay")]
use axiom::smithay::server::CompositorServer;
use axiom::{AxiomConfig, InputManager, ScrollableWorkspaces, WindowManager};

fn main() -> Result<()> {
    // Initialize logging (best-effort)
    let _ = env_logger::try_init();

    // Use default configuration for minimal server
    let config = AxiomConfig::default();

    // Initialize core managers
    let wm = Arc::new(RwLock::new(WindowManager::new(&config.window)?));
    let ws = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)?));
    let im = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )?));

    // Create compositor server without on-screen presenter or headless renderer
    // Respect config.window.force_client_side_decorations via env for the server
    if config.window.force_client_side_decorations {
        std::env::set_var("AXIOM_FORCE_CSD", "1");
    }
    let clip = Arc::new(RwLock::new(ClipboardManager::new()));
    let deco = Arc::new(RwLock::new(axiom::DecorationManager::new(&config.window)));
    let server = CompositorServer::new(
        wm,
        ws,
        im,
        clip,
        deco,
        /* present_rx */ None,
        /* size_rx */ None,
        /* redraw_tx */ None,
        /* input_rx_ext */ None,
        /* spawn_headless_renderer: */ false,
        wgpu::Backends::all(),
        /* outputs_init */ None,
        /* outputs_rx */ None,
    )?;

    info!("Starting minimal Wayland server (smithay-minimal)");
    // Run until interrupted; prints WAYLAND_DISPLAY on startup
    server.run().map(|_| ())
}
