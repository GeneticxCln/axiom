//! Test that we can create a real Wayland socket

use anyhow::Result;

mod backend_simple;
use backend_simple::SimpleBackend;

fn main() -> Result<()> {
    env_logger::init();

    println!("🧪 Testing Wayland Socket Creation");
    println!("===================================");

    let backend = SimpleBackend::new()?;

    println!(
        "\n✅ SUCCESS! Wayland socket created: {}",
        backend.socket_name()
    );
    println!("\n📋 You can verify this works by running:");
    println!("   ls -la /run/user/$(id -u)/{}", backend.socket_name());

    // Keep it alive so we can check
    println!("\nPress Enter to exit...");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    Ok(())
}
