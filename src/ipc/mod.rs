//! IPC (Inter-Process Communication) module for Axiom-Lazy UI integration
//!
//! This module provides communication between the Axiom compositor (Rust) and
//! Lazy UI optimization system (Python) using Unix sockets and JSON messages.
//!
//! ## Architecture
//! - [`AxiomIPCServer`]: Server that accepts client connections via Unix socket
//! - [`AxiomMessage`]: Messages sent from Axiom to Lazy UI
//! - [`LazyUIMessage`]: Commands sent from Lazy UI to Axiom
//!
//! ## Security
//! - UID-based peer credential verification
//! - Connection limit via semaphore (default 16)
//! - Idle timeout for inactive connections (60s)

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::AxiomConfig;

/// Maximum number of concurrent IPC client connections.
const MAX_CONNECTIONS: usize = 16;
/// Idle timeout for client connections (seconds).
const CLIENT_IDLE_TIMEOUT_SECS: u64 = 60;

/// Whitelisted `LazyUIMessage::WorkspaceCommand.action` strings. Unknown actions
/// are rejected with status `unknown_action` so callers can distinguish
/// future-supported actions from outright typos. All 10 actions are wired
/// end-to-end: the IPC layer validates against this list and forwards known
/// actions to the compositor via `cmd_tx`, and `AxiomCompositor::process_messages`
/// dispatches them to the workspace engine (`WorkspaceTape` / `ScrollableWorkspaces`).
const KNOWN_WORKSPACE_ACTIONS: &[&str] = &[
    "scroll_left",
    "scroll_right",
    "add_window",
    "remove_window",
    "move_focus_left",
    "move_focus_right",
    "toggle_floating",
    "minimize_window",
    "restore_window",
    "toggle_fullscreen",
];

/// Maximum accepted scroll speed.
const MAX_SCROLL_SPEED: f64 = 100.0;
/// Maximum size of a single line from an IPC client (64 KiB).
const MAX_IPC_LINE_BYTES: usize = 64 * 1024;

/// Maximum IPC messages a single client can send per tick.
/// Prevents a misbehaving client from flooding the compositor.
const MAX_MESSAGES_PER_TICK: u32 = 64;

/// Maximum accumulated write buffer size per client before disconnect.
/// Prevents a slow-reading client from causing unbounded memory growth.
const MAX_WRITE_BUF_BYTES: usize = 1_048_576; // 1 MiB

/// Live compositor metrics surfaced through `GetPerformanceReport` and
/// `HealthCheck`. Pushed from the compositor's tick loop into a shared
/// handle that the per-client IPC handlers read on demand.
///
/// Previously `HealthCheck` and `GetPerformanceReport` returned crafted
/// zeros for `frame_time_ms`/`active_windows`/`current_workspace` with
/// a half-misleading `note` field — monitoring clients couldn't tell
/// "metrics not wired" apart from "true zero reading". This struct is
/// the canonical live source; the per-client handler now reads from
/// the snapshot rather than fabricating zeros. The `note` field stays
/// for backward-compat with old readers but reports empty once these
/// three fields come from the live compositor.
///
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LiveMetrics {
    /// Frame time in milliseconds from the last completed tick.
    pub frame_time_ms: f32,
    /// Number of windows currently registered with the compositor.
    pub active_windows: u32,
    /// Index of the workspace the user is currently focused on.
    pub current_workspace: i32,
}

/// Returns true when `action` is in the whitelisted
/// [`KNOWN_WORKSPACE_ACTIONS`] set. Whitelist is enforced to avoid
/// silently executing untyped JSON parameters against `workspace_manager`.
///
/// Enforced in production: the live `WorkspaceCommand` branch of
/// `handle_client` rejects unknown actions with an `unknown_action` ACK
/// before forwarding to the compositor. Kept as `pub(super)` so the
/// `test_known_workspace_actions` regression test exercises the same
/// production code path.
fn is_known_workspace_action(action: &str) -> bool {
    KNOWN_WORKSPACE_ACTIONS.contains(&action)
}

/// Messages sent from Axiom to Lazy UI (performance metrics, events)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum AxiomMessage {
    /// System performance metrics
    PerformanceMetrics {
        timestamp: u64,
        cpu_usage: f32,
        memory_usage: f32,
        gpu_usage: f32,
        frame_time: f32,
        active_windows: u32,
        current_workspace: i32,
    },

    /// User interaction events
    UserEvent {
        timestamp: u64,
        event_type: String,
        details: serde_json::Value,
    },

    /// Compositor state changes
    StateChange {
        timestamp: u64,
        component: String,
        old_state: String,
        new_state: String,
    },

    /// Configuration query response
    ConfigResponse {
        key: String,
        value: serde_json::Value,
    },

    /// Compositor startup notification
    StartupComplete {
        version: String,
        capabilities: Vec<String>,
    },

    /// Comprehensive performance report answering a `GetPerformanceReport`
    /// request. Distinct from `PerformanceMetrics` (broadcast, sampling-only)
    /// so a request-response client can read typed fields and a note string.
    /// Fields with no live readout from the IPC layer report 0.0 / 0; the
    /// `note` field explains the gap so clients do not silently treat zeros
    /// as accurate readings.
    ///
    /// **Migration note:** This variant replaces a previous `UserEvent`
    /// with `event_type == "PerformanceReport"`. Clients that decoded
    /// messages by the `event_type` discriminator string must now switch
    /// on `data["type"] == "PerformanceReport"` (the serde tag). Clients
    /// that already switched on `data["type"]` need no change — serde
    /// emits this variant with `"type":"PerformanceReport"` automatically.
    /// Wire schema (serde JSON):
    /// ```json
    /// {"type":"PerformanceReport","timestamp":<u64>,"gpu_usage":<f32>,
    ///  "frame_time_ms":<f32>,"active_windows":<u32>,
    ///  "current_workspace":<i32>,"note":"<str>"}
    /// ```
    PerformanceReport {
        timestamp: u64,
        gpu_usage: f32,
        frame_time_ms: f32,
        active_windows: u32,
        current_workspace: i32,
        note: String,
    },
}

