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

// Core smithay backend implementations
pub mod smithay_backend;
pub mod smithay_backend_minimal;
pub mod smithay_backend_simple;
pub mod smithay_backend_working;
pub mod smithay_backend_real;
pub mod smithay_backend_real_minimal;
pub mod smithay_backend_production;
pub mod smithay_backend_phase6;
pub mod smithay_backend_phase6_2;
pub mod smithay_enhanced;

// Real compositor components
pub mod axiom_real_compositor;
pub mod real_smithay;
pub mod real_input;
pub mod real_window;
pub mod multi_output;
pub mod wayland_protocols;

// Re-export commonly used types for convenience
pub use smithay_backend::*;
pub use smithay_backend_phase6_2::AxiomSmithayBackendPhase6_2;
pub use axiom_real_compositor::*;
pub use real_smithay::*;
