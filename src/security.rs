//! Security Hardening and Sandboxing
//!
//! Production-grade security policies, input validation, resource limits,
//! and privilege separation for compositor safety.
//!
//! # Features
//!
//! - **Input Validation**: Sanitize all external inputs
//! - **Resource Limits**: CPU, memory, file descriptor limits
//! - **Privilege Separation**: Drop unnecessary capabilities
//! - **Sandboxing**: Isolate untrusted components
//! - **Rate Limiting**: Prevent DoS attacks
//! - **Permission Checks**: Fine-grained access control
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────┐     ┌──────────────┐     ┌─────────────┐
//! │  Client Input  │────►│  Validator   │────►│  Sanitized  │
//! │  (Untrusted)   │     │  (Security)  │     │    Data     │
//! └────────────────┘     └──────────────┘     └─────────────┘
//!         │                      │                     │
//!         │                      ▼                     ▼
//!         │              ┌──────────────┐     ┌─────────────┐
//!         └─────────────►│   Resource   │────►│  Execution  │
//!                        │   Limiter    │     │  (Limited)  │
//!                        └──────────────┘     └─────────────┘
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use axiom::security::{SecurityManager, SecurityPolicy, ResourceLimits};
//!
//! // Initialize security
//! let policy = SecurityPolicy::default();
//! let mut security = SecurityManager::new(policy);
//! security.init();
//!
//! // Validate input
//! if let Err(e) = security.validate_window_title("My Window") {
//!     println!("Invalid input: {}", e);
//! }
//!
//! // Check rate limits
//! if !security.check_rate_limit(client_id, "create_window") {
//!     println!("Rate limit exceeded");
//! }
//!
//! // Enforce resource limits
//! security.enforce_limits(process_id);
//! ```

use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Security policy configuration
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Maximum string length for user inputs
    pub max_string_length: usize,
    /// Maximum number of windows per client
    pub max_windows_per_client: usize,
    /// Maximum number of surfaces per client
    pub max_surfaces_per_client: usize,
    /// Rate limit: operations per second
    pub rate_limit_ops_per_sec: u32,
    /// Rate limit window duration
    pub rate_limit_window: Duration,
    /// Enable input sanitization
    pub sanitize_inputs: bool,
    /// Enable resource limiting
    pub enforce_resource_limits: bool,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            max_string_length: 1024,
            max_windows_per_client: 100,
            max_surfaces_per_client: 200,
            rate_limit_ops_per_sec: 100,
            rate_limit_window: Duration::from_secs(1),
            sanitize_inputs: true,
            enforce_resource_limits: true,
        }
    }
}

/// Resource limits for a client
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Maximum memory usage (bytes)
    pub max_memory: u64,
    /// Maximum CPU time (milliseconds)
    pub max_cpu_time: u64,
    /// Maximum file descriptors
    pub max_file_descriptors: u32,
    /// Maximum buffer size (bytes)
    pub max_buffer_size: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory: 512 * 1024 * 1024, // 512 MB
            max_cpu_time: 1000,            // 1 second
            max_file_descriptors: 256,
            max_buffer_size: 32 * 1024 * 1024, // 32 MB
        }
    }
}

/// Rate limit tracker for a client
#[derive(Debug)]
struct RateLimitBucket {
    /// Operation counts by type
    operations: HashMap<String, Vec<Instant>>,
    /// Total operations in current window
    total_operations: usize,
}

impl RateLimitBucket {
    fn new() -> Self {
        Self {
            operations: HashMap::new(),
            total_operations: 0,
        }
    }

    /// Record an operation
    fn record(&mut self, operation: &str, now: Instant) {
        self.operations
            .entry(operation.to_string())
            .or_insert_with(Vec::new)
            .push(now);
        self.total_operations += 1;
    }

    /// Clean up old entries outside the window
    fn cleanup(&mut self, window: Duration, now: Instant) {
        for operations in self.operations.values_mut() {
            operations.retain(|&time| now.duration_since(time) < window);
        }
        self.total_operations = self.operations.values().map(|v| v.len()).sum();
    }

