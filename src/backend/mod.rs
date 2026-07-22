//! Smithay 0.7 Backend for Axiom Compositor
//!
//! This module implements the Wayland compositor backend using Smithay 0.7's
//! handler trait pattern. It manages the Wayland display, protocol state,
//! input routing, and GL/WGPU rendering.
//!
//! ## Phase 6 completions
//! - 6.2: Wire toplevel state and window lifecycle
//! - 6.3: Route winit input events through InputManager for global keybindings
//! - 6.4: GL scissor-based window placeholder rendering at correct workspace positions

// Submodules split out of this file for maintainability. Each is a child of
// `backend`, so it can read the private fields of `State` and
// `AxiomSmithayBackendReal` (descendant modules see ancestor privates).
pub mod state;
pub mod winit;
pub mod screencopy;
mod clipboard;
mod input;
mod render;

// Public API re-exports — same as when everything was in mod.rs.
pub use state::State;
pub use state::SurfaceData;
pub use state::PopupState;
pub use state::PendingCapture;
pub use winit::AxiomSmithayBackendReal;
pub use winit::BackendKind;

// Private re-exports so sibling submodules can access items from each other
// via `use super::...`. These bring the names into the `backend` module scope,
// making them visible to all descendant modules.
use state::ClipboardUpdate;
use winit::WindowInteraction;
