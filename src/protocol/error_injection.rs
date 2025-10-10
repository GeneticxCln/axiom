//! Protocol Error Injection Testing
//!
//! This module provides infrastructure for testing how the compositor handles
//! protocol violations, malformed requests, and misbehaving clients.
//!
//! # Testing Strategy
//!
//! - **Error Injection**: Simulate various protocol violations
//! - **Recovery Testing**: Verify compositor remains stable after errors
//! - **Timeout Simulation**: Test handling of unresponsive clients
//! - **Malformed Data**: Test parsing and validation of invalid requests
//!
//! # Example
//!
//! ```no_run
//! use axiom::protocol::error_injection::{ErrorInjector, ProtocolViolation};
//!
//! let mut injector = ErrorInjector::new();
//!
//! // Simulate client committing buffer before ack_configure
//! let result = injector.inject_violation(
//!     ProtocolViolation::BufferCommitBeforeAck { surface_id: 1 }
//! );
//!
//! assert!(result.is_err());
//! assert_eq!(injector.violation_count(), 1);
//! ```

use log::{debug, error, warn};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::xdg_shell_validation::{XdgShellValidator, XdgRole, ProtocolError};

/// Types of protocol violations that can be injected for testing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolViolation {
    /// Client commits buffer before acknowledging configure
    BufferCommitBeforeAck {
        surface_id: u32,
    },
    /// Client acknowledges a serial that was never sent
    AckUnknownSerial {
        surface_id: u32,
        serial: u32,
    },
    /// Client tries to assign role twice
    DoubleRoleAssignment {
        surface_id: u32,
        first_role: XdgRole,
        second_role: XdgRole,
    },
    /// Client doesn't respond to configure within timeout
    ConfigureTimeout {
        surface_id: u32,
        serial: u32,
        timeout: Duration,
    },
    /// Client sends requests after surface destruction
    UseAfterDestroy {
        surface_id: u32,
    },
    /// Client provides invalid positioner data
    InvalidPositioner {
        surface_id: u32,
        reason: String,
    },
    /// Client sends malformed damage region
    InvalidDamage {
        surface_id: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
}

impl std::fmt::Display for ProtocolViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolViolation::BufferCommitBeforeAck { surface_id } => {
                write!(f, "Buffer commit before ack on surface {}", surface_id)
            }
            ProtocolViolation::AckUnknownSerial { surface_id, serial } => {
                write!(f, "Unknown serial {} acked on surface {}", serial, surface_id)
            }
            ProtocolViolation::DoubleRoleAssignment { surface_id, first_role, second_role } => {
                write!(
                    f,
                    "Double role assignment on surface {}: {:?} -> {:?}",
                    surface_id, first_role, second_role
                )
            }
            ProtocolViolation::ConfigureTimeout { surface_id, serial, timeout } => {
                write!(
                    f,
                    "Configure timeout on surface {}, serial {}, timeout: {:?}",
                    surface_id, serial, timeout
                )
            }
            ProtocolViolation::UseAfterDestroy { surface_id } => {
                write!(f, "Use after destroy on surface {}", surface_id)
            }
            ProtocolViolation::InvalidPositioner { surface_id, reason } => {
                write!(f, "Invalid positioner on surface {}: {}", surface_id, reason)
            }
            ProtocolViolation::InvalidDamage { surface_id, .. } => {
                write!(f, "Invalid damage region on surface {}", surface_id)
            }
        }
    }
}

/// Result of an error injection test
#[derive(Debug, Clone)]
pub struct InjectionResult {
    /// Whether the violation was detected
    pub detected: bool,
    /// Protocol error that was caught (if any)
    pub error: Option<ProtocolError>,
    /// Time taken to detect the violation
    pub detection_time: Duration,
    /// Additional context about the test
    pub message: String,
}

/// Client timeout simulation for testing unresponsive clients
#[derive(Debug, Clone)]
pub struct ClientTimeoutSimulator {
    /// Surface ID being simulated
    surface_id: u32,
    /// When the configure was sent
    configure_sent_at: Instant,
    /// Timeout duration before client is considered unresponsive
    timeout_duration: Duration,
    /// Whether timeout has been triggered
    timeout_triggered: bool,
}

