//! XDG-Shell Protocol Validation and State Machine Tracking
//!
//! This module implements comprehensive validation for the xdg-shell protocol,
//! tracking surface state machines, configure/ack sequences, and detecting
//! protocol violations that should result in protocol errors.
//!
//! # Protocol State Machine
//!
//! XDG surfaces go through well-defined states:
//! 1. **Created**: xdg_surface created but no role assigned yet
//! 2. **Configured**: Initial configure sent, awaiting ack
//! 3. **Mapped**: First buffer attached after ack_configure
//! 4. **Active**: Surface is actively displayed and receiving updates
//! 5. **Unmapped**: Surface hidden or destroyed
//!
//! # Configure Sequence Validation
//!
//! The protocol requires:
//! - Compositor sends configure with serial N
//! - Client must ack_configure with serial N before committing buffer
//! - Client cannot commit buffer before acking at least one configure
//! - Ack must match a previously sent configure serial

use log::{debug, error, warn};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Maximum number of pending configure events to track per surface
const MAX_PENDING_CONFIGURES: usize = 32;

/// Timeout for configure acknowledgment (client should respond within this time)
const CONFIGURE_ACK_TIMEOUT_SECS: u64 = 5;

/// Surface state in the xdg-shell protocol state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XdgSurfaceState {
    /// Surface created but no role assigned (toplevel/popup)
    Created,
    /// Role assigned, initial configure sent, awaiting ack
    WaitingForAck,
    /// Configure acknowledged but no buffer committed yet
    Configured,
    /// First buffer committed, surface is mapped and visible
    Mapped,
    /// Surface has been unmapped (hidden)
    Unmapped,
}

/// Type of XDG surface role
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XdgRole {
    /// No role assigned yet
    None,
    /// Toplevel window (xdg_toplevel)
    Toplevel,
    /// Popup menu/tooltip (xdg_popup)
    Popup,
}

/// A pending configure event awaiting acknowledgment
#[derive(Debug, Clone)]
pub struct PendingConfigure {
    /// Serial number of this configure
    pub serial: u32,
    /// When this configure was sent
    pub sent_at: Instant,
    /// Width suggested in configure (0 = client decides)
    pub width: u32,
    /// Height suggested in configure (0 = client decides)
    pub height: u32,
    /// Whether this configure has been acknowledged
    pub acknowledged: bool,
}

/// Validation state for a single xdg_surface
#[derive(Debug, Clone)]
pub struct XdgSurfaceValidation {
    /// Surface resource ID
    pub surface_id: u32,
    /// Current state in the state machine
    pub state: XdgSurfaceState,
    /// Assigned role (toplevel/popup/none)
    pub role: XdgRole,
    /// Queue of pending configure events
    pub pending_configures: VecDeque<PendingConfigure>,
    /// Last acknowledged configure serial
    pub last_acked_serial: Option<u32>,
    /// Number of commits since creation
    pub commit_count: u64,
    /// Whether this surface has ever been mapped
    pub ever_mapped: bool,
    /// Last state transition time (for timeout detection)
    pub last_state_change: Instant,
}

impl XdgSurfaceValidation {
    /// Creates a new validation state for a surface
    pub fn new(surface_id: u32) -> Self {
        Self {
            surface_id,
            state: XdgSurfaceState::Created,
            role: XdgRole::None,
            pending_configures: VecDeque::new(),
            last_acked_serial: None,
            commit_count: 0,
            ever_mapped: false,
            last_state_change: Instant::now(),
        }
    }

    /// Records that a role was assigned to this surface
    pub fn assign_role(&mut self, role: XdgRole) -> Result<(), ProtocolError> {
        if self.role != XdgRole::None && self.role != role {
            return Err(ProtocolError::RoleAlreadyAssigned {
                surface_id: self.surface_id,
                existing_role: self.role,
                new_role: role,
            });
        }
        
        debug!("Surface {} assigned role {:?}", self.surface_id, role);
        self.role = role;
        Ok(())
    }