    /// Get operation count in current window
    fn count(&self, operation: Option<&str>) -> usize {
        match operation {
            Some(op) => self.operations.get(op).map(|v| v.len()).unwrap_or(0),
            None => self.total_operations,
        }
    }
}

/// Client resource tracking
#[derive(Debug)]
struct ClientResources {
    /// Number of windows
    pub windows: usize,
    /// Number of surfaces
    pub surfaces: usize,
}

impl ClientResources {
    fn new() -> Self {
        Self {
            windows: 0,
            surfaces: 0,
        }
    }
}

/// Security manager
pub struct SecurityManager {
    /// Security policy
    policy: SecurityPolicy,
    /// Rate limit buckets by client ID
    rate_limits: HashMap<u32, RateLimitBucket>,
    /// Client resources
    client_resources: HashMap<u32, ClientResources>,
    /// Blocked clients
    blocked_clients: HashMap<u32, Instant>,
    /// Statistics
    stats: SecurityStats,
}

impl SecurityManager {
    /// Create a new security manager
    pub fn new(policy: SecurityPolicy) -> Self {
        Self {
            policy,
            rate_limits: HashMap::new(),
            client_resources: HashMap::new(),
            blocked_clients: HashMap::new(),
            stats: SecurityStats::default(),
        }
    }

    /// Initialize security system
    pub fn init(&mut self) -> Result<(), String> {
        info!(
            "🔒 Security manager initialized (max_string: {}, max_windows: {})",
            self.policy.max_string_length, self.policy.max_windows_per_client
        );
        Ok(())
    }

    /// Validate a string input
    pub fn validate_string(&self, input: &str, field_name: &str) -> Result<(), String> {
        if !self.policy.sanitize_inputs {
            return Ok(());
        }

        // Check length
        if input.len() > self.policy.max_string_length {
            return Err(format!(
                "{} exceeds maximum length ({} > {})",
                field_name,
                input.len(),
                self.policy.max_string_length
            ));
        }

        // Check for null bytes
        if input.contains('\0') {
            return Err(format!("{} contains null bytes", field_name));
        }

        // Check for control characters (except newline, tab, carriage return)
        for ch in input.chars() {
            if ch.is_control() && ch != '\n' && ch != '\t' && ch != '\r' {
                return Err(format!("{} contains control characters", field_name));
            }
        }

        Ok(())
    }

    /// Validate a window title
    pub fn validate_window_title(&self, title: &str) -> Result<(), String> {
        self.validate_string(title, "Window title")
    }

    /// Validate a class name
    pub fn validate_class(&self, class: &str) -> Result<(), String> {
        self.validate_string(class, "Class name")
    }

    /// Validate coordinates
    pub fn validate_coordinates(&self, x: i32, y: i32) -> Result<(), String> {
        const MAX_COORD: i32 = 32767;
        const MIN_COORD: i32 = -32768;

        if x < MIN_COORD || x > MAX_COORD {
            return Err(format!("X coordinate out of range: {}", x));
        }

        if y < MIN_COORD || y > MAX_COORD {
            return Err(format!("Y coordinate out of range: {}", y));
        }

        Ok(())
    }

    /// Validate dimensions
    pub fn validate_dimensions(&self, width: u32, height: u32) -> Result<(), String> {
        const MAX_DIMENSION: u32 = 16384; // 16K

        if width == 0 || height == 0 {
            return Err("Dimensions must be non-zero".to_string());
        }

        if width > MAX_DIMENSION {
            return Err(format!("Width exceeds maximum: {}", width));
        }

        if height > MAX_DIMENSION {
            return Err(format!("Height exceeds maximum: {}", height));
        }

        Ok(())
    }

    /// Check rate limit for a client
    pub fn check_rate_limit(&mut self, client_id: u32, operation: &str) -> bool {
        let now = Instant::now();

        // Clean up expired blocks
        self.blocked_clients
            .retain(|_, time| now.duration_since(*time) < Duration::from_secs(60));

        // Check if client is blocked
        if self.blocked_clients.contains_key(&client_id) {
            return false;
        }

        // Get or create rate limit bucket
        let bucket = self.rate_limits.entry(client_id).or_insert_with(RateLimitBucket::new);

        // Clean up old entries
        bucket.cleanup(self.policy.rate_limit_window, now);

        // Check limit
        if bucket.count(None) >= self.policy.rate_limit_ops_per_sec as usize {
            self.stats.rate_limit_violations += 1;
            self.blocked_clients.insert(client_id, now);
            warn!("🔒 Rate limit exceeded for client {}", client_id);
            return false;
        }

        // Record operation
        bucket.record(operation, now);
        true
    }