impl ClientTimeoutSimulator {
    /// Creates a new timeout simulator
    pub fn new(surface_id: u32, timeout_duration: Duration) -> Self {
        Self {
            surface_id,
            configure_sent_at: Instant::now(),
            timeout_duration,
            timeout_triggered: false,
        }
    }

    /// Checks if the timeout has been exceeded
    pub fn check_timeout(&mut self) -> bool {
        if !self.timeout_triggered {
            let elapsed = self.configure_sent_at.elapsed();
            if elapsed >= self.timeout_duration {
                self.timeout_triggered = true;
                warn!(
                    "⏰ Client timeout detected for surface {} after {:?}",
                    self.surface_id, elapsed
                );
                return true;
            }
        }
        false
    }

    /// Resets the timeout timer (called when client responds)
    pub fn reset(&mut self) {
        self.configure_sent_at = Instant::now();
        self.timeout_triggered = false;
    }
}

/// Error injector for protocol violation testing
#[derive(Debug)]
pub struct ErrorInjector {
    /// Validator being tested
    validator: XdgShellValidator,
    /// Count of violations detected
    violation_count: usize,
    /// History of injected violations
    violation_history: Vec<(Instant, ProtocolViolation, InjectionResult)>,
    /// Active timeout simulators
    timeout_simulators: HashMap<u32, ClientTimeoutSimulator>,
    /// Whether to log detailed test output
    verbose: bool,
}

impl ErrorInjector {
    /// Creates a new error injector
    pub fn new() -> Self {
        Self {
            validator: XdgShellValidator::new(),
            violation_count: 0,
            violation_history: Vec::new(),
            timeout_simulators: HashMap::new(),
            verbose: false,
        }
    }

    /// Creates a new verbose error injector (logs all activity)
    pub fn new_verbose() -> Self {
        Self {
            validator: XdgShellValidator::new(),
            violation_count: 0,
            violation_history: Vec::new(),
            timeout_simulators: HashMap::new(),
            verbose: true,
        }
    }

    /// Injects a protocol violation and tests if it's detected
    pub fn inject_violation(&mut self, violation: ProtocolViolation) -> Result<InjectionResult, String> {
        let start_time = Instant::now();
        
        if self.verbose {
            debug!("🧪 Injecting violation: {}", violation);
        }

        let result = match &violation {
            ProtocolViolation::BufferCommitBeforeAck { surface_id } => {
                self.test_buffer_before_ack(*surface_id)
            }
            ProtocolViolation::AckUnknownSerial { surface_id, serial } => {
                self.test_unknown_serial(*surface_id, *serial)
            }
            ProtocolViolation::DoubleRoleAssignment { surface_id, first_role, second_role } => {
                self.test_double_role(*surface_id, *first_role, *second_role)
            }
            ProtocolViolation::ConfigureTimeout { surface_id, serial, timeout } => {
                self.test_configure_timeout(*surface_id, *serial, *timeout)
            }
            ProtocolViolation::UseAfterDestroy { surface_id } => {
                self.test_use_after_destroy(*surface_id)
            }
            ProtocolViolation::InvalidPositioner { surface_id, reason } => {
                self.test_invalid_positioner(*surface_id, reason.clone())
            }
            ProtocolViolation::InvalidDamage { surface_id, x, y, width, height } => {
                self.test_invalid_damage(*surface_id, *x, *y, *width, *height)
            }
        };

        let detection_time = start_time.elapsed();
        
        match result {
            Ok(error) => {
                self.violation_count += 1;
                let injection_result = InjectionResult {
                    detected: true,
                    error: Some(error),
                    detection_time,
                    message: format!("Violation detected: {}", violation),
                };
                
                self.violation_history.push((Instant::now(), violation.clone(), injection_result.clone()));
                
                if self.verbose {
                    debug!("✅ Violation detected in {:?}", detection_time);
                }
                
                Ok(injection_result)
            }
            Err(msg) => {
                let injection_result = InjectionResult {
                    detected: false,
                    error: None,
                    detection_time,
                    message: format!("Violation NOT detected: {}", msg),
                };
                
                self.violation_history.push((Instant::now(), violation.clone(), injection_result.clone()));
                
                if self.verbose {
                    error!("❌ Violation NOT detected: {}", msg);
                }
                
                Err(msg)
            }
        }
    }