    /// Records that a configure event was sent to the client
    pub fn add_configure(&mut self, serial: u32, width: u32, height: u32) -> Result<(), ProtocolError> {
        // Check if we're tracking too many pending configures
        if self.pending_configures.len() >= MAX_PENDING_CONFIGURES {
            warn!(
                "Surface {} has {} pending configures, dropping oldest",
                self.surface_id,
                self.pending_configures.len()
            );
            self.pending_configures.pop_front();
        }

        let configure = PendingConfigure {
            serial,
            sent_at: Instant::now(),
            width,
            height,
            acknowledged: false,
        };

        self.pending_configures.push_back(configure);
        
        // Transition to WaitingForAck if this is the first configure
        if self.state == XdgSurfaceState::Created {
            self.transition_state(XdgSurfaceState::WaitingForAck);
        }

        debug!(
            "Surface {} configure sent: serial={}, size={}x{}, pending_count={}",
            self.surface_id, serial, width, height,
            self.pending_configures.len()
        );

        Ok(())
    }

    /// Validates and records an ack_configure from the client
    pub fn ack_configure(&mut self, serial: u32) -> Result<(), ProtocolError> {
        // Find the configure with this serial
        let found = self.pending_configures
            .iter_mut()
            .find(|c| c.serial == serial);

        match found {
            Some(configure) => {
                if configure.acknowledged {
                    warn!(
                        "Surface {} acking already-acknowledged configure serial {}",
                        self.surface_id, serial
                    );
                }

                configure.acknowledged = true;
                self.last_acked_serial = Some(serial);

                // Transition to Configured if we were waiting
                if self.state == XdgSurfaceState::WaitingForAck {
                    self.transition_state(XdgSurfaceState::Configured);
                }

                // Clean up old acknowledged configures
                self.cleanup_old_configures();

                debug!(
                    "Surface {} acknowledged configure serial {}",
                    self.surface_id, serial
                );

                Ok(())
            }
            None => {
                error!(
                    "Surface {} acked unknown serial {} (pending: {:?})",
                    self.surface_id,
                    serial,
                    self.pending_configures.iter().map(|c| c.serial).collect::<Vec<_>>()
                );
                
                Err(ProtocolError::InvalidAckSerial {
                    surface_id: self.surface_id,
                    acked_serial: serial,
                    pending_serials: self.pending_configures
                        .iter()
                        .map(|c| c.serial)
                        .collect(),
                })
            }
        }
    }

    /// Validates that a commit is allowed in the current state
    pub fn validate_commit(&mut self, has_buffer: bool) -> Result<(), ProtocolError> {
        self.commit_count += 1;

        // Must have acknowledged at least one configure before committing a buffer
        if has_buffer && self.last_acked_serial.is_none() {
            return Err(ProtocolError::BufferBeforeAck {
                surface_id: self.surface_id,
                commit_count: self.commit_count,
            });
        }

        // If committing a buffer after ack, transition to mapped
        if has_buffer && self.state == XdgSurfaceState::Configured {
            self.transition_state(XdgSurfaceState::Mapped);
            self.ever_mapped = true;
            debug!("Surface {} is now mapped", self.surface_id);
        }

        // If committing null buffer, transition to unmapped
        if !has_buffer && self.state == XdgSurfaceState::Mapped {
            self.transition_state(XdgSurfaceState::Unmapped);
            debug!("Surface {} unmapped (null buffer commit)", self.surface_id);
        }

        Ok(())
    }

    /// Checks for configure timeout violations
    pub fn check_timeouts(&self) -> Vec<ProtocolWarning> {
        let mut warnings = Vec::new();
        let now = Instant::now();

        for configure in &self.pending_configures {
            if !configure.acknowledged {
                let elapsed = now.duration_since(configure.sent_at).as_secs();
                if elapsed > CONFIGURE_ACK_TIMEOUT_SECS {
                    warnings.push(ProtocolWarning::ConfigureAckTimeout {
                        surface_id: self.surface_id,
                        serial: configure.serial,
                        elapsed_secs: elapsed,
                    });
                }
            }
        }

        warnings
    }

    /// Transitions to a new state
    fn transition_state(&mut self, new_state: XdgSurfaceState) {
        if self.state != new_state {
            debug!(
                "Surface {} state transition: {:?} -> {:?}",
                self.surface_id, self.state, new_state
            );
            self.state = new_state;
            self.last_state_change = Instant::now();
        }
    }

    /// Removes old acknowledged configures, keeping only recent ones
    fn cleanup_old_configures(&mut self) {
        // Keep the last acknowledged and any newer unacknowledged ones
        if let Some(last_idx) = self.pending_configures
            .iter()
            .rposition(|c| c.acknowledged)
        {
            // Remove everything before the last acknowledged
            if last_idx > 0 {
                self.pending_configures.drain(0..last_idx);
            }
        }
    }
}