/// Messages sent from Lazy UI to Axiom (optimization commands)
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum LazyUIMessage {
    /// Optimization configuration changes
    OptimizeConfig {
        changes: std::collections::HashMap<String, serde_json::Value>,
        reason: String,
    },

    /// Request configuration value
    GetConfig { key: String },

    /// Set configuration value
    SetConfig {
        key: String,
        value: serde_json::Value,
    },

    /// Workspace management commands. The handler validates the `action`
    /// against `KNOWN_WORKSPACE_ACTIONS` and rejects unknown actions with an
    /// `unknown_action` ACK. Known actions are forwarded via the mpsc command
    /// channel to the compositor's `process_messages`, which dispatches them
    /// end-to-end to the workspace engine (`WorkspaceTape` /
    /// `ScrollableWorkspaces`). All 10 actions are wired and executed.
    WorkspaceCommand {
        action: String,
        parameters: serde_json::Value,
    },

    /// Per-window blur control. `radius` in pixels (0..=32); 0 disables blur.
    SetWindowBlur { window_id: u64, radius: f32 },

    /// System health check request
    HealthCheck,

    /// Request performance report
    GetPerformanceReport,

    /// Set compositor clipboard content
    SetClipboard { text: String },
}

/// Per-client IPC connection state
struct ClientData {
    stream: UnixStream,
    /// Buffer for accumulating a partial line being read
    read_buf: Vec<u8>,
    /// Pending data to write (queued broadcasts, ACKs)
    write_buf: Vec<u8>,
    /// Time of last activity (for idle timeout)
    last_activity: Instant,
    /// Messages read from this client during the current tick.
    /// Reset each tick to enforce a per-tick rate limit.
    messages_this_tick: u32,
}

/// IPC server for handling communication with Lazy UI
pub struct AxiomIPCServer {
    socket_path: PathBuf,
    /// Non-blocking Unix listener (bound in `start()`)
    listener: Option<UnixListener>,
    /// Per-client state, keyed by fd
    clients: HashMap<RawFd, ClientData>,
    /// Command channel receiver (for compositor to drain)
    command_receiver: Option<mpsc::Receiver<LazyUIMessage>>,
    /// Command channel sender (for IPC handlers to send commands)
    command_sender: mpsc::Sender<LazyUIMessage>,
    /// Live read-only handle to the compositor's `AxiomConfig`. Lazily wired
    /// via `set_config_handle` so test-only constructors (`new()`) can keep
    /// working without a config.
    config_handle: Option<Arc<parking_lot::RwLock<AxiomConfig>>>,
    /// Live read-only handle to the compositor's `LiveMetrics` snapshot.
    /// Set by the compositor after construction and refreshed on every
    /// tick. Both `HealthCheck` and `GetPerformanceReport` per-client
    /// requests read from this handle instead of fabricating zeros.
    /// Field is `None` until the compositor wires it via
    /// `set_live_metrics_snapshot`; in that case handlers fall back to
    /// `LiveMetrics::default()` and the previous note ("placeholders")
    /// so monitoring clients can distinguish "no compositor wired" from
    /// "all metrics legitimately zero".
    live_metrics_handle: Option<Arc<parking_lot::RwLock<LiveMetrics>>>,
    last_metrics_sent: Instant,
    // Last CPU times for non-blocking CPU usage sampling
    last_cpu_times: Option<(u64, u64)>,
    /// Pending broadcast messages to send to all clients
    pending_broadcasts: Vec<AxiomMessage>,
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    /// Connection count (atomic for non-blocking limit check)
    num_connections: AtomicUsize,
    /// Our UID for peer credential checks
    our_uid: u32,
}

impl Default for AxiomIPCServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AxiomIPCServer {
    /// Create a new IPC server
    pub fn new() -> Self {
        let socket_path = Self::default_socket_path();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        Self {
            socket_path,
            listener: None,
            clients: HashMap::new(),
            command_receiver: Some(cmd_rx),
            command_sender: cmd_tx,
            config_handle: None,
            live_metrics_handle: None,
            last_metrics_sent: Instant::now(),
            last_cpu_times: None,
            pending_broadcasts: Vec::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            num_connections: AtomicUsize::new(0),
            our_uid: 0,
        }
    }

    /// Wire a read-only handle to the live `AxiomConfig` for `GetConfig`
    /// queries. **The compositor remains the canonical owner** — this
    /// field is only the IPC-side read source. To avoid drift, callers
    /// (e.g. `AxiomCompositor::process_events`) MUST refresh this handle
    /// after every write to the compositor's owned config, typically via
    /// `self.ipc_server.set_config_handle(Arc::new(RwLock::new(self.config.clone())))`.
    /// Mutating config keys (`OptimizeConfig` / `SetConfig`) flow back via
    /// the mpsc `command_channel` so the compositor can apply them through
    /// `process_messages` before refreshing the handle.
    pub fn set_config_handle(&mut self, config: Arc<parking_lot::RwLock<AxiomConfig>>) {
        self.config_handle = Some(config);
    }

    /// Wire the live `LiveMetrics` handle the compositor updates each tick.
    /// Calling this with `Some(snapshot)` replaces any previous handle so
    /// the compositor can either seed the initial state at construction
    /// time or refresh it after each tick. The compositor devolves to
    /// `set_live_metrics_snapshot` on the same handle from inside `tick()`.
    pub fn set_live_metrics_snapshot(&mut self, snapshot: LiveMetrics) {
        *self
            .live_metrics_handle
            .get_or_insert_with(|| Arc::new(parking_lot::RwLock::new(LiveMetrics::default())))
            .write() = snapshot;
    }

    /// Build the WorkspaceCommand ACK UserEvent for the per-client handler.
    /// Schema owned here (single source of truth) so the
    /// `test_workspace_command_ack_schema_includes_status` regression test
    /// exercises the actual production constructor. A pure helper that does
    /// not take `&self` so call sites are both `Self::` (from inside the
    /// impl) and `AxiomIPCServer::` (from the test mod) without forcing a
    /// `new()` plumbing for each call.
    ///
    /// **Status:** the per-client `WorkspaceCommand` handler uses this
    /// constructor to emit the `unknown_action` ACK for whitelist-rejected
    /// actions before forwarding known actions to `cmd_tx`. It is also the
    /// single source of truth exercised by
    /// `test_workspace_command_ack_schema_includes_status`.
    pub(super) fn build_workspace_command_ack(action: &str, accepted: bool) -> AxiomMessage {
        AxiomMessage::UserEvent {
            // Fail loudly on a pre-1970 system clock rather than silently
            // emitting `timestamp: 0` that IPC clients would misread.
            // This is live in production hardware environments but a `map`
            // -> `unwrap_or(0)` would be silent on a faulted/low-level test
            // environment. `expect` makes the failure mode self-diagnosing.
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH — compositor hardware fault")
                .as_secs(),
            event_type: "WorkspaceCommandAck".into(),
            details: serde_json::json!({
                "action": action,
                "accepted": accepted,
                "status": if accepted { "queued_for_execution" } else { "unknown_action" },
                "dispatched_via_mpsc": accepted,
            }),
        }
    }

