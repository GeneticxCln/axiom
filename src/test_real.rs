//! Test REAL Wayland Backend - This will accept real client connections

use anyhow::Result;
use log::info;

mod backend_real;
use backend_real::RealBackend;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("🚀 AXIOM REAL WAYLAND COMPOSITOR TEST");
    info!("=====================================");
    info!("");

    let backend = RealBackend::new()?;

    info!("");
    info!("📋 READY FOR REAL CLIENTS!");
    info!("");
    info!("Test with these commands in another terminal:");
    info!("");
    info!("1️⃣  Check compositor info:");
    info!("   weston-info");
    info!("");
    info!("2️⃣  Run a simple client:");
    info!("   weston-simple-shm");
    info!("");
    info!("3️⃣  Run a terminal:");
    info!("   weston-terminal");
    info!("");
    info!("4️⃣  Run any Wayland app:");
    info!("   GDK_BACKEND=wayland gedit");
    info!("");
    info!("Press Ctrl+C to stop");
    info!("");

    backend.run()?;

    Ok(())
}
