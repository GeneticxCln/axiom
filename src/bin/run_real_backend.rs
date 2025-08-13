//! Test the REAL Wayland backend - this will accept actual client connections!

use anyhow::Result;
use log::info;

// Import the real backend from the library
extern crate axiom;
use axiom::backend_real::RealBackend;

fn main() -> Result<()> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("🚀 Starting REAL Axiom Wayland Compositor Backend Test");
    info!("📋 This backend can accept REAL Wayland client connections!");

    // Create and run the real backend
    let backend = RealBackend::new()?;

    info!("✅ Backend created successfully!");
    info!("🔌 Wayland socket: {}", backend.socket_name());
    info!("");
    info!("📝 To test with a real application, run in another terminal:");
    info!(
        "   WAYLAND_DISPLAY={} weston-terminal",
        backend.socket_name()
    );
    info!("");
    info!("⌨️  Press Ctrl+C to stop the compositor");

    // Run the event loop - this will handle real client connections!
    backend.run()?;

    Ok(())
}