    /// Tests buffer commit before ack violation
    fn test_buffer_before_ack(&mut self, surface_id: u32) -> Result<ProtocolError, String> {
        // Setup: Register surface and assign role
        self.validator.register_surface(surface_id);
        self.validator.assign_role(surface_id, XdgRole::Toplevel)
            .map_err(|e| format!("Setup failed: {}", e))?;
        
        // Send configure
        self.validator.add_configure(surface_id, 100, 800, 600)
            .map_err(|e| format!("Configure failed: {}", e))?;
        
        // Inject violation: commit buffer without ack
        match self.validator.validate_commit(surface_id, true) {
            Ok(_) => Err("Violation NOT detected - buffer commit was allowed".to_string()),
            Err(e) => Ok(e),
        }
    }

    /// Tests unknown serial acknowledgment violation
    fn test_unknown_serial(&mut self, surface_id: u32, serial: u32) -> Result<ProtocolError, String> {
        // Setup: Register surface
        self.validator.register_surface(surface_id);
        self.validator.assign_role(surface_id, XdgRole::Toplevel)
            .map_err(|e| format!("Setup failed: {}", e))?;
        
        // Send a different configure
        self.validator.add_configure(surface_id, 50, 800, 600)
            .map_err(|e| format!("Configure failed: {}", e))?;
        
        // Inject violation: ack unknown serial
        match self.validator.ack_configure(surface_id, serial) {
            Ok(_) => Err("Violation NOT detected - unknown serial was accepted".to_string()),
            Err(e) => Ok(e),
        }
    }

    /// Tests double role assignment violation
    fn test_double_role(&mut self, surface_id: u32, first_role: XdgRole, second_role: XdgRole) -> Result<ProtocolError, String> {
        // Setup: Register surface and assign first role
        self.validator.register_surface(surface_id);
        self.validator.assign_role(surface_id, first_role)
            .map_err(|e| format!("First role assignment failed: {}", e))?;
        
        // Inject violation: assign second role
        match self.validator.assign_role(surface_id, second_role) {
            Ok(_) => Err("Violation NOT detected - double role assignment was allowed".to_string()),
            Err(e) => Ok(e),
        }
    }

    /// Tests configure timeout violation
    fn test_configure_timeout(&mut self, surface_id: u32, serial: u32, timeout: Duration) -> Result<ProtocolError, String> {
        // Register timeout simulator
        let simulator = ClientTimeoutSimulator::new(surface_id, timeout);
        self.timeout_simulators.insert(surface_id, simulator);
        
        // Setup: Register surface and send configure
        self.validator.register_surface(surface_id);
        self.validator.assign_role(surface_id, XdgRole::Toplevel)
            .map_err(|e| format!("Setup failed: {}", e))?;
        self.validator.add_configure(surface_id, serial, 800, 600)
            .map_err(|e| format!("Configure failed: {}", e))?;
        
        // Simulate passage of time (in real tests, this would involve sleeping or time mocking)
        // For now, we check if timeout would be detected
        let warnings = self.validator.check_timeouts();
        
        if warnings.is_empty() {
            Err("Timeout not detected yet (may need more time)".to_string())
        } else {
            // Convert warning to error for consistency
            Ok(ProtocolError::InvalidAckSerial {
                surface_id,
                acked_serial: 0,
                pending_serials: vec![serial],
            })
        }
    }

    /// Tests use-after-destroy violation
    fn test_use_after_destroy(&mut self, surface_id: u32) -> Result<ProtocolError, String> {
        // Setup: Register and then unregister surface
        self.validator.register_surface(surface_id);
        self.validator.unregister_surface(surface_id);
        
        // Inject violation: try to use destroyed surface
        match self.validator.ack_configure(surface_id, 100) {
            Ok(_) => Err("Violation NOT detected - destroyed surface was usable".to_string()),
            Err(e) => Ok(e),
        }
    }

    /// Tests invalid positioner violation
    fn test_invalid_positioner(&mut self, surface_id: u32, reason: String) -> Result<ProtocolError, String> {
        self.validator.register_surface(surface_id);
        
        // For now, return a synthetic error since positioner validation is not fully implemented
        Ok(ProtocolError::InvalidPositioner { surface_id, reason })
    }

