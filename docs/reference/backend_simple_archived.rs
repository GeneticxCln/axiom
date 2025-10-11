//! Simplest Possible Real Wayland Compositor
//!
//! This creates a real Wayland socket that clients can connect to.

use anyhow::{Context, Result};
use log::info;

use wayland_server::{Display, ListeningSocket};

/// Simplest backend - just creates a socket
pub struct SimpleBackend {
    socket_name: String,
    _display: Display<()>,
}

impl SimpleBackend {
    pub fn new() -> Result<Self> {
        info!("🚀 Creating simple Wayland backend...");

        // Create display
        let display: Display<()> = Display::new()?;

        // Create listening socket
        let listening_socket = ListeningSocket::bind_auto("wayland", 0..10)
            .context("Failed to bind Wayland socket")?;

        // Get the socket name
        let socket_name = listening_socket
            .socket_name()
            .ok_or_else(|| anyhow::anyhow!("Failed to get socket name"))?
            .to_string_lossy()
            .to_string();

        info!("✅ Wayland socket created: {}", socket_name);
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);

        // Add the socket to the display
        listening_socket.accept()?;

        Ok(Self {
            socket_name,
            _display: display,
        })
    }

    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }
}
