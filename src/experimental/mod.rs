//! # Experimental Features Module
//!
//! This module contains experimental implementations and features that are
//! gated behind feature flags to prevent them from affecting regular builds
//! and tests.
//!
//! ## Available Experimental Features
//!
//! - `smithay`: Experimental Smithay-based Wayland compositor backends

#[cfg(feature = "experimental-smithay")]
pub mod smithay;
