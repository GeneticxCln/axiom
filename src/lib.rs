//! Axiom Wayland Compositor Library
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::return_self_not_must_use)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::unused_self)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::float_cmp)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::single_match_else)]
//!
//! A hybrid Wayland compositor combining scrollable workspaces with beautiful visual effects.
//! This library exposes the core functionality for building Wayland compositors with:
//!
//! - **Scrollable Workspaces**: Smooth infinite scrolling between workspace columns
//! - **Visual Effects**: Blur, shadows, animations, and custom shaders
//! - **Window Management**: Intelligent tiling and floating window support
//! - **Input Handling**: Comprehensive keyboard and mouse input processing
//! - **IPC Communication**: Integration with Lazy UI and external tools
//! - **Smithay Integration**: Full Wayland compositor protocol support
//!
//! ## Architecture
//!
//! The compositor is built with a modular architecture:
//!
//! ```text
//! ┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
//! │   Lazy UI       │◄──►│  Axiom Compositor│◄──►│  Wayland Apps   │
//! │   (External)    │    │   (Main Process) │    │  (Clients)      │
//! └─────────────────┘    └──────────────────┘    └─────────────────┘
//!                                 │
//!                                 ▼
//!                        ┌──────────────────┐
//!                        │ Smithay Backend  │
//!                        │ (Wayland Server) │
//!                        └──────────────────┘
//! ```
//!
//! ## Features
//!
//! - **Phase-based Development**: Gradual implementation from basic functionality to full compositor
//! - **Multiple Backends**: Support for both development (windowed) and production (DRM) modes
//! - **Effects Pipeline**: GPU-accelerated visual effects with automatic performance scaling
//! - **Protocol Extensions**: Enhanced Wayland protocols for advanced window management
//!
//! ## Usage
//!
//! ```rust,ignore
//! use axiom::{AxiomCompositor, config::AxiomConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = AxiomConfig::load("config/axiom.toml")?;
//!     // Compositor requires pre-initialized subsystems
//!     // See main.rs for full initialization example
//!     Ok(())
//! }
//! ```

#![warn(missing_docs, rust_2018_idioms)]
#![allow(dead_code)] // Remove this once development is complete

// Re-export main compositor
pub use crate::compositor::AxiomCompositor;

// Re-export configuration
pub use crate::config::AxiomConfig;

// Re-export core modules
pub use crate::decoration::{DecorationManager, DecorationMode};
pub use crate::effects::EffectsEngine;
pub use crate::input::InputManager;
pub use crate::window::{Rectangle, WindowManager};
pub use crate::workspace::ScrollableWorkspaces;

// Module declarations
pub mod compositor;
pub mod config;
pub mod decoration;
pub mod effects;
pub mod input;
pub mod ipc;
pub mod renderer;
pub mod window;
pub mod workspace;
pub mod xwayland;

// Experimental features module
// pub mod experimental;
pub mod backend;
pub mod demo_phase4_effects;
pub mod demo_workspace;

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build information
pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: VERSION,
    git_commit: option_env!("GIT_COMMIT"),
    build_date: env!("BUILD_DATE"),
    target_triple: env!("TARGET_TRIPLE"),
};

/// Build information structure
pub struct BuildInfo {
    /// Crate version
    pub version: &'static str,
    /// Git commit hash (if available)
    pub git_commit: Option<&'static str>,
    /// Build date
    pub build_date: &'static str,
    /// Target triple
    pub target_triple: &'static str,
}

impl BuildInfo {
    /// Get formatted version string
    #[must_use]
    pub fn version_string(&self) -> String {
        match self.git_commit {
            Some(commit) => format!("{} ({})", self.version, &commit[..8]),
            None => self.version.to_string(),
        }
    }

    /// Get full build information
    #[must_use]
    pub fn full_info(&self) -> String {
        format!(
            "Axiom Compositor {} built on {} for {}",
            self.version_string(),
            self.build_date,
            self.target_triple
        )
    }
}

/// Result type alias for convenience
pub type Result<T> = anyhow::Result<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_info() {
        assert!(!VERSION.is_empty());
        assert!(!BUILD_INFO.version.is_empty());
        assert!(!BUILD_INFO.build_date.is_empty());
        assert!(!BUILD_INFO.target_triple.is_empty());
    }

    #[test]
    fn test_build_info_formatting() {
        let version_str = BUILD_INFO.version_string();
        assert!(version_str.contains(VERSION));

        let full_info = BUILD_INFO.full_info();
        assert!(full_info.contains("Axiom Compositor"));
        assert!(full_info.contains(VERSION));
    }
}