/// Protocol error types for xdg-shell violations
#[derive(Debug, Clone)]
pub enum ProtocolError {
    /// Client tried to assign a role when one was already assigned
    RoleAlreadyAssigned {
        surface_id: u32,
        existing_role: XdgRole,
        new_role: XdgRole,
    },
    /// Client acked a serial that was never sent
    InvalidAckSerial {
        surface_id: u32,
        acked_serial: u32,
        pending_serials: Vec<u32>,
    },
    /// Client committed a buffer before acknowledging any configure
    BufferBeforeAck {
        surface_id: u32,
        commit_count: u64,
    },
    /// Popup positioner validation failed
    InvalidPositioner {
        surface_id: u32,
        reason: String,
    },
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::RoleAlreadyAssigned { surface_id, existing_role, new_role } => {
                write!(
                    f,
                    "Surface {} already has role {:?}, cannot assign {:?}",
                    surface_id, existing_role, new_role
                )
            }
            ProtocolError::InvalidAckSerial { surface_id, acked_serial, pending_serials } => {
                write!(
                    f,
                    "Surface {} acked unknown serial {} (pending: {:?})",
                    surface_id, acked_serial, pending_serials
                )
            }
            ProtocolError::BufferBeforeAck { surface_id, commit_count } => {
                write!(
                    f,
                    "Surface {} committed buffer before ack_configure (commit #{})",
                    surface_id, commit_count
                )
            }
            ProtocolError::InvalidPositioner { surface_id, reason } => {
                write!(
                    f,
                    "Surface {} has invalid positioner: {}",
                    surface_id, reason
                )
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

/// Protocol warnings (non-fatal but suspicious behavior)
#[derive(Debug, Clone)]
pub enum ProtocolWarning {
    /// Configure acknowledgment is taking too long
    ConfigureAckTimeout {
        surface_id: u32,
        serial: u32,
        elapsed_secs: u64,
    },
    /// Too many pending configures
    TooManyPendingConfigures {
        surface_id: u32,
        count: usize,
    },
}

/// Validator for all xdg_surface instances in the compositor
#[derive(Debug)]
pub struct XdgShellValidator {
    /// Validation state per surface ID
    surfaces: HashMap<u32, XdgSurfaceValidation>,
    /// Total protocol errors detected
    error_count: u64,
    /// Total warnings issued
    warning_count: u64,
}

impl XdgShellValidator {
    /// Creates a new validator
    pub fn new() -> Self {
        Self {
            surfaces: HashMap::new(),
            error_count: 0,
            warning_count: 0,
        }
    }

    /// Registers a new xdg_surface
    pub fn register_surface(&mut self, surface_id: u32) {
        debug!("Registered xdg_surface {}", surface_id);
        self.surfaces.insert(surface_id, XdgSurfaceValidation::new(surface_id));
    }

    /// Removes a surface from tracking
    pub fn unregister_surface(&mut self, surface_id: u32) {
        if let Some(validation) = self.surfaces.remove(&surface_id) {
            debug!(
                "Unregistered xdg_surface {} (state: {:?}, commits: {})",
                surface_id, validation.state, validation.commit_count
            );
        }
    }

    /// Assigns a role to a surface
    pub fn assign_role(&mut self, surface_id: u32, role: XdgRole) -> Result<(), ProtocolError> {
        if let Some(validation) = self.surfaces.get_mut(&surface_id) {
            validation.assign_role(role)?;
        }
        Ok(())
    }

    /// Records a configure event
    pub fn add_configure(
        &mut self,
        surface_id: u32,
        serial: u32,
        width: u32,
        height: u32,
    ) -> Result<(), ProtocolError> {
        if let Some(validation) = self.surfaces.get_mut(&surface_id) {
            validation.add_configure(serial, width, height)?;
        }
        Ok(())
    }

    /// Validates and records an ack_configure
    pub fn ack_configure(&mut self, surface_id: u32, serial: u32) -> Result<(), ProtocolError> {
        match self.surfaces.get_mut(&surface_id) {
            Some(validation) => {
                validation.ack_configure(serial)?;
                Ok(())
            }
            None => {
                error!("ack_configure for unknown surface {}", surface_id);
                self.error_count += 1;
                Err(ProtocolError::InvalidAckSerial {
                    surface_id,
                    acked_serial: serial,
                    pending_serials: vec![],
                })
            }
        }
    }

    /// Validates a commit operation
    pub fn validate_commit(
        &mut self,
        surface_id: u32,
        has_buffer: bool,
    ) -> Result<(), ProtocolError> {
        match self.surfaces.get_mut(&surface_id) {
            Some(validation) => {
                match validation.validate_commit(has_buffer) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        self.error_count += 1;
                        error!("Commit validation failed: {}", e);
                        Err(e)
                    }
                }
            }
            None => {
                warn!("Commit for untracked xdg_surface {}", surface_id);
                Ok(())
            }
        }
    }

