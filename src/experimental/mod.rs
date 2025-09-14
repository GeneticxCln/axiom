//! # Experimental Features Module
//!
//! This module contains experimental implementations and features that are
//! gated behind feature flags to prevent them from affecting regular builds
//! and tests.
//!
//! ## Available Experimental Features
//!
//! - `smithay`: Experimental Smithay-based Wayland compositor backends

// Enable smithay module if either experimental-smithay or smithay-minimal is on
#[cfg(any(feature = "experimental-smithay", feature = "smithay-minimal"))]
pub mod smithay;