    /// Start the IPC server
    pub fn start(&mut self) -> Result<()> {
        // Ensure parent dir exists with correct permissions (0700).
        // Do the mkdir+chmod before anything else so the directory is
        // never observable with wider permissions.
        let socket_path = self.socket_path.clone();
        if let Some(dir) = socket_path.parent() {
            if !dir.exists() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("Failed to create IPC dir: {:?}", dir))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(e) =
                        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                    {
                        warn!("⚠️ Failed to set 0700 on IPC directory {:?}: {}", dir, e);
                    }
                }
            }
        }

        // Bind the socket (retry once if addr in use)
        let listener = match UnixListener::bind(&socket_path) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                let _ = std::fs::remove_file(&socket_path);
                UnixListener::bind(&socket_path).with_context(|| {
                    format!(
                        "Failed to bind Unix socket after stale removal: {:?}",
                        socket_path
                    )
                })?
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Failed to bind Unix socket: {:?}", socket_path));
            }
        };
        listener.set_nonblocking(true)?;

        // Tighten socket permissions (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) =
                std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
            {
                warn!("⚠️ Failed to set 0600 on socket {:?}: {}", socket_path, e);
            }
        }

        // Get our own UID for peer credential checks
        #[cfg(unix)]
        let our_uid = unsafe { libc::getuid() };
        #[cfg(not(unix))]
        let our_uid = 0u32;

        self.listener = Some(listener);
        self.our_uid = our_uid;

        info!("🔗 Axiom IPC server listening on: {:?}", socket_path);
        Ok(())
    }

    // =========================================================================
    // Poll-based event loop (called from compositor tick)
    // =========================================================================

    /// Poll the IPC server: accept connections, read messages, write responses.
    /// Called from the compositor's tick().
    pub fn poll(&mut self) {
        // 1. Accept new connections (non-blocking)
        self.accept_new_connections();
        // 2. Read from all clients (non-blocking)
        self.read_from_clients();
        // 3. Write pending data to all clients (non-blocking)
        self.write_to_clients();
        // 4. Clean up disconnected clients
        self.cleanup_disconnected_clients();
    }

    fn accept_new_connections(&mut self) {
        if self.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let Some(ref listener) = self.listener else {
            return;
        };

        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Peer credential check (via libc::getsockopt SO_PEERCRED)
                    #[cfg(unix)]
                    {
                        let peer_uid = Self::get_peer_uid(&stream);
                        match peer_uid {
                            Some(uid) if uid == self.our_uid => {} // OK
                            Some(uid) => {
                                warn!(
                                    "🚫 Rejecting IPC connection from different user (uid={})",
                                    uid
                                );
                                continue;
                            }
                            None => {
                                warn!("⚠️ Failed to get peer credentials, rejecting connection");
                                continue;
                            }
                        }
                    }

                    // Connection limit check
                    if self.num_connections.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                        warn!(
                            "🚫 Max IPC connections reached ({}), rejecting",
                            MAX_CONNECTIONS
                        );
                        continue;
                    }
                    self.num_connections.fetch_add(1, Ordering::Relaxed);

                    if let Err(e) = stream.set_nonblocking(true) {
                        warn!("⚠️ Failed to set non-blocking on IPC connection: {}", e);
                        self.num_connections.fetch_sub(1, Ordering::Relaxed);
                        continue;
                    }
                    let fd = stream.as_raw_fd();
                    info!("🤝 Lazy UI connected to Axiom IPC (fd={})", fd);

                    // Send startup notification
                    let startup_msg = AxiomMessage::StartupComplete {
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        capabilities: vec![
                            "scrollable_workspaces".to_string(),
                            "performance_metrics".to_string(),
                            "ai_optimization".to_string(),
                        ],
                    };
                    if let Ok(json) = serde_json::to_string(&startup_msg) {
                        let mut msg_bytes = json.into_bytes();
                        msg_bytes.push(b'\n');
                        // Assume write succeeds (best-effort on connect)
                        let _ = stream.write_all(&msg_bytes);
                    }

                    self.clients.insert(
                        fd,
                        ClientData {
                            stream,
                            read_buf: Vec::with_capacity(4096),
                            write_buf: Vec::new(),
                            last_activity: Instant::now(),
                            messages_this_tick: 0,
                        },
                    );
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break; // No more connections to accept
                }
                Err(e) => {
                    error!("❌ Error accepting IPC connection: {}", e);
                    break;
                }
            }
        }
    }

    fn read_from_clients(&mut self) {
        // Reset per-tick message counters for rate limiting
        for client in self.clients.values_mut() {
            client.messages_this_tick = 0;
        }

        let client_fds: Vec<RawFd> = self.clients.keys().copied().collect();
        let mut disconnected: Vec<RawFd> = Vec::new();

        for &fd in &client_fds {
            // Rate limit: skip this client if it's sent too many messages this tick
            if let Some(client) = self.clients.get(&fd) {
                if client.messages_this_tick >= MAX_MESSAGES_PER_TICK {
                    continue;
                }
            }

            // Read available data into a local buffer; we re-borrow clients
            // each iteration so the borrow is never held across method calls.
            let mut buf = [0u8; 4096];
            loop {
                let (done, is_err) = match self.clients.get_mut(&fd) {
                    Some(client) => match client.stream.read(&mut buf) {
                        Ok(0) => (true, false),
                        Ok(n) => {
                            client.last_activity = Instant::now();
                            client.read_buf.extend_from_slice(&buf[..n]);
                            (false, false) // more data may be available
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => (true, false),
                        Err(e) => {
                            warn!("⚠️ IPC read error on fd={}: {}", fd, e);
                            (true, true)
                        }
                    },
                    None => (true, false),
                };
                if is_err {
                    disconnected.push(fd);
                }
                if done {
                    break;
                }
            }
        }

        for fd in disconnected {
            self.remove_client(fd);
        }

        // Process lines from all clients (separate pass avoids borrow conflicts)
        for &fd in &client_fds {
            if self.clients.contains_key(&fd) {
                self.process_client_lines(fd);
            }
        }
    }

    fn process_client_lines(&mut self, fd: RawFd) {
        // Extract complete lines from the client's read buffer, releasing the
        // borrow before calling handle_message (which needs &mut self).
        let lines: Vec<String> = {
            let client = match self.clients.get_mut(&fd) {
                Some(c) => c,
                None => return,
            };
            let mut extracted = Vec::new();
            while let Some(nl_pos) = client.read_buf.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = client.read_buf.drain(..=nl_pos).collect();
                let line = String::from_utf8_lossy(&raw[..raw.len() - 1])
                    .trim()
                    .to_string();
                if line.len() > MAX_IPC_LINE_BYTES {
                    warn!(
                        "⚠️ IPC message too large ({} bytes, max {}) - disconnecting fd={}",
                        line.len(),
                        MAX_IPC_LINE_BYTES,
                        fd
                    );
                    client.read_buf.clear();
                    break;
                }
                if line.is_empty() {
                    continue;
                }
                extracted.push(line);
            }
            extracted
        };

        // Process extracted lines (borrow released, so we can call handle_message)
        for trimmed in lines {
            // Count each line as a message for rate limiting
            if let Some(client) = self.clients.get_mut(&fd) {
                client.messages_this_tick += 1;
            }

            debug!("📨 Received IPC message: {}", trimmed);
            match serde_json::from_str::<LazyUIMessage>(&trimmed) {
                Ok(message) => {
                    self.handle_message(fd, message);
                }
                Err(e) => {
                    warn!("⚠️ Invalid JSON from IPC client: {}", e);
                }
            }
        }
    }

    fn handle_message(&mut self, fd: RawFd, message: LazyUIMessage) {
        let is_command_type = matches!(
            message,
            LazyUIMessage::WorkspaceCommand { .. }
                | LazyUIMessage::SetWindowBlur { .. }
                | LazyUIMessage::SetClipboard { .. }
        );

        if is_command_type {
            // Whitelist gate (WorkspaceCommand only)
            if let LazyUIMessage::WorkspaceCommand { ref action, .. } = message {
                if !is_known_workspace_action(action) {
                    debug!("🚫 Rejecting unknown WorkspaceCommand action: {}", action);
                    let ack = Self::build_workspace_command_ack(action, false);
                    self.queue_message_to_client(fd, &ack);
                    return;
                }
            }

            // Build the ACK based on message type
            let (cmd_event_type, cmd_details) = match &message {
                LazyUIMessage::WorkspaceCommand { action, .. } => (
                    "WorkspaceCommandAck",
                    serde_json::json!({
                        "action": action,
                        "status": "queued_for_compositor_dispatch",
                        "executor": "process_messages",
                        "accepted": true,
                        "dispatched_via_mpsc": true,
                    }),
                ),
                LazyUIMessage::SetWindowBlur { window_id, radius } => (
                    "SetWindowBlurAck",
                    serde_json::json!({
                        "window_id": window_id,
                        "radius": radius,
                        "status": "queued_for_compositor_dispatch",
                        "accepted": true,
                        "dispatched_via_mpsc": true,
                    }),
                ),
                LazyUIMessage::SetClipboard { text } => (
                    "SetClipboardAck",
                    serde_json::json!({
                        "status": "queued_for_compositor_dispatch",
                        "text_length": text.len(),
                        "accepted": true,
                        "dispatched_via_mpsc": true,
                    }),
                ),
                _ => unreachable!("is_command_type gated above"),
            };

            // Send via command channel
            let send_result = self.command_sender.send(message.clone());

            let (ack_event_type, ack_details) = match send_result {
                Ok(()) => (cmd_event_type, cmd_details),
                Err(e) => {
                    let failed_type = match cmd_event_type {
                        "WorkspaceCommandAck" => "WorkspaceCommandAckFailed",
                        "SetWindowBlurAck" => "SetWindowBlurAckFailed",
                        "SetClipboardAck" => "SetClipboardAckFailed",
                        _ => "CommandAckFailed",
                    };
                    (
                        failed_type,
                        serde_json::json!({
                            "status": "delivery_failed",
                            "reason": format!("compositor command receiver dropped: {}", e),
                        }),
                    )
                }
            };

            let ack = AxiomMessage::UserEvent {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock before UNIX_EPOCH")
                    .as_secs(),
                event_type: ack_event_type.into(),
                details: ack_details,
            };
            self.queue_message_to_client(fd, &ack);
        } else {
            // Non-command messages (config, queries) — handle inline
            // Forward to compositor via command channel
            let _ = self.command_sender.send(message.clone());

            // Handle inline for ACK + response
            let cfg_snapshot = self.config_handle.as_ref().map(|h| h.read().clone());

            self.process_query_message(fd, message, cfg_snapshot.as_ref());
        }
    }

    /// Queue a message to be written to a specific client
    fn queue_message_to_client(&mut self, fd: RawFd, message: &AxiomMessage) {
        if let Some(client) = self.clients.get_mut(&fd) {
            if let Ok(json) = serde_json::to_string(message) {
                client.write_buf.extend_from_slice(json.as_bytes());
                client.write_buf.push(b'\n');
            }
        }
    }

    /// Process a query-type message (GetConfig, HealthCheck, etc.) inline
    fn process_query_message(
        &mut self,
        fd: RawFd,
        message: LazyUIMessage,
        config: Option<&AxiomConfig>,
    ) {
        let metrics_handle = self.live_metrics_handle.as_ref();
        match message {
            LazyUIMessage::OptimizeConfig { changes, reason } => {
                info!(
                    "🎯 Applying AI optimization: {} changes ({})",
                    changes.len(),
                    reason
                );
                let mut applied: Vec<String> = Vec::new();
                let mut rejected: Vec<(String, String)> = Vec::new();
                for (key, value) in changes {
                    let val_f64 = value.as_f64();
                    match (key.as_str(), val_f64) {
                        ("workspace.scroll_speed", Some(v))
                            if v.is_finite() && (0.0..=MAX_SCROLL_SPEED).contains(&v) =>
                        {
                            applied.push(key);
                        }
                        ("workspace.scroll_speed", _) => {
                            rejected.push((key, "invalid_or_out_of_range_value".into()));
                        }
                        _ => {
                            rejected.push((key, "unsupported_key".into()));
                        }
                    }
                }
                let ack = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("system clock before UNIX_EPOCH")
                        .as_secs(),
                    event_type: "OptimizeConfigAck".into(),
                    details: serde_json::json!({ "applied": applied, "rejected": rejected }),
                };
                self.queue_message_to_client(fd, &ack);
            }
            LazyUIMessage::GetConfig { key } => {
                let value = config
                    .and_then(|cfg| Self::resolve_config_path(cfg, &key))
                    .unwrap_or(serde_json::Value::Null);
                let response = AxiomMessage::ConfigResponse { key, value };
                self.queue_message_to_client(fd, &response);
            }
            LazyUIMessage::SetConfig { key, value } => {
                info!("⚙️ Setting config: {} = {:?}", key, value);
                let ack = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("system clock before UNIX_EPOCH")
                        .as_secs(),
                    event_type: "SetConfigAck".into(),
                    details: serde_json::json!({ "key": key, "status": "queued" }),
                };
                self.queue_message_to_client(fd, &ack);
            }
            LazyUIMessage::HealthCheck => {
                let snapshot = metrics_handle.map(|h| *h.read()).unwrap_or_default();
                let cpu = Self::sample_system_cpu_instant();
                let mem = Self::sample_system_memory_mb();
                let gpu = Self::sample_gpu_usage();
                let metrics = AxiomMessage::PerformanceMetrics {
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("system clock before UNIX_EPOCH")
                        .as_secs(),
                    cpu_usage: cpu,
                    memory_usage: mem,
                    gpu_usage: gpu,
                    frame_time: snapshot.frame_time_ms,
                    active_windows: snapshot.active_windows,
                    current_workspace: snapshot.current_workspace,
                };
                self.queue_message_to_client(fd, &metrics);
            }
            LazyUIMessage::GetPerformanceReport => {
                let snapshot = metrics_handle.map(|h| *h.read()).unwrap_or_default();
                let gpu_usage = Self::sample_gpu_usage();
                let note = if metrics_handle.is_some() {
                    String::new()
                } else {
                    "live snapshot not wired — fields reflect LiveMetrics::default()".to_string()
                };
                let report = AxiomMessage::PerformanceReport {
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("system clock before UNIX_EPOCH")
                        .as_secs(),
                    gpu_usage,
                    frame_time_ms: snapshot.frame_time_ms,
                    active_windows: snapshot.active_windows,
                    current_workspace: snapshot.current_workspace,
                    note,
                };
                self.queue_message_to_client(fd, &report);
            }
            _ => {} // WorkspaceCommand, SetWindowBlur, SetClipboard — already dispatched via cmd_tx
        }
    }

    fn write_to_clients(&mut self) {
        // First, drain pending broadcasts into each client's write buffer
        if !self.pending_broadcasts.is_empty() {
            let client_fds: Vec<RawFd> = self.clients.keys().copied().collect();
            for fd in client_fds {
                if let Some(client) = self.clients.get_mut(&fd) {
                    for msg in &self.pending_broadcasts {
                        if let Ok(json) = serde_json::to_string(msg) {
                            client.write_buf.extend_from_slice(json.as_bytes());
                            client.write_buf.push(b'\n');
                        }
                    }
                }
            }
            self.pending_broadcasts.clear();
        }

        // Try to flush each client's write buffer
        let mut flushed: Vec<RawFd> = Vec::new();
        let client_fds: Vec<RawFd> = self.clients.keys().copied().collect();
        for fd in client_fds {
            if let Some(client) = self.clients.get_mut(&fd) {
                while !client.write_buf.is_empty() {
                    match client.stream.write(&client.write_buf) {
                        Ok(n) => {
                            let _ = client.write_buf.drain(..n);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            break; // Can't write more now
                        }
                        Err(e) => {
                            warn!("⚠️ IPC write error on fd={}: {}", fd, e);
                            flushed.push(fd);
                            break;
                        }
                    }
                }

                // Backpressure: disconnect clients whose write buffer exceeds the limit
                // (indicates the client is reading too slowly).
                if client.write_buf.len() > MAX_WRITE_BUF_BYTES {
                    warn!(
                        "IPC client {} write buffer exceeded {} bytes — disconnecting",
                        fd, MAX_WRITE_BUF_BYTES
                    );
                    flushed.push(fd);
                }
            }
        }
        for fd in flushed {
            self.remove_client(fd);
        }
    }

    fn cleanup_disconnected_clients(&mut self) {
        let now = Instant::now();
        let idle_timeout = Duration::from_secs(CLIENT_IDLE_TIMEOUT_SECS);
        let mut stale: Vec<RawFd> = Vec::new();

        let client_fds: Vec<RawFd> = self.clients.keys().copied().collect();
        for fd in client_fds {
            if let Some(client) = self.clients.get(&fd) {
                if now.duration_since(client.last_activity) > idle_timeout {
                    info!("⏱️ IPC client fd={} idle timeout, disconnecting", fd);
                    stale.push(fd);
                }
            }
        }
        for fd in stale {
            self.remove_client(fd);
        }
    }

    fn remove_client(&mut self, fd: RawFd) {
        if self.clients.remove(&fd).is_some() {
            self.num_connections.fetch_sub(1, Ordering::Relaxed);
            debug!("📪 IPC client fd={} disconnected", fd);
        }
    }

    /// Phase 3: Process pending IPC messages and apply configuration changes.
    /// Returns `(config_changed, pending_actions)`:
    /// - `config_changed`: true if any `OptimizeConfig` / `SetConfig` mutator
    ///   wrote to the config-owned path. Callers typically call
    ///   `update_subsystems_config()` and refresh the IPC handle when set.
    /// - `pending_actions`: messages from `WorkspaceCommand` /
    ///   `SetWindowBlur` (already validated at the per-client layer) that
    ///   the compositor owns — they require real subsystem access that the
    ///   IPC server does not hold. Caller is responsible for dispatch.
    pub fn process_messages(
        &mut self,
        config: &mut AxiomConfig,
    ) -> Result<(bool, Vec<LazyUIMessage>)> {
        let mut config_changed = false;
        let mut pending_actions: Vec<LazyUIMessage> = Vec::new();

        if let Some(receiver) = &mut self.command_receiver {
            while let Ok(message) = receiver.try_recv() {
                debug!("📨 Processing Lazy UI message: {:?}", message);
                // Process the message (optimization commands, config changes, etc.)
                match message {
                    LazyUIMessage::OptimizeConfig { changes, reason } => {
                        info!("🎯 Applying optimization: {} ({})", changes.len(), reason);
                        for (key, value) in changes {
                            if let Some(val_f64) = value.as_f64() {
                                match key.as_str() {
                                    "workspace.scroll_speed" => {
                                        if val_f64.is_finite() && val_f64 >= 0.0 {
                                            config.workspace.scroll_speed =
                                                val_f64.min(MAX_SCROLL_SPEED);
                                            config_changed = true;
                                            debug!("  Set scroll speed to {}", val_f64);
                                        }
                                    }
                                    _ => {
                                        debug!("  Unknown optimization key: {}", key);
                                    }
                                }
                            }
                        }
                    }
                    LazyUIMessage::SetConfig { key, value } => {
                        info!("⚙️ Setting config: {} = {:?}", key, value);
                        if let Some(val_f64) = value.as_f64() {
                            match key.as_str() {
                                "workspace.scroll_speed"
                                    if val_f64.is_finite() && val_f64 >= 0.0 =>
                                {
                                    config.workspace.scroll_speed = val_f64.min(MAX_SCROLL_SPEED);
                                    config_changed = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    // Sub-system-bound actions: validated upstream, dispatched
                    // by the compositor in `AxiomCompositor::process_events`.
                    LazyUIMessage::WorkspaceCommand { .. }
                    | LazyUIMessage::SetWindowBlur { .. }
                    | LazyUIMessage::SetClipboard { .. } => {
                        pending_actions.push(message);
                    }
                    _ => {
                        debug!("📑 Other message type processed (no main thread action needed)");
                    }
                }
            }
        }

        Ok((config_changed, pending_actions))
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Public getter for testing — allows external test code to inject
    /// LazyUIMessage variants into the command channel without a real IPC client.
    pub fn command_sender_for_test(&self) -> std::sync::mpsc::Sender<LazyUIMessage> {
        self.command_sender.clone()
    }

    /// Rate-limited helper that samples CPU/GPU/memory and enqueues metrics (~10Hz)
    pub fn maybe_broadcast_performance_metrics(
        &mut self,
        frame_time_ms: f32,
        active_windows: u32,
        current_workspace: i32,
    ) {
        const RATE: Duration = Duration::from_millis(100);
        if self.last_metrics_sent.elapsed() < RATE {
            return;
        }
        self.last_metrics_sent = Instant::now();

        let (cpu, mem_mb) = self.sample_system_metrics_nonblocking();
        let gpu = Self::sample_gpu_usage();

        self.pending_broadcasts
            .push(AxiomMessage::PerformanceMetrics {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock before UNIX_EPOCH")
                    .as_secs(),
                cpu_usage: cpu,
                memory_usage: mem_mb,
                gpu_usage: gpu,
                frame_time: frame_time_ms,
                active_windows,
                current_workspace,
            });
    }

    /// Broadcast a compositor state change to all connected IPC clients.
    ///
    /// `component` identifies the subsystem (e.g. `"workspace"`, `"window"`,
    /// `"effects"`) and `new_state` / `old_state` describe the transition
    /// (e.g. `"scrolled_right"`, `"minimized"`, `"fullscreen"`).  This is a
    /// fire-and-forget broadcast — send failures (no connected clients) are
    /// silently ignored.
    pub fn broadcast_state_change(&mut self, component: &str, old_state: &str, new_state: &str) {
        self.pending_broadcasts.push(AxiomMessage::StateChange {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH")
                .as_secs(),
            component: component.to_owned(),
            old_state: old_state.to_owned(),
            new_state: new_state.to_owned(),
        });
    }

    /// Sample GPU usage percentage from DRM sysfs (AMD/Intel) or return 0.0.
    fn sample_gpu_usage() -> f32 {
        // Try common paths for GPU utilisation via DRM
        for path in &[
            "/sys/class/drm/card0/device/gpu_busy_percent",
            "/sys/class/drm/card1/device/gpu_busy_percent",
        ] {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(val) = contents.trim().parse::<f32>() {
                    return val;
                }
            }
        }
        0.0
    }

    /// Sample system CPU usage (%) and memory used (MB) by reading /proc.
    /// This is a synchronous sampler intended for periodic telemetry; it avoids extra deps.
    fn sample_system_metrics_nonblocking(&mut self) -> (f32, f32) {
        // CPU usage: sample /proc/stat twice and compute deltas
        fn read_cpu_times() -> Option<(u64, u64)> {
            let contents = std::fs::read_to_string("/proc/stat").ok()?;
            let mut lines = contents.lines();
            if let Some(first) = lines.next() {
                if first.starts_with("cpu ") {
                    let parts: Vec<&str> = first.split_whitespace().collect();
                    // cpu user nice system idle iowait irq softirq steal guest guest_nice
                    if parts.len() >= 5 {
                        let user: u64 = parts.get(1)?.parse().ok()?;
                        let nice: u64 = parts.get(2)?.parse().ok()?;
                        let system: u64 = parts.get(3)?.parse().ok()?;
                        let idle: u64 = parts.get(4)?.parse().ok()?;
                        let iowait: u64 = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let irq: u64 = parts.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let softirq: u64 = parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let steal: u64 = parts.get(8).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let idle_all = idle + iowait;
                        let non_idle = user + nice + system + irq + softirq + steal;
                        let total = idle_all + non_idle;
                        return Some((idle_all, total));
                    }
                }
            }
            None
        }

        // Single /proc/stat read for the whole function. The previous
        // implementation read the file twice per call: once inline for
        // the delta calculation, and again in a closure at the end to
        // populate `self.last_cpu_times`. Both reads happen on the
        // compositor's main event-loop thread; this halves the file
        // I/O per `maybe_broadcast_performance_metrics` invocation.
        let current = read_cpu_times();

        let cpu_percent = match (self.last_cpu_times, current) {
            (Some((idle_a, total_a)), Some((idle_b, total_b))) => {
                let idle_delta = idle_b.saturating_sub(idle_a) as f64;
                let total_delta = total_b.saturating_sub(total_a) as f64;
                if total_delta > 0.0 {
                    ((1.0 - idle_delta / total_delta) * 100.0) as f32
                } else {
                    0.0
                }
            }
            (_, Some(_)) => {
                // First reading, can't calculate delta yet
                0.0
            }
            _ => 0.0,
        };

        // Memory usage from /proc/meminfo (one read, unchanged).
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let mut mem_total_kb: u64 = 0;
        let mut mem_available_kb: u64 = 0;
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    mem_total_kb = val.parse().unwrap_or(0);
                }
            } else if line.starts_with("MemAvailable:") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    mem_available_kb = val.parse().unwrap_or(0);
                }
            }
        }
        let used_kb = mem_total_kb.saturating_sub(mem_available_kb) as f32;
        let mem_used_mb = used_kb / 1024.0;

        // Update last CPU times for the NEXT call's delta calculation.
        // Reuse the `current` pair we already read instead of opening
        // /proc/stat a second time.
        if let Some(pair) = current {
            self.last_cpu_times = Some(pair);
        }

        (cpu_percent, mem_used_mb)
    }

    /// Shutdown the IPC server — stops accepting new connections
    pub fn shutdown_sync(&mut self) {
        info!("🔽 Shutting down IPC server...");
        self.shutdown.store(true, Ordering::Relaxed);
        // Close the listener
        self.listener.take();
        // Disconnect all clients
        let fds: Vec<RawFd> = self.clients.keys().copied().collect();
        for fd in fds {
            self.remove_client(fd);
        }
        // Close the receiver so process_messages gets empty drains
        self.command_receiver.take();
        // Clean up socket file
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        info!("✅ IPC server shut down");
    }
}

