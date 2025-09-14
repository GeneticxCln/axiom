//! # Experimental Smithay Backend Implementations
//!
//! This module contains experimental implementations of Smithay-based Wayland compositor
//! backends for Axiom. These backends are gated behind the `experimental-smithay` feature
//! flag to prevent them from affecting regular builds and tests.
//!
//! ## Available Backends
//!
//! - `smithay_backend`: Basic Smithay backend implementation
//! - `smithay_backend_minimal`: Minimal working backend for testing
//! - `smithay_backend_simple`: Simplified backend for development
//! - `smithay_backend_working`: Production-ready working backend
//! - `smithay_backend_real`: Real backend with full protocol support
//! - `smithay_backend_real_minimal`: Minimal real backend implementation
//! - `smithay_backend_production`: Full production backend with all features
//! - `smithay_backend_phase6`: Phase 6 development backend
//! - `smithay_backend_phase6_2`: Enhanced Phase 6 backend with protocol simulation
//! - `smithay_enhanced`: Enhanced Smithay backend with additional features
//!
//! ## Real Compositor Components
//!
//! - `axiom_real_compositor`: Complete real compositor implementation
//! - `real_smithay`: Core Smithay integration
//! - `real_input`: Real input handling
//! - `real_window`: Real window management
//! - `multi_output`: Multi-monitor support
//! - `wayland_protocols`: Extended Wayland protocol implementations

// Protocol helpers are only available with the full experimental Smithay feature
#[cfg(feature = "experimental-smithay")]
pub mod wayland_protocols;

// Only compile the minimal backend in the safe subset feature
#[cfg(feature = "smithay-minimal")]
pub mod minimal_server;

#[cfg(feature = "smithay-minimal")]
pub use minimal_server::*;

// The rest of these backends are experimental and often non-compiling.
// They remain available only under the broad experimental-smithay feature.
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_minimal;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_simple;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_working;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_real;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_production;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_phase6;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_backend_phase6_2;
#[cfg(all(feature = "experimental-smithay", not(feature = "smithay-minimal")))]
pub mod smithay_enhanced;

// Minimal, conservative re-exports handled above for smithay-minimal