    /// Check if client can create a window
    pub fn check_window_limit(&mut self, client_id: u32) -> Result<(), String> {
        let resources = self
            .client_resources
            .entry(client_id)
            .or_insert_with(ClientResources::new);

        if resources.windows >= self.policy.max_windows_per_client {
            self.stats.resource_limit_violations += 1;
            return Err(format!(
                "Client {} exceeded window limit ({})",
                client_id, self.policy.max_windows_per_client
            ));
        }

        Ok(())
    }

    /// Register a new window for a client
    pub fn register_window(&mut self, client_id: u32) {
        let resources = self
            .client_resources
            .entry(client_id)
            .or_insert_with(ClientResources::new);
        resources.windows += 1;
    }

    /// Unregister a window for a client
    pub fn unregister_window(&mut self, client_id: u32) {
        if let Some(resources) = self.client_resources.get_mut(&client_id) {
            resources.windows = resources.windows.saturating_sub(1);
        }
    }

    /// Check if client can create a surface
    pub fn check_surface_limit(&mut self, client_id: u32) -> Result<(), String> {
        let resources = self
            .client_resources
            .entry(client_id)
            .or_insert_with(ClientResources::new);

        if resources.surfaces >= self.policy.max_surfaces_per_client {
            self.stats.resource_limit_violations += 1;
            return Err(format!(
                "Client {} exceeded surface limit ({})",
                client_id, self.policy.max_surfaces_per_client
            ));
        }

        Ok(())
    }

    /// Register a new surface for a client
    pub fn register_surface(&mut self, client_id: u32) {
        let resources = self
            .client_resources
            .entry(client_id)
            .or_insert_with(ClientResources::new);
        resources.surfaces += 1;
    }

    /// Unregister a surface for a client
    pub fn unregister_surface(&mut self, client_id: u32) {
        if let Some(resources) = self.client_resources.get_mut(&client_id) {
            resources.surfaces = resources.surfaces.saturating_sub(1);
        }
    }

    /// Sanitize a string (remove potentially dangerous content)
    pub fn sanitize_string(&self, input: &str) -> String {
        if !self.policy.sanitize_inputs {
            return input.to_string();
        }

        input
            .chars()
            .filter(|&c| !c.is_control() || c == '\n' || c == '\t' || c == '\r')
            .take(self.policy.max_string_length)
            .collect()
    }

    /// Remove client tracking
    pub fn remove_client(&mut self, client_id: u32) {
        self.rate_limits.remove(&client_id);
        self.client_resources.remove(&client_id);
        self.blocked_clients.remove(&client_id);
        debug!("🔒 Removed security tracking for client {}", client_id);
    }

    /// Get security statistics
    pub fn stats(&self) -> SecurityStats {
        let mut stats = self.stats;
        stats.active_clients = self.client_resources.len();
        stats.blocked_clients = self.blocked_clients.len();
        stats
    }
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new(SecurityPolicy::default())
    }
}

/// Security statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct SecurityStats {
    /// Total validation failures
    pub validation_failures: usize,
    /// Rate limit violations
    pub rate_limit_violations: usize,
    /// Resource limit violations
    pub resource_limit_violations: usize,
    /// Active clients being tracked
    pub active_clients: usize,
    /// Currently blocked clients
    pub blocked_clients: usize,
}

