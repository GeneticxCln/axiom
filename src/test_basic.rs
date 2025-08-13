//! Test the basic Wayland backend

use anyhow::Result;

mod backend_basic;
use backend_basic::BasicBackend;

fn main() -> Result<()> {
    env_logger::init();

    println!("🧪 Testing Basic Wayland Backend");
    println!("=================================");

    let mut backend = BasicBackend::new()?;

    println!("\n📋 Instructions:");
    println!("1. Open another terminal");
    println!("2. Run: weston-info");
    println!("3. You should see Axiom's compositor globals listed!");
    println!("\nPress Ctrl+C to stop\n");

    backend.run()?;

    Ok(())
}