    /// Checks all surfaces for timeout violations
    pub fn check_timeouts(&mut self) -> Vec<ProtocolWarning> {
        let mut all_warnings = Vec::new();

        for validation in self.surfaces.values() {
            let warnings = validation.check_timeouts();
            self.warning_count += warnings.len() as u64;
            all_warnings.extend(warnings);
        }

        all_warnings
    }

    /// Gets statistics about validation state
    pub fn stats(&self) -> ValidatorStats {
        ValidatorStats {
            tracked_surfaces: self.surfaces.len(),
            total_errors: self.error_count,
            total_warnings: self.warning_count,
            mapped_surfaces: self.surfaces.values()
                .filter(|v| v.state == XdgSurfaceState::Mapped)
                .count(),
        }
    }

    /// Gets the validation state for a specific surface
    pub fn get_surface_state(&self, surface_id: u32) -> Option<&XdgSurfaceValidation> {
        self.surfaces.get(&surface_id)
    }
}

impl Default for XdgShellValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the validator state
#[derive(Debug, Clone)]
pub struct ValidatorStats {
    pub tracked_surfaces: usize,
    pub total_errors: u64,
    pub total_warnings: u64,
    pub mapped_surfaces: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_configure_ack_flow() {
        let mut validator = XdgShellValidator::new();
        let surface_id = 1;

        // Register surface
        validator.register_surface(surface_id);
        
        // Assign toplevel role
        validator.assign_role(surface_id, XdgRole::Toplevel).unwrap();

        // Send initial configure
        validator.add_configure(surface_id, 100, 800, 600).unwrap();

        // Client acks
        validator.ack_configure(surface_id, 100).unwrap();

        // Client commits with buffer
        validator.validate_commit(surface_id, true).unwrap();

        let state = validator.get_surface_state(surface_id).unwrap();
        assert_eq!(state.state, XdgSurfaceState::Mapped);
        assert!(state.ever_mapped);
    }

    #[test]
    fn test_buffer_before_ack_error() {
        let mut validator = XdgShellValidator::new();
        let surface_id = 1;

        validator.register_surface(surface_id);
        validator.assign_role(surface_id, XdgRole::Toplevel).unwrap();
        validator.add_configure(surface_id, 100, 800, 600).unwrap();

        // Try to commit buffer without ack - should fail
        let result = validator.validate_commit(surface_id, true);
        assert!(result.is_err());
        assert_eq!(validator.stats().total_errors, 1);
    }

    #[test]
    fn test_invalid_ack_serial() {
        let mut validator = XdgShellValidator::new();
        let surface_id = 1;

        validator.register_surface(surface_id);
        validator.assign_role(surface_id, XdgRole::Toplevel).unwrap();
        validator.add_configure(surface_id, 100, 800, 600).unwrap();

        // Ack wrong serial - should fail
        let result = validator.ack_configure(surface_id, 999);
        assert!(result.is_err());
    }

    #[test]
    fn test_role_already_assigned() {
        let mut validator = XdgShellValidator::new();
        let surface_id = 1;

        validator.register_surface(surface_id);
        validator.assign_role(surface_id, XdgRole::Toplevel).unwrap();

        // Try to assign popup role - should fail
        let result = validator.assign_role(surface_id, XdgRole::Popup);
        assert!(result.is_err());
    }

    #[test]
    fn test_unmapping() {
        let mut validator = XdgShellValidator::new();
        let surface_id = 1;

        validator.register_surface(surface_id);
        validator.assign_role(surface_id, XdgRole::Toplevel).unwrap();
        validator.add_configure(surface_id, 100, 800, 600).unwrap();
        validator.ack_configure(surface_id, 100).unwrap();
        validator.validate_commit(surface_id, true).unwrap();

        // Commit null buffer to unmap
        validator.validate_commit(surface_id, false).unwrap();

        let state = validator.get_surface_state(surface_id).unwrap();
        assert_eq!(state.state, XdgSurfaceState::Unmapped);
    }
}
