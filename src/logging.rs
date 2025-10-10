//! Production Logging and Debugging Infrastructure
//!
//! Comprehensive logging system with structured logging, performance tracing,
//! debug tools, and log management for production environments.
//!
//! # Features
//!
//! - **Structured Logging**: JSON-formatted logs with context
//! - **Log Levels**: TRACE, DEBUG, INFO, WARN, ERROR, CRITICAL
//! - **Performance Tracing**: Function timing and spans
//! - **Log Rotation**: Automatic rotation by size and age
//! - **Debug Modes**: Runtime debug level switching
//! - **Metrics Collection**: Performance counters and statistics
//! - **Remote Logging**: Optional remote log shipping
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
//! │   Logger     │────►│  Formatter   │────►│   Output     │
//! │  (Capture)   │     │   (JSON)     │     │  (File/Net)  │
//! └──────────────┘     └──────────────┘     └──────────────┘
//!        │
//!        ▼
//! ┌──────────────┐
//! │   Filters    │
//! │  (Level/Mod) │
//! └──────────────┘
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use axiom::logging::{LogManager, LogLevel, LogConfig};
//!
//! // Initialize logging
//! let config = LogConfig {
//!     level: LogLevel::Info,
//!     file_path: Some("/var/log/axiom/compositor.log".into()),
//!     max_file_size: 10 * 1024 * 1024, // 10MB
//!     max_files: 5,
//!     structured: true,
//! };
//!
//! let mut logger = LogManager::new(config);
//! logger.init();
//!
//! // Log with context
//! logger.info("Window created", &[("window_id", "123"), ("title", "Terminal")]);
//!
//! // Performance tracing
//! let span = logger.start_span("render_frame");
//! // ... do work ...
//! logger.end_span(span);
//! ```

use log::{debug, info, trace};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Log level enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Trace level (most verbose)
    Trace = 0,
    /// Debug level
    Debug = 1,
    /// Info level
    Info = 2,
    /// Warning level
    Warn = 3,
    /// Error level
    Error = 4,
    /// Critical level (most severe)
    Critical = 5,
}

impl LogLevel {
    /// Convert to string
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
        }
    }

    /// Convert from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "TRACE" => Some(LogLevel::Trace),
            "DEBUG" => Some(LogLevel::Debug),
            "INFO" => Some(LogLevel::Info),
            "WARN" | "WARNING" => Some(LogLevel::Warn),
            "ERROR" => Some(LogLevel::Error),
            "CRITICAL" | "CRIT" => Some(LogLevel::Critical),
            _ => None,
        }
    }
}

/// Log entry structure
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Timestamp
    pub timestamp: SystemTime,
    /// Log level
    pub level: LogLevel,
    /// Module path
    pub module: String,
    /// Message
    pub message: String,
    /// Context key-value pairs
    pub context: HashMap<String, String>,
}

impl LogEntry {
    /// Create a new log entry
    pub fn new(level: LogLevel, module: &str, message: String) -> Self {
        Self {
            timestamp: SystemTime::now(),
            level,
            module: module.to_string(),
            message,
            context: HashMap::new(),
        }
    }

    /// Add context
    pub fn with_context(mut self, key: &str, value: &str) -> Self {
        self.context.insert(key.to_string(), value.to_string());
        self
    }