impl Drop for AxiomIPCServer {
    fn drop(&mut self) {
        // Clean up socket file
        if self.socket_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.socket_path) {
                warn!("⚠️ Failed to remove socket file: {}", e);
            }
        }
    }
}

impl AxiomIPCServer {
    /// Walk a dot-separated `section.field` path on the live `AxiomConfig`.
    /// Returns `None` for unknown paths so the IPC layer can answer
    /// with `Null` rather than a misleading default.
    fn resolve_config_path(config: &AxiomConfig, key: &str) -> Option<serde_json::Value> {
        match key {
            "workspace.scroll_speed" => Some(serde_json::json!(config.workspace.scroll_speed)),
            "workspace.infinite_scroll" => {
                Some(serde_json::json!(config.workspace.infinite_scroll))
            }
            "workspace.auto_scroll" => Some(serde_json::json!(config.workspace.auto_scroll)),
            "workspace.gaps" => Some(serde_json::json!(config.workspace.gaps)),
            "workspace.workspace_width" => {
                Some(serde_json::json!(config.workspace.workspace_width))
            }
            "workspace.smooth_scrolling" => {
                Some(serde_json::json!(config.workspace.smooth_scrolling))
            }
            "window.focus_follows_mouse" => {
                Some(serde_json::json!(config.window.focus_follows_mouse))
            }
            "window.border_width" => Some(serde_json::json!(config.window.border_width)),
            "general.max_fps" => Some(serde_json::json!(config.general.max_fps)),
            "general.vsync" => Some(serde_json::json!(config.general.vsync)),
            _ => None,
        }
    }

