use anyhow::Result;
use axiom::backend_real::RealBackend;
use log::info;

fn main() -> Result<()> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("🚀 Starting Axiom Real Backend...");
    info!("📄 Running standalone Wayland compositor backend");

    // Create and run the real backend
    let backend = RealBackend::new()?;
    let socket_name = backend.socket_name().to_string();
    
    info!("✅ Backend created, socket: {}", socket_name);
    info!("💡 Set WAYLAND_DISPLAY={} for clients to connect", socket_name);
    
    // Run the backend event loop
    backend.run()?;

    Ok(())
}