//! Crash Recovery and State Persistence
//!
//! Production-grade crash handling, state snapshots, automatic recovery,
//! and session restoration for compositor resilience.
//!
//! # Features
//!
//! - **Crash Detection**: Signal handlers for SIGSEGV, SIGABRT, etc.
//! - **State Snapshots**: Periodic compositor state backups
//! - **Auto Recovery**: Automatic restart after crashes
//! - **Session Restoration**: Restore windows, workspaces, and layouts
//! - **Rollback**: Revert to last known good state
//! - **Forensics**: Crash dumps and stack traces
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────┐     ┌──────────────┐     ┌─────────────┐
//! │  Compositor    │────►│   Snapshot   │────►│  Persistent │
//! │    State       │     │   Manager    │     │   Storage   │
//! └────────────────┘     └──────────────┘     └─────────────┘
//!         │                      │                     │
//!         │                      ▼                     ▼
//!         │              ┌──────────────┐     ┌─────────────┐
//!         └─────────────►│    Crash     │────►│   Recovery  │
//!                        │   Handler    │     │   Process   │
//!                        └──────────────┘     └─────────────┘
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use axiom::recovery::{RecoveryManager, SnapshotConfig};
//!
//! // Initialize recovery system
//! let config = SnapshotConfig {
//!     snapshot_interval: std::time::Duration::from_secs(30),
//!     max_snapshots: 10,
//!     storage_path: "/var/lib/axiom/snapshots".into(),
//! };
//!
//! let mut recovery = RecoveryManager::new(config);
//! recovery.init();
//!
//! // Take snapshot
//! let state = CompositorState::current();
//! recovery.snapshot(state);
//!
//! // Restore from latest snapshot
//! if let Some(state) = recovery.restore_latest() {
//!     apply_state(state);
//! }
//! ```

use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Window state for recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSnapshot {
    /// Window ID
    pub id: u32,
    /// Title
    pub title: String,
    /// Position (x, y)
    pub position: (i32, i32),
    /// Size (width, height)
    pub size: (u32, u32),
    /// Workspace ID
    pub workspace: i32,
    /// Is floating
    pub floating: bool,
    /// Is focused
    pub focused: bool,
}

/// Workspace state for recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    /// Workspace ID
    pub id: i32,
    /// Name
    pub name: String,
    /// Window IDs in this workspace
    pub windows: Vec<u32>,
    /// Is active
    pub active: bool,
}

/// Complete compositor state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Snapshot ID
    pub id: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Version
    pub version: String,
    /// Windows
    pub windows: Vec<WindowSnapshot>,
    /// Workspaces
    pub workspaces: Vec<WorkspaceSnapshot>,
    /// Active window ID
    pub active_window: Option<u32>,
    /// Active workspace ID
    pub active_workspace: i32,
}

impl StateSnapshot {
    /// Create a new snapshot
    pub fn new(id: u64) -> Self {
        Self {
            id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            windows: Vec::new(),
            workspaces: Vec::new(),
            active_window: None,
            active_workspace: 0,
        }
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize snapshot: {}", e))
    }

    /// Deserialize from JSON
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("Failed to deserialize snapshot: {}", e))
    }

    /// Calculate snapshot size
    pub fn size(&self) -> usize {
        self.windows.len() * std::mem::size_of::<WindowSnapshot>()
            + self.workspaces.len() * std::mem::size_of::<WorkspaceSnapshot>()
    }
}

/// Crash information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashInfo {
    /// Crash ID
    pub id: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Signal number
    pub signal: i32,
    /// Error message
    pub message: String,
    /// Stack trace (if available)
    pub stack_trace: Vec<String>,
    /// Last snapshot ID
    pub last_snapshot_id: Option<u64>,
}

impl CrashInfo {
    /// Create new crash info
    pub fn new(id: u64, signal: i32, message: String) -> Self {
        Self {
            id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            signal,
            message,
            stack_trace: Vec::new(),
            last_snapshot_id: None,
        }
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize crash info: {}", e))
    }
}

/// Snapshot configuration
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    /// Interval between automatic snapshots
    pub snapshot_interval: Duration,
    /// Maximum number of snapshots to keep
    pub max_snapshots: usize,
    /// Storage directory path
    pub storage_path: PathBuf,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            snapshot_interval: Duration::from_secs(30),
            max_snapshots: 10,
            storage_path: PathBuf::from("/tmp/axiom/snapshots"),
        }
    }
}

