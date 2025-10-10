//! Wayland Protocol Validation and Robustness
//!
//! This module provides comprehensive protocol validation, error detection,
//! and robustness features for the Axiom Wayland compositor.
//!
//! # Overview
//!
//! Protocol validation ensures that Wayland clients follow the protocol
//! specification correctly. This includes:
//!
//! - **XDG-Shell validation**: Configure/ack sequences, surface state machines
//! - **Buffer lifecycle**: Proper attachment, commit, and release semantics  
//! - **Protocol error detection**: Catching violations that should disconnect clients
//! - **Timeout monitoring**: Detecting unresponsive or misbehaving clients
//!
//! # Usage
//!
//! ```no_run
//! use axiom::protocol::xdg_shell_validation::{XdgShellValidator, XdgRole};
//!
//! let mut validator = XdgShellValidator::new();
//!
//! // Register new surface
//! validator.register_surface(surface_id);
//!
//! // Assign role
//! validator.assign_role(surface_id, XdgRole::Toplevel).unwrap();
//!
//! // Track configure/ack sequence
//! validator.add_configure(surface_id, serial, width, height).unwrap();
//! validator.ack_configure(surface_id, serial).unwrap();
//!
//! // Validate commits
//! validator.validate_commit(surface_id, has_buffer).unwrap();
//!
//! // Check for timeouts periodically
//! let warnings = validator.check_timeouts();
//! for warning in warnings {
//!     eprintln!("Protocol warning: {:?}", warning);
//! }
//! ```

pub mod xdg_shell_validation;

// Re-export commonly used types
pub use xdg_shell_validation::{
    ProtocolError, ProtocolWarning, ValidatorStats, XdgRole, XdgShellValidator,
    XdgSurfaceState, XdgSurfaceValidation,
};