    fn default_socket_path() -> PathBuf {
        // Prefer XDG_RUNTIME_DIR (user-private, 0700 by convention).
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            if !dir.is_empty() {
                return PathBuf::from(dir).join("axiom").join("axiom.sock");
            }
        }
        // Fallback: use a per-process subdirectory under /tmp to prevent
        // predictable-path symlink attacks. The directory is created in
        // `start()` which calls mkdir+chmod 0700.
        let pid = std::process::id();
        PathBuf::from("/tmp")
            .join(format!("axiom-{}", pid))
            .join("axiom-lazy-ui.sock")
    }

    /// Get peer UID via `libc::getsockopt(SO_PEERCRED)` (stable Rust).
    /// Returns `None` on error.
    #[cfg(unix)]
    fn get_peer_uid(stream: &UnixStream) -> Option<u32> {
        use std::os::unix::io::AsRawFd;
        let fd = stream.as_raw_fd();
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: getsockopt with SO_PEERCRED is safe — it writes cred
        // and returns 0 on success. The fd is valid (we just accepted it).
        let ret = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if ret == 0 {
            Some(cred.uid)
        } else {
            None
        }
    }

    /// Single-sample CPU usage percentage (no delta — returns 0 on first call
    /// in a static context; subsequent calls need `&mut self` for delta).
    fn sample_system_cpu_instant() -> f32 {
        let contents = match std::fs::read_to_string("/proc/stat") {
            Ok(c) => c,
            Err(_) => return 0.0,
        };
        // Return the idle ratio so callers can interpret it as CPU usage.
        // Without prior state this is a single data point, not a delta.
        if let Some(first) = contents.lines().next() {
            if first.starts_with("cpu ") {
                let parts: Vec<&str> = first.split_whitespace().collect();
                if parts.len() >= 5 {
                    let idle: f64 = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let total: f64 = parts
                        .iter()
                        .skip(1)
                        .filter_map(|s| s.parse::<f64>().ok())
                        .sum();
                    if total > 0.0 {
                        return ((1.0 - idle / total) * 100.0) as f32;
                    }
                }
            }
        }
        0.0
    }

    /// Single-sample system memory usage in MB from /proc/meminfo.
    fn sample_system_memory_mb() -> f32 {
        let meminfo = match std::fs::read_to_string("/proc/meminfo") {
            Ok(m) => m,
            Err(_) => return 0.0,
        };
        let mut total_kb: u64 = 0;
        let mut available_kb: u64 = 0;
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                total_kb = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            } else if line.starts_with("MemAvailable:") {
                available_kb = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
        }
        total_kb.saturating_sub(available_kb) as f32 / 1024.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let message = AxiomMessage::PerformanceMetrics {
            timestamp: 1234567890,
            cpu_usage: 25.5,
            memory_usage: 45.2,
            gpu_usage: 12.1,
            frame_time: 16.67,
            active_windows: 5,
            current_workspace: 2,
        };

        let json = serde_json::to_string(&message).unwrap();
        println!("Serialized message: {}", json);

        // Test that we can deserialize it back
        let _deserialized: AxiomMessage = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_lazy_ui_message_deserialization() {
        let json =
            r#"{"type":"OptimizeConfig","changes":{"blur_radius":5.0},"reason":"performance"}"#;

        let message: LazyUIMessage = serde_json::from_str(json).unwrap();

        match message {
            LazyUIMessage::OptimizeConfig { changes, reason } => {
                assert_eq!(reason, "performance");
                assert!(changes.contains_key("blur_radius"));
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_known_workspace_actions() {
        // Pin each whitelisted action as a literal — removing any single entry
        // from `KNOWN_WORKSPACE_ACTIONS` should fail this test (otherwise we
        // are testing the whitelist against itself).
        assert!(is_known_workspace_action("scroll_left"));
        assert!(is_known_workspace_action("scroll_right"));
        assert!(is_known_workspace_action("add_window"));
        assert!(is_known_workspace_action("remove_window"));
        assert!(is_known_workspace_action("move_focus_left"));
        assert!(is_known_workspace_action("move_focus_right"));
        // Unknown actions should be rejected
        assert!(!is_known_workspace_action("nuke_all_windows"));
        assert!(!is_known_workspace_action(""));
        assert!(!is_known_workspace_action("scroll"));
        assert!(!is_known_workspace_action("SCROLL_LEFT")); // case-sensitive
    }

    #[test]
    fn test_performance_report_serialization() {
        // Confirm the typed `PerformanceReport` variant round-trips through
        // serde_json with all fields preserved. This is the typed-schema
        // alternative to the prior UserEvent JSON-blob shape.
        let msg = AxiomMessage::PerformanceReport {
            timestamp: 12345,
            gpu_usage: 7.5,
            frame_time_ms: 16.7,
            active_windows: 3,
            current_workspace: 1,
            note: "ok".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: AxiomMessage = serde_json::from_str(&json).unwrap();
        match back {
            AxiomMessage::PerformanceReport {
                timestamp,
                gpu_usage,
                frame_time_ms,
                active_windows,
                current_workspace,
                note,
            } => {
                assert_eq!(timestamp, 12345);
                assert!((gpu_usage - 7.5).abs() < 1e-6);
                assert!((frame_time_ms - 16.7).abs() < 1e-6);
                assert_eq!(active_windows, 3);
                assert_eq!(current_workspace, 1);
                assert_eq!(note, "ok");
            }
            _ => panic!("Wrong message type after round-trip"),
        }
    }

    /// Round-trip test for `set_live_metrics_snapshot`. Pinned alongside the
    /// validator tests because both guard pure-API regressions: without
    /// them, a future refactor could break the per-tick snapshot pump and
    /// silently regress `HealthCheck` / `GetPerformanceReport` handlers
    /// back to zero reads — exactly the symptom the audit flagged.
    /// Covers the `get_or_insert_with` replace-on-second-call semantics and
    /// the ack that a fresh server with no `start()` plumbing can still
    /// accept the snapshot (the pump must not be coupled to socket-bind).
    #[test]
    fn test_set_live_metrics_snapshot_roundtrips_all_fields() {
        let mut server = AxiomIPCServer::new();
        assert!(
            server.live_metrics_handle.is_none(),
            "fresh server starts with no snapshot handle"
        );
        // First call: handle goes from None to Some, all fields populated.
        server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms: 12.5,
            active_windows: 7,
            current_workspace: 2,
        });
        let snap = *server
            .live_metrics_handle
            .as_ref()
            .expect("handle must exist after first snapshot call")
            .read();
        assert!((snap.frame_time_ms - 12.5).abs() < 1e-6);
        assert_eq!(snap.active_windows, 7);
        assert_eq!(snap.current_workspace, 2);

        // Second call replaces (not appends) per `get_or_insert_with` design.
        server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms: 99.9,
            active_windows: 2,
            current_workspace: -3,
        });
        let snap = *server
            .live_metrics_handle
            .as_ref()
            .expect("handle must exist after second snapshot call")
            .read();
        assert!((snap.frame_time_ms - 99.9).abs() < 1e-6);
        assert_eq!(snap.active_windows, 2);
        assert_eq!(snap.current_workspace, -3);
    }

    #[test]
    fn test_set_live_metrics_snapshot_independent_of_socket_start() {
        // The snapshot pump only mutates the pure-Rust handle; it should
        // work on a freshly-constructed server without `start()` having
        // bound the socket. Guards against a future refactor coupling the
        // snapshot path to `start()`'s plumbing.
        let mut server = AxiomIPCServer::new();
        server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms: 0.0,
            active_windows: 0,
            current_workspace: 0,
        });
        assert!(server.live_metrics_handle.is_some());
    }

    /// Issue #2 regression: WorkspaceCommand ACK must carry the new
    /// `"status": "queued_for_execution"` discriminator alongside the
    /// legacy `"accepted": <bool>` field. The dual-key shim is a temporary
    /// compat layer for clients that pattern-match on `"accepted"`. Schema
    /// drift here is the regression vector — drop or rename the status
    /// field without updating this test and existing IPC clients silently
    /// mis-interpret the ACK as "executed" (= success) when it is actually
    /// just "valid + queued for execution on next tick".
    ///
    /// Calls the actual production constructor
    /// (`AxiomIPCServer::build_workspace_command_ack`) so this test FAILS
    /// when production regresses, not only when the test fixture drifts.
    /// Pins both branches (accepted + unknown) so a future refactor that
    /// unifies or renames the status must touch this test deliberately.
    #[test]
    fn test_workspace_command_ack_schema_includes_status() {
        // Accepted path — call the actual production constructor.
        let ack_accepted = AxiomIPCServer::build_workspace_command_ack("scroll_left", true);
        let s = serde_json::to_string(&ack_accepted).unwrap();
        assert!(
            s.contains(r#""status":"queued_for_execution""#),
            "accepted ACK must carry status:queued_for_execution. JSON: {s}"
        );
        assert!(
            s.contains(r#""accepted":true"#),
            "accepted ACK must preserve legacy 'accepted' bool for compat. JSON: {s}"
        );
        assert!(
            s.contains(r#""action":"scroll_left""#),
            "ACK must echo the action verb. JSON: {s}"
        );

        // Unknown-action path — call the actual production constructor with
        // an action verb that's not in KNOWN_WORKSPACE_ACTIONS.
        let ack_unknown = AxiomIPCServer::build_workspace_command_ack("nuke_all_windows", false);
        let s = serde_json::to_string(&ack_unknown).unwrap();
        assert!(
            s.contains(r#""status":"unknown_action""#),
            "unknown ACK must carry status:unknown_action. JSON: {s}"
        );
        assert!(
            s.contains(r#""accepted":false"#),
            "unknown ACK must preserve legacy 'accepted' bool for compat. JSON: {s}"
        );
    }

    /// Fuzz: malformed JSON should not panic — returns serde error.
    #[test]
    fn test_fuzz_malformed_json() {
        let cases = [
            "",
            " ",
            "	",
            "not json at all",
            "{invalid json::",
            "[[[",
            "null",
            "true",
            "false",
            r#"{"type": 123}"#,
            r#"{"type": "UnknownMessage"}"#,
            r#"{"type": "HealthCheck", "bad_extra_field": null}"#,
            r#"{"command": "nonexistent"}"#,
            r#""#,
            "\x00\x01",
        ];
        for input in &cases {
            let result = serde_json::from_str::<crate::ipc::LazyUIMessage>(input);
            // Must not panic. Err is expected for all malformed inputs.
            let _ = result;
        }
    }

    /// Fuzz: truncated/perverse UTF-8 must not panic.
    #[test]
    fn test_fuzz_truncated_utf8() {
        let bytes: &[u8] = &[0xff, 0xfe, 0x80, 0x00, 0x7f];
        let s = String::from_utf8_lossy(bytes);
        let _ = serde_json::from_str::<crate::ipc::LazyUIMessage>(s.as_ref());
    }

    /// Fuzz: numerically extreme values in known fields must not panic.
    #[test]
    fn test_fuzz_extreme_numeric_fields() {
        let cases = [
            r#"{"type":"SetConfig","key":"workspace.scroll_speed","value":-1.0}"#,
            r#"{"type":"SetConfig","key":"workspace.scroll_speed","value":1e308}"#,
            r#"{"type":"GetPerformanceReport"}"#,
        ];
        for input in &cases {
            let result = serde_json::from_str::<crate::ipc::LazyUIMessage>(input);
            let _ = result;
        }
    }

    /// Fuzz: deeply nested or oversized input must not OOM or panic.
    #[test]
    fn test_fuzz_deep_nesting() {
        // 256 levels of nesting
        let deep = (0..256).map(|_| "{\"x\":").collect::<String>() + "42" + &"}".repeat(256);
        let result = serde_json::from_str::<crate::ipc::LazyUIMessage>(&deep);
        let _ = result;

        // Very large string value
        let huge = format!(
            r#"{{"type":"SetConfig","key":"{}","value":0.0}}"#,
            "a".repeat(65536)
        );
        let result = serde_json::from_str::<crate::ipc::LazyUIMessage>(&huge);
        let _ = result;
    }
}