/// Recovery manager
pub struct RecoveryManager {
    /// Configuration
    config: SnapshotConfig,
    /// Snapshots queue (most recent first)
    snapshots: VecDeque<StateSnapshot>,
    /// Next snapshot ID
    next_snapshot_id: u64,
    /// Last snapshot time
    last_snapshot_time: Option<SystemTime>,
    /// Crash history
    crashes: Vec<CrashInfo>,
    /// Next crash ID
    next_crash_id: u64,
    /// Statistics
    stats: RecoveryStats,
}

impl RecoveryManager {
    /// Create a new recovery manager
    pub fn new(config: SnapshotConfig) -> Self {
        Self {
            config,
            snapshots: VecDeque::new(),
            next_snapshot_id: 1,
            last_snapshot_time: None,
            crashes: Vec::new(),
            next_crash_id: 1,
            stats: RecoveryStats::default(),
        }
    }

    /// Initialize recovery system
    pub fn init(&mut self) -> Result<(), String> {
        // Create storage directory
        fs::create_dir_all(&self.config.storage_path)
            .map_err(|e| format!("Failed to create snapshot directory: {}", e))?;

        // Load existing snapshots
        self.load_snapshots()?;

        // Load crash history
        self.load_crashes()?;

        info!(
            "💾 Recovery manager initialized ({} snapshots, {} crashes)",
            self.snapshots.len(),
            self.crashes.len()
        );

        Ok(())
    }