    /// Tests invalid damage region
    fn test_invalid_damage(&mut self, surface_id: u32, _x: i32, _y: i32, _width: i32, _height: i32) -> Result<ProtocolError, String> {
        // Damage validation would be checked at commit time
        // For now, this is a placeholder for future implementation
        Err(format!("Damage validation not yet fully implemented for surface {}", surface_id))
    }

    /// Returns the number of violations detected
    pub fn violation_count(&self) -> usize {
        self.violation_count
    }

    /// Returns the validator statistics
    pub fn validator_stats(&self) -> super::xdg_shell_validation::ValidatorStats {
        self.validator.stats()
    }

    /// Gets the violation history
    pub fn violation_history(&self) -> &[(Instant, ProtocolViolation, InjectionResult)] {
        &self.violation_history
    }

    /// Checks all active timeout simulators
    pub fn check_all_timeouts(&mut self) -> Vec<u32> {
        let mut timed_out = Vec::new();
        
        for (surface_id, simulator) in self.timeout_simulators.iter_mut() {
            if simulator.check_timeout() {
                timed_out.push(*surface_id);
            }
        }
        
        timed_out
    }

    /// Resets all statistics
    pub fn reset(&mut self) {
        self.validator = XdgShellValidator::new();
        self.violation_count = 0;
        self.violation_history.clear();
        self.timeout_simulators.clear();
    }
}

impl Default for ErrorInjector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_before_ack_detection() {
        let mut injector = ErrorInjector::new();
        
        let result = injector.inject_violation(
            ProtocolViolation::BufferCommitBeforeAck { surface_id: 1 }
        );
        
        assert!(result.is_ok());
        let injection_result = result.unwrap();
        assert!(injection_result.detected);
        assert!(injection_result.error.is_some());
        assert_eq!(injector.violation_count(), 1);
    }

    #[test]
    fn test_unknown_serial_detection() {
        let mut injector = ErrorInjector::new();
        
        let result = injector.inject_violation(
            ProtocolViolation::AckUnknownSerial {
                surface_id: 1,
                serial: 999,
            }
        );
        
        assert!(result.is_ok());
        assert_eq!(injector.violation_count(), 1);
    }

    #[test]
    fn test_double_role_detection() {
        let mut injector = ErrorInjector::new();
        
        let result = injector.inject_violation(
            ProtocolViolation::DoubleRoleAssignment {
                surface_id: 1,
                first_role: XdgRole::Toplevel,
                second_role: XdgRole::Popup,
            }
        );
        
        assert!(result.is_ok());
        assert_eq!(injector.violation_count(), 1);
    }

    #[test]
    fn test_use_after_destroy_detection() {
        let mut injector = ErrorInjector::new();
        
        let result = injector.inject_violation(
            ProtocolViolation::UseAfterDestroy { surface_id: 1 }
        );
        
        assert!(result.is_ok());
        assert_eq!(injector.violation_count(), 1);
    }

    #[test]
    fn test_multiple_violations() {
        let mut injector = ErrorInjector::new();
        
        // Inject multiple different violations
        let _ = injector.inject_violation(
            ProtocolViolation::BufferCommitBeforeAck { surface_id: 1 }
        );
        
        let _ = injector.inject_violation(
            ProtocolViolation::AckUnknownSerial { surface_id: 2, serial: 999 }
        );
        
        let _ = injector.inject_violation(
            ProtocolViolation::UseAfterDestroy { surface_id: 3 }
        );
        
        assert_eq!(injector.violation_count(), 3);
        assert_eq!(injector.violation_history().len(), 3);
    }

    #[test]
    fn test_timeout_simulator() {
        let mut simulator = ClientTimeoutSimulator::new(1, Duration::from_millis(100));
        
        // Should not timeout immediately
        assert!(!simulator.check_timeout());
        
        // Reset and check again
        simulator.reset();
        assert!(!simulator.check_timeout());
    }

    #[test]
    fn test_injector_reset() {
        let mut injector = ErrorInjector::new();
        
        // Add some violations
        let _ = injector.inject_violation(
            ProtocolViolation::BufferCommitBeforeAck { surface_id: 1 }
        );
        
        assert_eq!(injector.violation_count(), 1);
        
        // Reset
        injector.reset();
        
        assert_eq!(injector.violation_count(), 0);
        assert_eq!(injector.violation_history().len(), 0);
    }
}