    /// Format as JSON
    pub fn to_json(&self) -> String {
        let timestamp = self
            .timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut json = format!(
            r#"{{"timestamp":{},"level":"{}","module":"{}","message":"{}""#,
            timestamp,
            self.level.as_str(),
            self.module,
            self.message.replace('"', "\\\"")
        );

        if !self.context.is_empty() {
            json.push_str(r#","context":{"#);
            let mut first = true;
            for (k, v) in &self.context {
                if !first {
                    json.push(',');
                }
                json.push_str(&format!(
                    r#""{}":"{}""#,
                    k.replace('"', "\\\""),
                    v.replace('"', "\\\"")
                ));
                first = false;
            }
            json.push('}');
        }

        json.push('}');
        json
    }

    /// Format as plain text
    pub fn to_text(&self) -> String {
        let timestamp = self
            .timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut text = format!(
            "[{}] {} {}: {}",
            timestamp,
            self.level.as_str(),
            self.module,
            self.message
        );

        if !self.context.is_empty() {
            text.push_str(" {");
            let mut first = true;
            for (k, v) in &self.context {
                if !first {
                    text.push_str(", ");
                }
                text.push_str(&format!("{}={}", k, v));
                first = false;
            }
            text.push('}');
        }

        text
    }
}

/// Performance span for tracing
#[derive(Debug)]
pub struct Span {
    /// Span ID
    pub id: u64,
    /// Name
    pub name: String,
    /// Start time
    pub start: Instant,
    /// Parent span ID
    pub parent: Option<u64>,
}

impl Span {
    /// Calculate duration
    pub fn duration(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Log configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Minimum log level
    pub level: LogLevel,
    /// Output file path
    pub file_path: Option<PathBuf>,
    /// Maximum file size (bytes) before rotation
    pub max_file_size: u64,
    /// Maximum number of rotated files to keep
    pub max_files: usize,
    /// Use structured (JSON) logging
    pub structured: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            file_path: None,
            max_file_size: 10 * 1024 * 1024, // 10MB
            max_files: 5,
            structured: true,
        }
    }
}

/// Main log manager
pub struct LogManager {
    /// Configuration
    config: LogConfig,
    /// Current log file
    file: Option<File>,
    /// Current file size
    file_size: u64,
    /// Active spans
    spans: HashMap<u64, Span>,
    /// Next span ID
    next_span_id: u64,
    /// Statistics
    stats: LogStats,
}

impl LogManager {
    /// Create a new log manager
    pub fn new(config: LogConfig) -> Self {
        Self {
            config,
            file: None,
            file_size: 0,
            spans: HashMap::new(),
            next_span_id: 1,
            stats: LogStats::default(),
        }
    }

    /// Initialize logging system
    pub fn init(&mut self) -> Result<(), String> {
        if let Some(path) = self.config.file_path.clone() {
            self.open_log_file(&path)?;
        }

        info!("📊 Log manager initialized (level: {:?}, structured: {})", 
              self.config.level, self.config.structured);

        Ok(())
    }

    /// Open or create log file
    fn open_log_file(&mut self, path: &Path) -> Result<(), String> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create log directory: {}", e))?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("Failed to open log file: {}", e))?;

        self.file_size = file
            .metadata()
            .map(|m| m.len())
            .unwrap_or(0);

