//! # Axiom Wayland Compositor Library
//!
//! The first Wayland compositor combining niri's scrollable workspace innovation
//! with Hyprland's visual effects system.
//!
//! ## Architecture
//!
//! Axiom is built on a modular architecture:
//! - `compositor`: Core compositor logic and event loop
//! - `workspace`: Scrollable workspace management (niri-inspired)
//! - `effects`: Visual effects engine (Hyprland-inspired)
//! - `window`: Window management and layout algorithms
//! - `input`: Keyboard, mouse, and gesture input handling
//! - `config`: Configuration parsing and management
//! - `xwayland`: X11 compatibility layer
//! - `ipc`: IPC communication with Lazy UI
//! - `smithay_backend`: Smithay Wayland compositor integration
//!
//! ## Usage
//!
//! ```rust,no_run
//! use axiom::{AxiomCompositor, AxiomConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = AxiomConfig::default();
//!     let mut compositor = AxiomCompositor::new(config, false).await?;
//!     compositor.run().await
//! }
//! ```

pub mod compositor;
pub mod config;
pub mod effects;
pub mod input;
pub mod ipc;
pub mod smithay_backend;
pub mod smithay_enhanced;  // Enhanced Smithay with Wayland socket support
// TODO: Real Wayland protocol implementation will be integrated when Smithay API is stable
pub mod window;
pub mod workspace;
pub mod xwayland;

// Demo modules for development and testing
#[cfg(any(test, feature = "demo"))]
pub mod demo_phase4_effects;
#[cfg(any(test, feature = "demo"))]
pub mod demo_workspace;

// Re-export main types for easy access
pub use compositor::AxiomCompositor;
pub use config::AxiomConfig;
pub use effects::EffectsEngine;
pub use input::InputManager;
pub use ipc::AxiomIPCServer;
pub use window::WindowManager;
pub use workspace::ScrollableWorkspaces;

// Re-export common error types
pub use anyhow::{Context, Error, Result};

/// Version information for Axiom
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
