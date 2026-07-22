//! Axiom Wayland Compositor Library
//!
//! A winit-only Wayland compositor with scrollable workspaces and server-side
//! decorations rendered via GLES. Built on [Smithay 0.7](https://github.com/Smithay/smithay).
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────┐    ┌──────────────────────┐    ┌─────────────────┐
//! │   Lazy UI       │◄──►│   Axiom Compositor   │◄──►│  Wayland Apps   │
//! │   (IPC Client)  │    │    (Main Process)    │    │  (Clients)      │
//! └─────────────────┘    └──────────────────────┘    └─────────────────┘
//!                                 │
//!                    ┌────────────┼────────────┐
//!                    ▼            ▼            ▼
//!              ┌──────────┐ ┌──────────┐ ┌──────────┐
//!              │ Workspace│ │  Window  │ │  Input   │
//!              │  Engine  │ │ Manager  │ │ Handler  │
//!              └──────────┘ └──────────┘ └──────────┘
//!                    │            │            │
//!                    └────────────┼────────────┘
//!                                 ▼
//!                        ┌──────────────────┐
//!                        │ Smithay 0.7      │
//!                        │ (GLES + Wayland) │
//!                        └──────────────────┘
//!                                 │
//!                                 ▼
//!                        ┌──────────────────┐
//!                        │   Winit Window   │
//!                        │  (nested mode)   │
//!                        └──────────────────┘
//! ```
//!
//! ## Core modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`compositor`] | Top-level orchestration, event loop, tick scheduling |
//! | [`backend`] | Smithay backend: winit surface, GLES rendering, input/event dispatch, clipboard |
//! | [`workspace`] | Scrollable workspace tape model (niri-inspired) |
//! | [`window`] | Window registry, lifecycle, tiling/floating layout |
//! | [`input`] | Keybindings, action dispatch, compositor shortcuts |
//! | [`ipc`] | Unix-socket JSON IPC protocol and server |
//! | [`config`] | TOML configuration model, loading, and validation |
//! | [`decoration`] | Server-side decoration geometry and hit-testing |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use axiom::config::AxiomConfig;
//! use axiom::AxiomCompositor;
//!
//! fn main() -> anyhow::Result<()> {
//!     let config = AxiomConfig::default();
//!     let compositor = AxiomCompositor::new(config, false)?;
//!     compositor.run()?;
//!     Ok(())
//! }
//! ```
//!
//! See [`main.rs`](https://github.com/GeneticxCln/axiom/blob/main/src/main.rs)
//! for the full CLI initialization path.

#![warn(rust_2018_idioms)]

// Re-export main compositor
pub use crate::compositor::AxiomCompositor;

// Re-export configuration
pub use crate::config::AxiomConfig;

// Re-export core modules
pub use crate::decoration::{DecorationManager, DecorationMode};
pub use crate::input::InputManager;
pub use crate::window::{Rectangle, WindowManager};
pub use crate::workspace::ScrollableWorkspaces;

// Module declarations
pub mod compositor;
pub mod config;
pub mod decoration;
pub mod input;
pub mod ipc;
pub mod window;
pub mod workspace;

pub mod backend;

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build information
pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: VERSION,
    git_commit: option_env!("GIT_COMMIT"),
    build_date: match option_env!("BUILD_DATE") {
        Some(s) => s,
        None => "unknown",
    },
    target_triple: match option_env!("TARGET_TRIPLE") {
        Some(s) => s,
        None => "unknown",
    },
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
            Some(commit) => format!("{} ({})", self.version, commit.get(..8).unwrap_or(commit)),
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