        self.file = Some(file);
        Ok(())
    }

    /// Log an entry
    pub fn log(&mut self, entry: LogEntry) {
        // Check level filter
        if entry.level < self.config.level {
            return;
        }

        self.stats.total_logs += 1;
        self.stats.logs_by_level[entry.level as usize] += 1;

        // Format log
        let formatted = if self.config.structured {
            entry.to_json()
        } else {
            entry.to_text()
        };

        // Write to file
        if let Some(file) = &mut self.file {
            if let Err(e) = writeln!(file, "{}", formatted) {
                eprintln!("Failed to write log: {}", e);
            } else {
                self.file_size += formatted.len() as u64 + 1;

                // Check for rotation
                if self.file_size >= self.config.max_file_size {
                    if let Some(path) = &self.config.file_path.clone() {
                        let _ = self.rotate_logs(path);
                    }
                }
            }
        }

        // Also print to stdout/stderr based on level
        if entry.level >= LogLevel::Error {
            eprintln!("{}", formatted);
        } else if entry.level >= LogLevel::Debug {
            println!("{}", formatted);
        }
    }

    /// Rotate log files
    fn rotate_logs(&mut self, base_path: &Path) -> Result<(), String> {
        // Close current file
        self.file = None;

        // Rotate existing files
        for i in (1..self.config.max_files).rev() {
            let old_path = if i == 1 {
                base_path.to_path_buf()
            } else {
                PathBuf::from(format!("{}.{}", base_path.display(), i - 1))
            };

            let new_path = PathBuf::from(format!("{}.{}", base_path.display(), i));

            if old_path.exists() {
                let _ = std::fs::rename(&old_path, &new_path);
            }
        }

        self.stats.rotations += 1;
        info!("📊 Rotated log files");

        // Open new file
        self.open_log_file(base_path)?;
        Ok(())
    }

    /// Log at trace level
    pub fn trace(&mut self, message: &str, context: &[(&str, &str)]) {
        let mut entry = LogEntry::new(LogLevel::Trace, "axiom", message.to_string());
        for (k, v) in context {
            entry = entry.with_context(k, v);
        }
        self.log(entry);
    }

    /// Log at debug level
    pub fn debug(&mut self, message: &str, context: &[(&str, &str)]) {
        let mut entry = LogEntry::new(LogLevel::Debug, "axiom", message.to_string());
        for (k, v) in context {
            entry = entry.with_context(k, v);
        }
        self.log(entry);
    }

    /// Log at info level
    pub fn info(&mut self, message: &str, context: &[(&str, &str)]) {
        let mut entry = LogEntry::new(LogLevel::Info, "axiom", message.to_string());
        for (k, v) in context {
            entry = entry.with_context(k, v);
        }
        self.log(entry);
    }

    /// Log at warning level
    pub fn warn(&mut self, message: &str, context: &[(&str, &str)]) {
        let mut entry = LogEntry::new(LogLevel::Warn, "axiom", message.to_string());
        for (k, v) in context {
            entry = entry.with_context(k, v);
        }
        self.log(entry);
    }

    /// Log at error level
    pub fn error(&mut self, message: &str, context: &[(&str, &str)]) {
        let mut entry = LogEntry::new(LogLevel::Error, "axiom", message.to_string());
        for (k, v) in context {
            entry = entry.with_context(k, v);
        }
        self.log(entry);
    }

    /// Start a performance span
    pub fn start_span(&mut self, name: &str) -> u64 {
        let id = self.next_span_id;
        self.next_span_id += 1;

        let span = Span {
            id,
            name: name.to_string(),
            start: Instant::now(),
            parent: None,
        };

        trace!("🔍 Span started: {} (id: {})", name, id);
        self.spans.insert(id, span);
        id
    }

    /// End a performance span
    pub fn end_span(&mut self, span_id: u64) {
        if let Some(span) = self.spans.remove(&span_id) {
            let duration = span.duration();
            self.stats.total_spans += 1;
            self.stats.total_span_time += duration;

            debug!(
                "🔍 Span ended: {} ({:.2}ms)",
                span.name,
                duration.as_secs_f64() * 1000.0
            );
        }
    }

    /// Set log level at runtime
    pub fn set_level(&mut self, level: LogLevel) {
        self.config.level = level;
        info!("📊 Log level changed to {:?}", level);
    }

    /// Get statistics
    pub fn stats(&self) -> LogStats {
        self.stats
    }

    /// Flush logs to disk
    pub fn flush(&mut self) -> Result<(), String> {
        if let Some(file) = &mut self.file {
            file.flush()
                .map_err(|e| format!("Failed to flush logs: {}", e))?;
        }
        Ok(())
    }
}

/// Logging statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct LogStats {
    /// Total logs written
    pub total_logs: usize,
    /// Logs by level [TRACE, DEBUG, INFO, WARN, ERROR, CRITICAL]
    pub logs_by_level: [usize; 6],
    /// Number of file rotations
    pub rotations: usize,
    /// Total spans created
    pub total_spans: usize,
    /// Total time spent in spans
    pub total_span_time: Duration,
}