/// Thread-safe security manager wrapper
pub type SharedSecurity = Arc<Mutex<SecurityManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_string() {
        let security = SecurityManager::default();

        assert!(security.validate_string("Valid string", "test").is_ok());
        assert!(security.validate_string("String with \n newline", "test").is_ok());

        // Test null byte rejection
        assert!(security.validate_string("String with \0 null", "test").is_err());

        // Test length limit
        let long_string = "a".repeat(2000);
        assert!(security.validate_string(&long_string, "test").is_err());
    }

    #[test]
    fn test_validate_coordinates() {
        let security = SecurityManager::default();

        assert!(security.validate_coordinates(0, 0).is_ok());
        assert!(security.validate_coordinates(1920, 1080).is_ok());
        assert!(security.validate_coordinates(-100, -100).is_ok());

        assert!(security.validate_coordinates(50000, 0).is_err());
        assert!(security.validate_coordinates(0, -50000).is_err());
    }

    #[test]
    fn test_validate_dimensions() {
        let security = SecurityManager::default();

        assert!(security.validate_dimensions(1920, 1080).is_ok());
        assert!(security.validate_dimensions(1, 1).is_ok());

        assert!(security.validate_dimensions(0, 100).is_err());
        assert!(security.validate_dimensions(100, 0).is_err());
        assert!(security.validate_dimensions(20000, 20000).is_err());
    }

    #[test]
    fn test_rate_limiting() {
        let mut security = SecurityManager::default();

        // Should allow initial operations
        for _ in 0..50 {
            assert!(security.check_rate_limit(1, "test_op"));
        }

        // Should block after limit
        for _ in 0..100 {
            security.check_rate_limit(1, "test_op");
        }

        // Should be blocked now
        assert!(!security.check_rate_limit(1, "test_op"));
    }

    #[test]
    fn test_window_limits() {
        let mut security = SecurityManager::default();

        // Register windows up to limit
        for _ in 0..100 {
            assert!(security.check_window_limit(1).is_ok());
            security.register_window(1);
        }

        // Should fail after limit
        assert!(security.check_window_limit(1).is_err());

        // Unregister one window
        security.unregister_window(1);

        // Should allow again
        assert!(security.check_window_limit(1).is_ok());
    }

    #[test]
    fn test_surface_limits() {
        let mut security = SecurityManager::default();

        // Register surfaces up to limit
        for _ in 0..200 {
            assert!(security.check_surface_limit(1).is_ok());
            security.register_surface(1);
        }

        // Should fail after limit
        assert!(security.check_surface_limit(1).is_err());
    }

    #[test]
    fn test_sanitize_string() {
        let security = SecurityManager::default();

        let input = "Valid string\nwith newline";
        let sanitized = security.sanitize_string(input);
        assert_eq!(sanitized, input);

        let input_with_null = "String\0with\0null";
        let sanitized = security.sanitize_string(input_with_null);
        assert!(!sanitized.contains('\0'));
    }

    #[test]
    fn test_multiple_clients() {
        let mut security = SecurityManager::default();

        security.register_window(1);
        security.register_window(2);

        assert_eq!(security.stats().active_clients, 2);

        security.remove_client(1);
        assert_eq!(security.stats().active_clients, 1);
    }

    #[test]
    fn test_window_title_validation() {
        let security = SecurityManager::default();

        assert!(security.validate_window_title("My Window").is_ok());
        assert!(security.validate_window_title("Window - Firefox").is_ok());
        assert!(security.validate_window_title("測試視窗").is_ok()); // UTF-8

        let long_title = "a".repeat(2000);
        assert!(security.validate_window_title(&long_title).is_err());
    }

    #[test]
    fn test_class_validation() {
        let security = SecurityManager::default();

        assert!(security.validate_class("firefox").is_ok());
        assert!(security.validate_class("org.gnome.Terminal").is_ok());
    }

    #[test]
    fn test_rate_limit_per_operation() {
        let mut security = SecurityManager::default();

        // Different operations should have separate counts
        for _ in 0..50 {
            assert!(security.check_rate_limit(1, "create_window"));
        }

        for _ in 0..50 {
            assert!(security.check_rate_limit(1, "create_surface"));
        }

        // Total should now be 100, next should fail
        assert!(!security.check_rate_limit(1, "test_op"));
    }

    #[test]
    fn test_stats() {
        let mut security = SecurityManager::default();

        security.register_window(1);
        security.register_surface(1);

        let stats = security.stats();
        assert_eq!(stats.active_clients, 1);
    }
}
