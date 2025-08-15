//! Test program for the minimal real Smithay backend
//!
//! This is a standalone test that focuses on getting real Wayland functionality working.

use anyhow::Result;
use log::info;

mod smithay_backend_real_minimal;
use smithay_backend_real_minimal::MinimalRealBackend;

fn main() -> Result<()> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("🧪 Testing Minimal Real Wayland Backend");
    info!("=====================================");

    // Create and initialize the backend
    let mut backend = MinimalRealBackend::new()?;
    backend.initialize()?;

    info!("✅ Backend initialized successfully!");
    info!("");
    info!("📋 To test the compositor:");
    info!("   1. Open another terminal");
    info!("   2. Run: WAYLAND_DISPLAY=wayland-1 weston-terminal");
    info!("   3. Or: WAYLAND_DISPLAY=wayland-1 weston-simple-shm");
    info!("");
    info!("🎬 Starting event loop...");

    // Run the compositor
    backend.run()?;

    Ok(())
}