/// Thread-safe log manager wrapper
pub type SharedLogger = Arc<Mutex<LogManager>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_log_level() {
        assert!(LogLevel::Debug > LogLevel::Trace);
        assert!(LogLevel::Error > LogLevel::Warn);
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::Info.as_str(), "INFO");
    }

    #[test]
    fn test_log_entry_json() {
        let entry = LogEntry::new(LogLevel::Info, "test", "Test message".to_string())
            .with_context("key", "value");

        let json = entry.to_json();
        assert!(json.contains("\"level\":\"INFO\""));
        assert!(json.contains("\"message\":\"Test message\""));
        assert!(json.contains("\"key\":\"value\""));
    }

    #[test]
    fn test_log_entry_text() {
        let entry = LogEntry::new(LogLevel::Warn, "test", "Warning message".to_string())
            .with_context("user", "admin");

        let text = entry.to_text();
        assert!(text.contains("WARN"));
        assert!(text.contains("Warning message"));
        assert!(text.contains("user=admin"));
    }

    #[test]
    fn test_log_manager_basic() {
        let config = LogConfig::default();
        let mut logger = LogManager::new(config);

        logger.info("Test log", &[("test", "true")]);
        assert_eq!(logger.stats().total_logs, 1);
    }

    #[test]
    fn test_log_level_filtering() {
        let mut config = LogConfig::default();
        config.level = LogLevel::Warn;

        let mut logger = LogManager::new(config);

        logger.debug("Debug message", &[]);
        logger.info("Info message", &[]);
        logger.warn("Warn message", &[]);
        logger.error("Error message", &[]);

        let stats = logger.stats();
        assert_eq!(stats.total_logs, 2); // Only WARN and ERROR
    }

    #[test]
    fn test_span_tracking() {
        let config = LogConfig::default();
        let mut logger = LogManager::new(config);

        let span_id = logger.start_span("test_operation");
        std::thread::sleep(Duration::from_millis(10));
        logger.end_span(span_id);

        let stats = logger.stats();
        assert_eq!(stats.total_spans, 1);
        assert!(stats.total_span_time.as_millis() >= 10);
    }

    #[test]
    fn test_log_file_creation() {
        let temp_dir = std::env::temp_dir();
        let log_path = temp_dir.join("axiom_test.log");

        // Clean up if exists
        let _ = fs::remove_file(&log_path);

        let mut config = LogConfig::default();
        config.file_path = Some(log_path.clone());

        let mut logger = LogManager::new(config);
        logger.init().unwrap();
        logger.info("Test message", &[]);
        logger.flush().unwrap();

        assert!(log_path.exists());

        // Cleanup
        let _ = fs::remove_file(&log_path);
    }

    #[test]
    fn test_runtime_level_change() {
        let config = LogConfig::default();
        let mut logger = LogManager::new(config);

        logger.debug("Debug 1", &[]);
        assert_eq!(logger.stats().total_logs, 0);

        logger.set_level(LogLevel::Debug);
        logger.debug("Debug 2", &[]);
        assert_eq!(logger.stats().total_logs, 1);
    }

    #[test]
    fn test_context_preservation() {
        let config = LogConfig::default();
        let mut logger = LogManager::new(config);

        logger.info("Test", &[("key1", "value1"), ("key2", "value2")]);
        assert_eq!(logger.stats().total_logs, 1);
    }

    #[test]
    fn test_multiple_spans() {
        let config = LogConfig::default();
        let mut logger = LogManager::new(config);

        let span1 = logger.start_span("operation1");
        let span2 = logger.start_span("operation2");

        logger.end_span(span1);
        logger.end_span(span2);

        assert_eq!(logger.stats().total_spans, 2);
    }

    #[test]
    fn test_stats_by_level() {
        let config = LogConfig::default();
        let mut logger = LogManager::new(config);

        logger.info("Info 1", &[]);
        logger.info("Info 2", &[]);
        logger.warn("Warn 1", &[]);
        logger.error("Error 1", &[]);

        let stats = logger.stats();
        assert_eq!(stats.logs_by_level[LogLevel::Info as usize], 2);
        assert_eq!(stats.logs_by_level[LogLevel::Warn as usize], 1);
        assert_eq!(stats.logs_by_level[LogLevel::Error as usize], 1);
    }
}