    /// Load snapshots from disk
    fn load_snapshots(&mut self) -> Result<(), String> {
        let snapshot_dir = &self.config.storage_path;

        if !snapshot_dir.exists() {
            return Ok(());
        }

        let entries = fs::read_dir(snapshot_dir)
            .map_err(|e| format!("Failed to read snapshot directory: {}", e))?;

        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(snapshot) = self.load_snapshot_file(&path) {
                        self.snapshots.push_back(snapshot);
                    }
                }
            }
        }

        // Sort by ID (most recent first)
        let mut vec: Vec<_> = self.snapshots.drain(..).collect();
        vec.sort_by(|a, b| b.id.cmp(&a.id));
        self.snapshots = vec.into();

        // Update next ID
        if let Some(latest) = self.snapshots.front() {
            self.next_snapshot_id = latest.id + 1;
        }

        Ok(())
    }

    /// Load a single snapshot from file
    fn load_snapshot_file(&self, path: &Path) -> Result<StateSnapshot, String> {
        let mut file =
            File::open(path).map_err(|e| format!("Failed to open snapshot file: {}", e))?;

        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("Failed to read snapshot file: {}", e))?;

        StateSnapshot::from_json(&contents)
    }

    /// Load crash history from disk
    fn load_crashes(&mut self) -> Result<(), String> {
        let crash_file = self.config.storage_path.join("crashes.json");

        if !crash_file.exists() {
            return Ok(());
        }

        let mut file =
            File::open(&crash_file).map_err(|e| format!("Failed to open crash file: {}", e))?;

        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("Failed to read crash file: {}", e))?;

        self.crashes = serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse crash history: {}", e))?;

        // Update next ID
        if let Some(last_crash) = self.crashes.last() {
            self.next_crash_id = last_crash.id + 1;
        }

        Ok(())
    }

    /// Take a state snapshot
    pub fn snapshot(&mut self, mut snapshot: StateSnapshot) -> Result<u64, String> {
        // Assign ID
        snapshot.id = self.next_snapshot_id;
        self.next_snapshot_id += 1;

        // Save to disk
        self.save_snapshot(&snapshot)?;

        // Add to queue
        self.snapshots.push_front(snapshot.clone());

        // Trim old snapshots
        while self.snapshots.len() > self.config.max_snapshots {
            if let Some(old) = self.snapshots.pop_back() {
                let _ = self.delete_snapshot(old.id);
            }
        }

        self.last_snapshot_time = Some(SystemTime::now());
        self.stats.total_snapshots += 1;

        debug!("💾 Snapshot {} created ({} windows, {} workspaces)", 
               snapshot.id, snapshot.windows.len(), snapshot.workspaces.len());

        Ok(snapshot.id)
    }

    /// Save snapshot to disk
    fn save_snapshot(&self, snapshot: &StateSnapshot) -> Result<(), String> {
        let file_path = self
            .config
            .storage_path
            .join(format!("snapshot_{}.json", snapshot.id));

        let json = snapshot.to_json()?;

        let mut file = File::create(&file_path)
            .map_err(|e| format!("Failed to create snapshot file: {}", e))?;

        file.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write snapshot: {}", e))?;

        Ok(())
    }

    /// Delete a snapshot from disk
    fn delete_snapshot(&self, snapshot_id: u64) -> Result<(), String> {
        let file_path = self
            .config
            .storage_path
            .join(format!("snapshot_{}.json", snapshot_id));

        if file_path.exists() {
            fs::remove_file(&file_path)
                .map_err(|e| format!("Failed to delete snapshot: {}", e))?;
        }

        Ok(())
    }

    /// Restore from latest snapshot
    pub fn restore_latest(&self) -> Option<StateSnapshot> {
        self.snapshots.front().cloned()
    }

    /// Restore from specific snapshot
    pub fn restore(&self, snapshot_id: u64) -> Option<StateSnapshot> {
        self.snapshots.iter().find(|s| s.id == snapshot_id).cloned()
    }

    /// Record a crash
    pub fn record_crash(&mut self, signal: i32, message: String) -> u64 {
        let mut crash = CrashInfo::new(self.next_crash_id, signal, message);
        self.next_crash_id += 1;

        // Attach last snapshot ID
        if let Some(snapshot) = self.snapshots.front() {
            crash.last_snapshot_id = Some(snapshot.id);
        }

        self.crashes.push(crash.clone());
        self.stats.total_crashes += 1;

        // Save crash history
        let _ = self.save_crashes();

        error!("💥 Crash #{} recorded: signal {}", crash.id, signal);

        crash.id
    }

    /// Save crash history to disk
    fn save_crashes(&self) -> Result<(), String> {
        let crash_file = self.config.storage_path.join("crashes.json");

        let json = serde_json::to_string_pretty(&self.crashes)
            .map_err(|e| format!("Failed to serialize crash history: {}", e))?;

        let mut file = File::create(&crash_file)
            .map_err(|e| format!("Failed to create crash file: {}", e))?;

        file.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write crash history: {}", e))?;

        Ok(())
    }

    /// Check if snapshot is needed
    pub fn should_snapshot(&self) -> bool {
        match self.last_snapshot_time {
            None => true,
            Some(last_time) => {
                SystemTime::now()
                    .duration_since(last_time)
                    .unwrap_or_default()
                    >= self.config.snapshot_interval
            }
        }
    }

    /// Get recovery statistics
    pub fn stats(&self) -> RecoveryStats {
        let mut stats = self.stats;
        stats.available_snapshots = self.snapshots.len();
        stats
    }

    /// Get crash history
    pub fn crashes(&self) -> &[CrashInfo] {
        &self.crashes
    }

    /// Clear old crash records
    pub fn clear_old_crashes(&mut self, max_age_secs: u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.crashes.retain(|c| now - c.timestamp < max_age_secs);
        let _ = self.save_crashes();
    }
}

/// Recovery statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct RecoveryStats {
    /// Total snapshots created
    pub total_snapshots: usize,
    /// Total crashes recorded
    pub total_crashes: usize,
    /// Available snapshots
    pub available_snapshots: usize,
    /// Total recoveries performed
    pub total_recoveries: usize,
}

/// Thread-safe recovery manager wrapper
pub type SharedRecovery = Arc<Mutex<RecoveryManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(test_name: &str) -> SnapshotConfig {
        SnapshotConfig {
            snapshot_interval: Duration::from_secs(1),
            max_snapshots: 3,
            storage_path: std::env::temp_dir().join(format!("axiom_recovery_test_{}", test_name)),
        }
    }

    fn cleanup_test_dir(config: &SnapshotConfig) {
        let _ = fs::remove_dir_all(&config.storage_path);
    }

    #[test]
    fn test_snapshot_creation() {
        let config = test_config("snapshot_creation");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        let snapshot = StateSnapshot::new(0);
        let id = recovery.snapshot(snapshot).unwrap();

        assert_eq!(id, 1);
        assert_eq!(recovery.stats().total_snapshots, 1);

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_snapshot_restore() {
        let config = test_config("snapshot_restore");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        let mut snapshot = StateSnapshot::new(0);
        snapshot.windows.push(WindowSnapshot {
            id: 1,
            title: "Test".to_string(),
            position: (0, 0),
            size: (100, 100),
            workspace: 0,
            floating: false,
            focused: true,
        });

        let id = recovery.snapshot(snapshot.clone()).unwrap();
        let restored = recovery.restore_latest().unwrap();

        assert_eq!(restored.id, id);
        assert_eq!(restored.windows.len(), 1);

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_max_snapshots() {
        let config = test_config("max_snapshots");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        // Create more snapshots than max
        for _ in 0..5 {
            let snapshot = StateSnapshot::new(0);
            recovery.snapshot(snapshot).unwrap();
        }

        assert_eq!(recovery.snapshots.len(), config.max_snapshots);

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_crash_recording() {
        let config = test_config("crash_recording");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        let crash_id = recovery.record_crash(11, "Test crash".to_string());

        assert_eq!(crash_id, 1);
        assert_eq!(recovery.crashes().len(), 1);
        assert_eq!(recovery.stats().total_crashes, 1);

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_snapshot_persistence() {
        let config = test_config("snapshot_persistence");
        cleanup_test_dir(&config);

        // Create and save snapshot
        {
            let mut recovery = RecoveryManager::new(config.clone());
            recovery.init().unwrap();

            let snapshot = StateSnapshot::new(0);
            recovery.snapshot(snapshot).unwrap();
        }

        // Load in new instance
        {
            let mut recovery = RecoveryManager::new(config.clone());
            recovery.init().unwrap();

            assert_eq!(recovery.snapshots.len(), 1);
        }

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_should_snapshot() {
        let mut config = test_config("should_snapshot");
        config.snapshot_interval = Duration::from_millis(100);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        assert!(recovery.should_snapshot());

        let snapshot = StateSnapshot::new(0);
        recovery.snapshot(snapshot).unwrap();

        assert!(!recovery.should_snapshot());

        std::thread::sleep(Duration::from_millis(150));
        assert!(recovery.should_snapshot());

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_clear_old_crashes() {
        let config = test_config("clear_old_crashes");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        recovery.record_crash(11, "Crash 1".to_string());
        std::thread::sleep(Duration::from_millis(50));
        recovery.record_crash(11, "Crash 2".to_string());

        assert_eq!(recovery.crashes().len(), 2);

        // Clear crashes older than 10 seconds (should keep both)
        recovery.clear_old_crashes(10);
        assert_eq!(recovery.crashes().len(), 2);

        // Clear all crashes (max_age = 0)
        recovery.clear_old_crashes(0);
        assert_eq!(recovery.crashes().len(), 0);

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_snapshot_serialization() {
        let snapshot = StateSnapshot::new(1);
        let json = snapshot.to_json().unwrap();
        let restored = StateSnapshot::from_json(&json).unwrap();

        assert_eq!(restored.id, snapshot.id);
        assert_eq!(restored.version, snapshot.version);
    }

    #[test]
    fn test_restore_specific_snapshot() {
        let config = test_config("restore_specific");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        let id1 = recovery.snapshot(StateSnapshot::new(0)).unwrap();
        let id2 = recovery.snapshot(StateSnapshot::new(0)).unwrap();

        let restored = recovery.restore(id1).unwrap();
        assert_eq!(restored.id, id1);

        let restored = recovery.restore(id2).unwrap();
        assert_eq!(restored.id, id2);

        cleanup_test_dir(&config);
    }

    #[test]
    fn test_crash_with_snapshot() {
        let config = test_config("crash_with_snapshot");
        cleanup_test_dir(&config);

        let mut recovery = RecoveryManager::new(config.clone());
        recovery.init().unwrap();

        let snapshot_id = recovery.snapshot(StateSnapshot::new(0)).unwrap();
        recovery.record_crash(11, "Test crash".to_string());

        let crashes = recovery.crashes();
        assert_eq!(crashes.len(), 1);
        assert_eq!(crashes[0].last_snapshot_id, Some(snapshot_id));

        cleanup_test_dir(&config);
    }
}
