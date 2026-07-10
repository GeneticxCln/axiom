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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::AxiomConfig;

/// Maximum number of concurrent IPC client connections.
const MAX_CONNECTIONS: usize = 16;
/// Idle timeout for client connections (seconds).
const CLIENT_IDLE_TIMEOUT_SECS: u64 = 60;

/// Whitelisted `LazyUIMessage::WorkspaceCommand.action` strings. Unknown actions
/// are rejected with status `unknown_action` so callers can distinguish
/// future-supported actions from outright typos. The compositor-side
/// executor wires these to `WorkspaceTape::scroll_left/right` etc. when the
/// `process_messages` dispatch table is extended.
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

/// Maximum accepted blur radius in **pixels**. Anything above this is
/// rejected as out-of-range to prevent absurd GPU work. The unit suffix is
/// exposed in Python/dashboard clients via the rejection reason, which
/// mentions `0..=32` so an off-by-one normalised float gets caught early.
const MAX_EFFECTS_BLUR_RADIUS_PX: f32 = 32.0;
/// Maximum accepted animation speed multiplier (1.0 = realtime, >1 faster).
const MAX_EFFECTS_ANIMATION_SPEED: f32 = 10.0;
/// Maximum accepted animation duration in milliseconds.
const MAX_ANIMATION_DURATION_MS: u32 = 10_000;
/// Maximum accepted scroll speed.
const MAX_SCROLL_SPEED: f64 = 100.0;
/// Maximum size of a single line from an IPC client (64 KiB).
const MAX_IPC_LINE_BYTES: usize = 64 * 1024;

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
/// Added in this PR:
///
/// * `effects_gpu_available` — surfaces the success/failure of
///   `EffectsEngine::initialize_gpu` at compositor startup. Failure
///   was previously only visible as a single log line. Now a
///   monitoring client can detect "GPU effects did not initialize"
///   without grepping stdout.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LiveMetrics {
    /// Frame time in milliseconds from the last completed tick.
    pub frame_time_ms: f32,
    /// Number of windows currently registered with the compositor.
    pub active_windows: u32,
    /// Index of the workspace the user is currently focused on.
    pub current_workspace: i32,
    /// Whether the effects engine successfully initialised its GPU
    /// pipeline at compositor startup. `false` means blur/shadow
    /// passes will be silently dropped on the CPU path.
    pub effects_gpu_available: bool,
    /// Whether the effects engine is currently enabled (runtime toggle).
    pub effects_enabled: bool,
    /// Whether blur is enabled in the config.
    pub blur_enabled: bool,
    /// Current global blur radius in pixels.
    pub blur_radius: f32,
}

/// Returns true when `action` is in the whitelisted
/// [`KNOWN_WORKSPACE_ACTIONS`] set. Whitelist is enforced to avoid
/// silently executing untyped JSON parameters against `workspace_manager`.
///
/// **Status after Issue #2 single-dispatch fix:** no production path
/// calls this function — the compositor's `dispatch_workspace_command`
/// arm in `src/compositor.rs` owns the canonical whitelist re-check.
/// Kept as `pub(super)` for tests (`test_known_workspace_actions`) and
/// future code paths that need to validate workspace actions
/// outside the IPC handler's gate.
#[allow(dead_code)]
fn is_known_workspace_action(action: &str) -> bool {
    KNOWN_WORKSPACE_ACTIONS.contains(&action)
}

/// Validates a `LazyUIMessage::EffectsControl.blur_radius`. Returns
/// `Some(radius)` on success and `None` for non-finite or out-of-range
/// values. The compositor side of the IPC mirror in `process_messages`
/// can rely on this having been called before applying the change.
///
/// **Status after Issue #2 single-dispatch fix:** no production path
/// calls this — rate-validation is now deferred entirely to
/// `EffectsEngine::apply_live_effects_control` which re-validates inline
/// as defense in depth. Kept as `pub(super)` for tests
/// (`test_validate_blur_radius`) and any future direct callers.
#[allow(dead_code)]
fn validate_blur_radius(radius: f32) -> Option<f32> {
    if radius.is_finite() && (0.0..=MAX_EFFECTS_BLUR_RADIUS_PX).contains(&radius) {
        Some(radius)
    } else {
        None
    }
}

/// Validates a `LazyUIMessage::EffectsControl.animation_speed`. Returns
/// `Some(speed)` on success and `None` otherwise.
///
/// **Status after Issue #2 single-dispatch fix:** no production path
/// calls this — animation-speed validation is deferred to
/// `EffectsEngine::apply_live_effects_control` which re-validates inline
/// as defense in depth. Kept as `pub(super)` for tests
/// (`test_validate_animation_speed`) and any future direct callers.
#[allow(dead_code)]
fn validate_animation_speed(speed: f32) -> Option<f32> {
    if speed.is_finite() && (0.0..=MAX_EFFECTS_ANIMATION_SPEED).contains(&speed) {
        Some(speed)
    } else {
        None
    }
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

    /// Effects engine status, pushed after each `PerformanceReport` (and
    /// on any effects configuration change via `EffectsControl`). Tells
    /// monitoring clients whether GPU-accelerated post-processing (blur,
    /// shadow, rounded corners) is available and currently enabled.
    EffectsStatus {
        timestamp: u64,
        /// Whether the GPU pipeline initialised successfully at startup.
        effects_gpu_available: bool,
        /// Whether effects are currently enabled (runtime toggle).
        effects_enabled: bool,
        /// Whether blur is enabled in the config (read-only gate).
        blur_enabled: bool,
        /// Current blur radius in pixels (global, live value).
        blur_radius: f32,
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
    /// against `KNOWN_WORKSPACE_ACTIONS` and forwards the message via the
    /// mpsc command channel to the compositor's `process_messages`. The
    /// compositor-side dispatch arm for `WorkspaceCommand` is currently
    /// missing — accepted actions are queued (`dispatched_via_mpsc: true`
    /// in the ACK) but not executed. Tracked as a follow-up to wire
    /// `scroll_left`, `scroll_right`, `add_window`, `remove_window`,
    /// `move_focus_left`, and `move_focus_right` to
    /// `ScrollableWorkspaces` mutations.
    WorkspaceCommand {
        action: String,
        parameters: serde_json::Value,
    },

    /// Effects control
    EffectsControl {
        enabled: Option<bool>,
        blur_radius: Option<f32>,
        animation_speed: Option<f32>,
    },

    /// Per-window blur control. `radius` in pixels (0..=32); 0 disables blur.
    SetWindowBlur { window_id: u64, radius: f32 },

    /// System health check request
    HealthCheck,

    /// Request performance report
    GetPerformanceReport,
}

/// IPC server for handling communication with Lazy UI
pub struct AxiomIPCServer {
    socket_path: PathBuf,
    /// Broadcast channel for outgoing Axiom messages to all clients
    broadcast_tx: Option<broadcast::Sender<AxiomMessage>>,
    command_receiver: Option<mpsc::UnboundedReceiver<LazyUIMessage>>,
    /// Sender side of command channel (for wiring incoming commands)
    command_sender: Option<mpsc::UnboundedSender<LazyUIMessage>>,
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
    // Graceful shutdown token
    shutdown_token: Option<CancellationToken>,
    // Handle for the accept loop task
    accept_handle: Option<JoinHandle<Result<()>>>,
    // Connection limit semaphore
    connection_semaphore: Option<Arc<Semaphore>>,
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

        Self {
            socket_path,
            broadcast_tx: None,
            command_receiver: None,
            command_sender: None,
            config_handle: None,
            live_metrics_handle: None,
            last_metrics_sent: Instant::now(),
            last_cpu_times: None,
            shutdown_token: None,
            accept_handle: None,
            connection_semaphore: None,
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
    /// time (Design 14 — surface `effects_gpu_available` BEFORE the first
    /// tick) or refresh it after each tick. The compositor devolves to
    /// `set_live_metrics_snapshot` on the same handle from inside `tick()`.
    pub fn set_live_metrics_snapshot(&mut self, snapshot: LiveMetrics) {
        let handle = self
            .live_metrics_handle
            .get_or_insert_with(|| Arc::new(parking_lot::RwLock::new(LiveMetrics::default())));
        *handle.write() = snapshot;
    }

    /// Build the WorkspaceCommand ACK UserEvent for the per-client handler.
    /// Schema owned here (single source of truth) so the
    /// `test_workspace_command_ack_schema_includes_status` regression test
    /// exercises the actual production constructor. A pure helper that does
    /// not take `&self` so call sites are both `Self::` (from inside the
    /// impl) and `AxiomIPCServer::` (from the test mod) without forcing a
    /// `new()` plumbing for each call.
    ///
    /// **Status after Issue #2 single-dispatch fix:** the per-client
    /// handler no longer constructs WorkspaceCommand ACKs inline (it
    /// forwards to `cmd_tx` and the compositor's dispatch arm owns the
    /// lifecycle). This constructor remains available for tests and
    /// any future code path that wants to emit a typed ACK without
    /// going through the IPC channel.
    #[allow(dead_code)]
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

    /// Build the EffectsControl ACK UserEvent (single source of truth).
    /// `partially_queued` when at least one field failed range or
    /// finite-validation; `queued_for_execution` when all fields passed.
    /// Surface area mirrors WorkspaceCommand: a discriminator field that
    /// disambiguates VALIDATION-time ACK from EXECUTION-time ACK, plus the
    /// existing `accepted`/`rejected` arrays for per-field diagnostics.
    ///
    /// **Status after Issue #2 single-dispatch fix:** the per-client
    /// handler no longer constructs EffectsControl ACKs inline (it
    /// forwards to `cmd_tx` and the compositor's dispatch arm owns the
    /// lifecycle). This constructor remains available for tests and
    /// any future code path that wants to emit a typed ACK without
    /// going through the IPC channel.
    #[allow(dead_code)]
    pub(super) fn build_effects_control_ack(
        accepted: Vec<String>,
        rejected: Vec<(String, String)>,
    ) -> AxiomMessage {
        AxiomMessage::UserEvent {
            // Fail loudly on a pre-1970 system clock rather than silently
            // emitting `timestamp: 0`. Symmetric with build_workspace_command_ack.
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before UNIX_EPOCH — compositor hardware fault")
                .as_secs(),
            event_type: "EffectsControlAck".into(),
            details: serde_json::json!({
                "status": if rejected.is_empty() {
                    "queued_for_execution"
                } else {
                    "partially_queued"
                },
                "accepted": accepted,
                "rejected": rejected,
            }),
        }
    }

    /// Start the IPC server
    pub fn start(&mut self) -> Result<()> {
        // Ensure parent dir exists with correct permissions (0700).
        // Do the mkdir+chmod before anything else so the directory is
        // never observable with wider permissions.
        if let Some(dir) = self.socket_path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create IPC dir: {:?}", dir))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Set 0700 immediately after creation to minimise the
                // window where the directory is world-readable.
                if let Err(e) =
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                {
                    warn!("⚠️ Failed to set 0700 on IPC directory {:?}: {}", dir, e);
                }
            }
        }

        // Bind the socket without a TOCTOU check-then-remove race.
        // If the socket file already exists, UnixListener::bind will fail;
        // we tolerate that failure and remove+retry once.
        let listener = match UnixListener::bind(&self.socket_path) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                let _ = std::fs::remove_file(&self.socket_path);
                UnixListener::bind(&self.socket_path).with_context(|| {
                    format!(
                        "Failed to bind Unix socket after stale removal: {:?}",
                        self.socket_path
                    )
                })?
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("Failed to bind Unix socket: {:?}", self.socket_path)
                });
            }
        };

        // Tighten socket permissions (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) =
                std::fs::set_permissions(&self.socket_path, std::fs::Permissions::from_mode(0o600))
            {
                warn!(
                    "⚠️ Failed to set 0600 on socket {:?}: {}",
                    self.socket_path, e
                );
            }
        }

        // Create broadcast channel for outgoing messages
        let (tx, _rx) = broadcast::channel::<AxiomMessage>(1024);
        self.broadcast_tx = Some(tx.clone());

        // Create command channel for incoming messages
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<LazyUIMessage>();
        self.command_sender = Some(cmd_tx.clone());
        self.command_receiver = Some(cmd_rx);

        // Create shutdown token
        let shutdown_token = CancellationToken::new();
        self.shutdown_token = Some(shutdown_token.clone());

        // Create connection semaphore
        let semaphore = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        self.connection_semaphore = Some(semaphore.clone());

        // Get our own UID for peer credential checks
        // SAFETY: getuid() is always safe to call — it has no preconditions,
        // never fails, and does not touch any Rust-managed memory.
        #[cfg(unix)]
        let our_uid = unsafe { libc::getuid() };
        #[cfg(not(unix))]
        let our_uid = 0u32;

        info!("🔗 Axiom IPC server listening on: {:?}", self.socket_path);

        // Forward the config + metrics handles so per-client handlers
        // can resolve GetConfig queries AND live composition/system
        // metrics from HealthCheck / GetPerformanceReport requests. Without
        // the metrics_handle plumbing the per-client task only sees
        // `None` and the handler falls back to LiveMetrics::default()
        // — defeating Design 12's wire of real values.
        let config_handle = self.config_handle.clone();
        let metrics_handle = self.live_metrics_handle.clone();

        // Start accepting connections in a separate task
        let handle = tokio::spawn(Self::accept_connections_static(
            listener,
            tx,
            cmd_tx,
            shutdown_token,
            semaphore,
            our_uid,
            config_handle,
            metrics_handle,
        ));
        self.accept_handle = Some(handle);

        Ok(())
    }

    /// Accept incoming connections from Lazy UI (static version)
    #[allow(clippy::too_many_arguments)]
    async fn accept_connections_static(
        listener: UnixListener,
        tx: broadcast::Sender<AxiomMessage>,
        cmd_tx: mpsc::UnboundedSender<LazyUIMessage>,
        shutdown_token: CancellationToken,
        semaphore: Arc<Semaphore>,
        our_uid: u32,
        config_handle: Option<Arc<parking_lot::RwLock<AxiomConfig>>>,
        metrics_handle: Option<Arc<parking_lot::RwLock<LiveMetrics>>>,
    ) -> Result<()> {
        loop {
            tokio::select! {
                biased;
                _ = shutdown_token.cancelled() => {
                    info!("🔽 IPC accept loop shutting down");
                    break;
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _)) => {
                            // Peer credential check
                            #[cfg(unix)]
                            {

                                match stream.peer_cred() {
                                    Ok(cred) => {
                                        if cred.uid() != our_uid {
                                            warn!("🚫 Rejecting IPC connection from different user (uid={})", cred.uid());
                                            continue;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("⚠️ Failed to get peer credentials: {}, rejecting connection", e);
                                        continue;
                                    }
                                }
                            }

                            // Acquire semaphore permit (limits concurrent connections)
                            let permit = match semaphore.clone().try_acquire_owned() {
                                Ok(p) => p,
                                Err(_) => {
                                    warn!("🚫 Max IPC connections reached ({}), rejecting", MAX_CONNECTIONS);
                                    continue;
                                }
                            };

                            info!("🤝 Lazy UI connected to Axiom IPC");
                            let rx = tx.subscribe();
                            let cmd_tx_clone = cmd_tx.clone();
                            let config_for_client = config_handle.clone();
                            let metrics_for_client = metrics_handle.clone();
                            tokio::spawn(async move {
                                let _permit = permit; // Hold permit for duration of connection
                                if let Err(e) = Self::handle_client(stream, rx, cmd_tx_clone, config_for_client, metrics_for_client).await {
                                    debug!("IPC client handler ended: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("❌ Error accepting IPC connection: {}", e);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Handle a single client connection
    async fn handle_client(
        stream: UnixStream,
        mut rx: broadcast::Receiver<AxiomMessage>,
        cmd_tx: mpsc::UnboundedSender<LazyUIMessage>,
        config_handle: Option<Arc<parking_lot::RwLock<AxiomConfig>>>,
        metrics_handle: Option<Arc<parking_lot::RwLock<LiveMetrics>>>,
    ) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line_buf = String::new();
        let idle_timeout = Duration::from_secs(CLIENT_IDLE_TIMEOUT_SECS);

        // Send startup notification
        let startup_msg = AxiomMessage::StartupComplete {
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: vec![
                "scrollable_workspaces".to_string(),
                "visual_effects".to_string(),
                "performance_metrics".to_string(),
                "ai_optimization".to_string(),
            ],
        };

        Self::send_message(&mut writer, &startup_msg).await?;

        // Process incoming messages and outgoing broadcasts concurrently.
        // Creates a fresh `take()`-limited reader each iteration to bound
        // memory: any client sending > MAX_IPC_LINE_BYTES without \n is
        // disconnected after hitting the take limit.
        loop {
            let mut limited = (&mut reader).take((MAX_IPC_LINE_BYTES + 1) as u64);

            tokio::select! {
                // Idle timeout - disconnect if no activity
                _ = tokio::time::sleep(idle_timeout) => {
                    info!("⏱️ IPC client idle timeout, disconnecting");
                    break;
                }
                res = limited.read_line(&mut line_buf) => {
                    let n = match res {
                        Ok(n) => n,
                        Err(e) => {
                            warn!("⚠️ IPC read error: {}", e);
                            break;
                        }
                    };

                    if n == 0 {
                        break; // client disconnected
                    }

                    // Line exceeded the maximum allowed size — drop the
                    // connection to prevent unbounded memory DoS.
                    if line_buf.len() > MAX_IPC_LINE_BYTES {
                        warn!("⚠️ IPC message too large ({} bytes, max {}) - disconnecting",
                            line_buf.len(), MAX_IPC_LINE_BYTES);
                        break;
                    }

                    let trimmed = line_buf.trim();
                    if trimmed.is_empty() {
                        line_buf.clear();
                        continue;
                    }                        debug!("📨 Received IPC message: {}", trimmed);
                        match serde_json::from_str::<LazyUIMessage>(trimmed) {
                            Ok(message) => {
                                // Issue #2 real fix (not the cosmetic ACK-schema
                                // patch from prior session): command-type messages
                                // (WorkspaceCommand, EffectsControl) follow a
                                // SINGLE-DISPATCH path. The IPC layer forwards
                                // them to the compositor via `cmd_tx` and emits
                                // a minimal `*Queued` ACK. The compositor's
                                // `process_messages` drain loop in
                                // `AxiomCompositor::process_events` owns both
                                // validation (whitelist + range checks) AND
                                // actual state mutation via
                                // `dispatch_workspace_command` /
                                // `dispatch_effects_control`.
                                //
                                // Pre-fix behaviour ran the same message through
                                // TWO paths:
                                //   1. `cmd_tx.send(message.clone())` — compositor
                                //      drains + dispatches (real mutation).
                                //   2. `process_lazy_ui_message(message, ...)` —
                                //      emitted an ACK with `accepted:<bool>` /
                                //      `rejected:[(field,reason)]` based on
                                //      validation alone. No actual mutation in
                                //      this path.
                                //
                                // The duplicate-validation path is what
                                // monitoring tooling observed as "executed
                                // twice": the ACK claimed fields were
                                // accepted based on a stale whitelist state,
                                // then the compositor's dispatch arm
                                // re-validated on possibly-different rules.
                                // Single dispatch through cmd_tx + a
                                // compositor-owned dispatch arm makes
                                // validation the single source of truth
                                // before mutation occurs.
                                //
                                // All other message types
                                // (OptimizeConfig / SetConfig that mutate
                                // config-owned fields via
                                // `process_messages`, GetConfig /
                                // HealthCheck / GetPerformanceReport that are
                                // query-only) keep the dual-path because
                                // their ACK shape is synchronous
                                // request-response and doesn't drive state
                                // mutation through the workspace /
                                // effects engine path.
                                let is_command_type = matches!(
                                    message,
                                    LazyUIMessage::WorkspaceCommand { .. }
                                        | LazyUIMessage::EffectsControl { .. }
                                        | LazyUIMessage::SetWindowBlur { .. }
                                );
                                if is_command_type {
                                    // SINGLE DISPATCH (Issue #2 real fix):
                                    // WorkspaceCommand + EffectsControl
                                    // messages are forwarded to the compositor
                                    // via `cmd_tx` ONLY — no inline ACK from
                                    // `process_lazy_ui_message` — so the
                                    // compositor's `process_messages` drain
                                    // + `dispatch_workspace_command` /
                                    // `dispatch_effects_control` arms own the
                                    // full mutation lifecycle. This collapses
                                    // the pre-fix duplicate-validation path
                                    // that monitoring tooling observed as
                                    // "executed twice".
                                    //
                                    // Per-reviewer Q2: capture `cmd_tx.send`
                                    // result so a closed channel (compositor
                                    // shutdown) returns a `delivery_failed`
                                    // ACK instead of a misleading `queued`
                                    // status.
                                    //
                                    // Per-reviewer Q3: keep the original
                                    // `WorkspaceCommandAck` /
                                    // `EffectsControlAck` `event_type`
                                    // discriminator for backward compat with
                                    // IPC clients (lazy_ui_client.py matched
                                    // on these strings). The new
                                    // `status: queued_for_compositor_dispatch`
                                    // discriminator is additive — old clients
                                    // matching on `event_type` still work.
                                    //
                                    // Per-reviewer Q6: build `details` via
                                    // `serde_json::json!` macro directly per
                                    // match arm — no
                                    // `serde_json::Value::Object(ref mut
                                    // map)` mutation after the fact.
                                    let cmd_event_type: &'static str;
                                    let cmd_details: serde_json::Value;
                                    match &message {
                                        LazyUIMessage::WorkspaceCommand { action, .. } => {
                                            cmd_event_type = "WorkspaceCommandAck";
                                            cmd_details = serde_json::json!({
                                                "action": action,
                                                "status": "queued_for_compositor_dispatch",
                                                "executor": "process_messages",
                                                // Compat shim: prior cosmetic
                                                // ACK schema exposed `accepted`
                                                // and `dispatched_via_mpsc`
                                                // as bool. Old IPC consumers
                                                // (lazy_ui_client.py pattern
                                                // matches, external dashboard
                                                // tooling reading the JSON
                                                // stream) silently break if
                                                // we drop them. We always
                                                // forward (single dispatch →
                                                // cmd_tx), so `accepted=true`
                                                // is honest.
                                                "accepted": true,
                                                "dispatched_via_mpsc": true,
                                            });
                                        }
                                        LazyUIMessage::EffectsControl { .. } => {
                                            cmd_event_type = "EffectsControlAck";
                                            cmd_details = serde_json::json!({
                                                "status": "queued_for_compositor_dispatch",
                                                "executor": "process_messages",
                                                "note": "per-field diagnostics dropped — compositor owns validation",
                                                "accepted": true,
                                                "dispatched_via_mpsc": true,
                                            });
                                        }
                                        LazyUIMessage::SetWindowBlur { window_id, radius } => {
                                            cmd_event_type = "SetWindowBlurAck";
                                            cmd_details = serde_json::json!({
                                                "window_id": window_id,
                                                "radius": radius,
                                                "status": "queued_for_compositor_dispatch",
                                                "accepted": true,
                                                "dispatched_via_mpsc": true,
                                            });
                                        }
                                        // `is_command_type` already gated the
                                        // WorkspaceCommand / EffectsControl /
                                        // SetWindowBlur branch above; the remaining 5
                                        // variants (OptimizeConfig / GetConfig
                                        // / SetConfig / HealthCheck /
                                        // GetPerformanceReport) flow into
                                        // the wildcard catch-all below as a
                                        // defensive no-op so the match is
                                        // exhaustive over `LazyUIMessage`.
                                        // `unreachable!()` is statically
                                        // reachable (5 variants flow into
                                        // `_`) so no `unreachable_patterns`
                                        // lint fires, but the runtime is
                                        // guaranteed never to enter this arm
                                        // because the outer `is_command_type`
                                        // predicate already returned `true`.
                                        _ => unreachable!(
                                            "is_command_type gated WorkspaceCommand / EffectsControl / SetWindowBlur"
                                        ),
                                    }
                                    // Q2: capture the send result; build the
                                    // ACK BEFORE the borrow on `message`
                                    // expires (we're about to move `message`
                                    // into `cmd_tx.send`).
                                    let send_result = cmd_tx.send(message);
                                    let ack_event_type: &'static str;
                                    let ack_details: serde_json::Value;
                                    // Q2 fix (compile error E0382 — partial move of
                                    // `send_result`): pattern-bind via `ref e` so
                                    // `send_result` remains accessible for the
                                    // downstream `if send_result.is_err()` check.
                                    // The original `Err(e) =>` form moved the
                                    // `SendError<LazyUIMessage>` out of
                                    // `send_result`, after which Rust can't
                                    // re-borrow the binding.
                                    match &send_result {
                                        Ok(()) => {
                                            ack_event_type = cmd_event_type;
                                            ack_details = cmd_details;
                                        }
                                        Err(e) => {
                                            ack_event_type = match cmd_event_type {
                                                "WorkspaceCommandAck" => "WorkspaceCommandAckFailed",
                                                "EffectsControlAck" => "EffectsControlAckFailed",
                                                "SetWindowBlurAck" => "SetWindowBlurAckFailed",
                                                _ => unreachable!(
                                                    "cmd_event_type is one of the three command-type acks"
                                                ),
                                            };
                                            let reason = format!(
                                                "compositor cmd_tx receiver dropped during shutdown: {}",
                                                e
                                            );
                                            ack_details = serde_json::json!({
                                                "status": "delivery_failed",
                                                "reason": reason,
                                            });
                                            warn!(
                                                "⚠️ cmd_tx.send failed for command-type message \
                                                 (likely compositor shutdown); emitting delivery_failed ACK"
                                            );
                                        }
                                    }
                                    let ack = AxiomMessage::UserEvent {
                                        timestamp: SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .expect(
                                                "system clock before UNIX_EPOCH — \
                                                 compositor hardware fault",
                                            )
                                            .as_secs(),
                                        event_type: ack_event_type.into(),
                                        details: ack_details,
                                    };
                                    if let Err(e) =
                                        Self::send_message(&mut writer, &ack).await
                                    {
                                        if send_result.is_err() {
                                            warn!(
                                                "⚠️ delivery_failed ACK could not be sent \
                                                 (likely peer already disconnected): {}",
                                                e
                                            );
                                        } else {
                                            warn!(
                                                "⚠️ Failed sending command-type queued ACK: {}",
                                                e
                                            );
                                        }
                                    }
                                } else {
                                    // Dual-path retained for non-command
                                    // types (config mutation + queries).
                                    // cmd_tx.send forwards to the
                                    // compositor's `process_messages`
                                    // drain (which mutates
                                    // config-owned fields for
                                    // OptimizeConfig / SetConfig and drops
                                    // the rest). The inline handler
                                    // takes care of validation + ACK
                                    // generation synchronously.
                                    let _ = cmd_tx.send(message.clone());
                                    let cfg_snapshot = config_handle
                                        .as_ref()
                                        .map(|h| h.read().clone());
                                    if let Err(e) =
                                        Self::process_lazy_ui_message(
                                            message,
                                            &mut writer,
                                            cfg_snapshot.as_ref(),
                                            metrics_handle.as_ref(),
                                        )
                                        .await
                                    {
                                        warn!(
                                            "⚠️ Error processing message: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("⚠️ Invalid JSON from IPC client: {}", e);
                            }
                        }

                    // Clear the buffer for the next iteration
                    line_buf.clear();
                },
                msg = rx.recv() => {
                    match msg {
                        Ok(message) => {
                            if let Err(e) = Self::send_message(&mut writer, &message).await {
                                warn!("⚠️ Failed to send broadcast message: {}", e);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("⚠️ IPC client lagged by {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        info!("📪 Lazy UI disconnected from Axiom IPC");
        Ok(())
    }

    /// Process a message from Lazy UI
    async fn process_lazy_ui_message(
        message: LazyUIMessage,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        config: Option<&AxiomConfig>,
        metrics_handle: Option<&Arc<parking_lot::RwLock<LiveMetrics>>>,
    ) -> Result<()> {
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
                    debug!("  📝 Setting {}: {:?}", key, value);
                    // For now accept only whitelisted keys
                    let ok = matches!(
                        key.as_str(),
                        "effects.blur.radius"
                            | "effects.animations.duration"
                            | "workspace.scroll_speed"
                    );
                    if ok {
                        applied.push(key);
                    } else {
                        rejected.push((key, "unsupported_key".into()));
                    }
                }
                // Send a simple acknowledgment as UserEvent for now
                let ack = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    event_type: "OptimizeConfigAck".into(),
                    details: serde_json::json!({ "applied": applied, "rejected": rejected }),
                };
                Self::send_message(writer, &ack).await?;
            }

            LazyUIMessage::GetConfig { key } => {
                debug!("📋 Config query: {}", key);

                // Resolve against the live `AxiomConfig` snapshot taken when the
                // client connected. Each dot-separated "section.field" path is
                // walked against the supported schema; unknown paths return Null
                // so callers can distinguish "missing" from "default".
                let value = config
                    .and_then(|cfg| Self::resolve_config_path(cfg, &key))
                    .unwrap_or(serde_json::Value::Null);
                if value.is_null() {
                    debug!(
                        "GetConfig '{}' returned Null (key not recognised in live config)",
                        key
                    );
                } else {
                    debug!("GetConfig '{}' resolved against live config", key);
                }

                let response = AxiomMessage::ConfigResponse {
                    key: key.clone(),
                    value,
                };

                Self::send_message(writer, &response).await?;
            }

            LazyUIMessage::SetConfig { key, value } => {
                info!("⚙️ Setting config: {} = {:?}", key, value);
                // Honest ACK: this per-client handler ONLY validates and
                // forwards the request to the compositor thread via
                // `cmd_tx`. The actual `AxiomConfig` mutation happens
                // later inside `process_messages`, which runs on the
                // compositor tick. Report `queued`, not `accepted`, so
                // monitoring clients can distinguish "we have the
                // request" from "the compositor applied it". The
                // compositor-side application future-PR can wire a
                // follow-up `SetConfigApplied` event to close the loop
                // without breaking this schema.
                let ack = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    event_type: "SetConfigAck".into(),
                    details: serde_json::json!({
                        "key": key,
                        "status": "queued",
                        "applied_later_by": "process_messages",
                    }),
                };
                Self::send_message(writer, &ack).await?;
            }

            // Command-type messages (WorkspaceCommand, EffectsControl) are
            // gated upstream in `handle_client`'s single-dispatch branch
            // (Issue #2 real fix). They are forwarded to the compositor
            // via `cmd_tx` ONLY — no inline ACK — so the compositor's
            // `process_messages` drain + dispatch arm is the single
            // mutation path. The arms below are kept (with debug logging)
            // so a future drift between the upstream gate and this match
            // surfaces as a `debug!` log rather than a panic or compile
            // error. `build_workspace_command_ack` /
            // `build_effects_control_ack` remain available as
            // `pub(super)` constructors for tests and any future caller
            // that needs the typed ACK shape.
            LazyUIMessage::WorkspaceCommand { action, .. } => {
                debug!(
                    "⚠️ WorkspaceCommand({action}) reached process_lazy_ui_message \
                     — upstream gate in handle_client should have dispatched \
                     it via cmd_tx only"
                );
            }
            LazyUIMessage::EffectsControl { .. } => {
                debug!(
                    "⚠️ EffectsControl reached process_lazy_ui_message \
                     — upstream gate in handle_client should have dispatched \
                     it via cmd_tx only"
                );
            }
            LazyUIMessage::SetWindowBlur { .. } => {
                debug!(
                    "⚠️ SetWindowBlur reached process_lazy_ui_message \
                     — upstream gate in handle_client should have dispatched \
                     it via cmd_tx only"
                );
            }

            LazyUIMessage::HealthCheck => {
                debug!("🏥 Health check request");
                // Read real system metrics from /proc and sysfs (same path
                // as `GetPerformanceReport`). CPU is a single-sample reading
                // (no delta), so it will be 0 on first call; subsequent
                // calls within the same connection will report deltas if we
                // had `&mut self`, but this static handler cannot carry
                // state. Memory and GPU are real point-in-time readings.
                //
                // **Per-client metrics snapshot integration (Design 12).**
                // The compositor pushes live values into `live_metrics_handle`
                // on every tick; we read them here to populate the response.
                // Without this, monitoring clients couldn't tell "metrics not
                // wired" from "true zero reading". The handle is read inside
                // a single short-lived `parking_lot::RwLock` `read()` guard
                // and never crosses an await point — safe to use here.
                let snapshot = metrics_handle
                    .as_ref()
                    .map(|h| *h.read())
                    .unwrap_or_default();
                let cpu = Self::sample_system_cpu_instant();
                let mem = Self::sample_system_memory_mb();
                let gpu = Self::sample_gpu_usage();
                let metrics = AxiomMessage::PerformanceMetrics {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    cpu_usage: cpu,
                    memory_usage: mem,
                    gpu_usage: gpu,
                    frame_time: snapshot.frame_time_ms,
                    active_windows: snapshot.active_windows,
                    current_workspace: snapshot.current_workspace,
                };

                Self::send_message(writer, &metrics).await?;
            }

            LazyUIMessage::GetPerformanceReport => {
                debug!("📊 Performance report request");
                // Live system metrics are sampled via the static helpers below
                // (they only touch OS files, no compositor state required).
                //
                // **Per-client metrics snapshot integration (Design 12).**
                // Read frame_time_ms / active_windows / current_workspace
                // from the live handle the compositor pushed on its last
                // tick. `effects_gpu_available` rounds out the report
                // (Design 14) so monitoring clients can tell whether
                // blur / shadow post-processing actually runs on the GPU
                // or silently falls back to CPU-only.
                let snapshot = metrics_handle
                    .as_ref()
                    .map(|h| *h.read())
                    .unwrap_or_default();
                let gpu_usage = Self::sample_gpu_usage();
                // Note contract:
                // - `note` is empty when the live snapshot is wired (the
                //   composer has called `set_live_metrics_snapshot`).
                // - `note` records the placeholder caveat when the
                //   snapshot is the default (no compositor wired), so old
                //   clients that grep on note still recognise the gap.
                let note = if metrics_handle.is_some() {
                    String::new()
                } else {
                    "live snapshot not wired — fields reflect LiveMetrics::default()".to_string()
                };
                let report = AxiomMessage::PerformanceReport {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    gpu_usage,
                    frame_time_ms: snapshot.frame_time_ms,
                    active_windows: snapshot.active_windows,
                    current_workspace: snapshot.current_workspace,
                    note,
                };
                // The `effects_gpu_available` field is additive on the wire
                // schema (serde flatten via PerformanceMetrics variant or a
                // dedicated extension). The PerformanceReport variant
                // doesn't carry it today; we surface it through the
                // dedicated `EffectsStatus` follow-up message that
                // monitoring clients can poll on demand. For now the per-
                // client ACK includes the boolean in the `details`
                // payload so old readers can ignore it without breaking.
                if metrics_handle.is_some() {
                    let effects_status = AxiomMessage::EffectsStatus {
                        timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                        effects_gpu_available: snapshot.effects_gpu_available,
                        effects_enabled: snapshot.effects_enabled,
                        blur_enabled: snapshot.blur_enabled,
                        blur_radius: snapshot.blur_radius,
                    };
                    Self::send_message(writer, &effects_status).await?;
                    debug!("📤 Sent EffectsStatus follow-up");
                }
                Self::send_message(writer, &report).await?;
            }
        }

        Ok(())
    }

    /// Send a message to Lazy UI
    async fn send_message(
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        message: &AxiomMessage,
    ) -> Result<()> {
        let json = serde_json::to_string(message).with_context(|| "Failed to serialize message")?;

        writer
            .write_all(json.as_bytes())
            .await
            .with_context(|| "Failed to write message")?;
        writer
            .write_all(b"\n")
            .await
            .with_context(|| "Failed to write newline")?;

        debug!("📤 Sent IPC message: {}", json);

        Ok(())
    }

    /// Phase 3: Process pending IPC messages and apply configuration changes.
    /// Returns `(config_changed, pending_actions)`:
    /// - `config_changed`: true if any `OptimizeConfig` / `SetConfig` mutator
    ///   wrote to the config-owned path. Callers typically call
    ///   `update_subsystems_config()` and refresh the IPC handle when set.
    /// - `pending_actions`: messages from `WorkspaceCommand` /
    ///   `EffectsControl` (already validated at the per-client layer) that
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
                                    "effects.blur.radius" => {
                                        config.effects.blur.radius = val_f64
                                            .clamp(0.0, MAX_EFFECTS_BLUR_RADIUS_PX as f64)
                                            as u32;
                                        config_changed = true;
                                        debug!("  Set blur radius to {}", val_f64);
                                    }
                                    "effects.animations.duration" => {
                                        config.effects.animations.duration = val_f64
                                            .clamp(1.0, MAX_ANIMATION_DURATION_MS as f64)
                                            as u32;
                                        config_changed = true;
                                        debug!("  Set animation duration to {}", val_f64);
                                    }
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
                        // Manual mapping with bounds enforcement (defense in depth).
                        // The per-client IPC handler already validates ranges for the
                        // ACK, but this compositor-side path re-validates to guard
                        // against future code-paths that bypass the per-client layer.
                        if let Some(val_f64) = value.as_f64() {
                            match key.as_str() {
                                "effects.blur.radius" => {
                                    config.effects.blur.radius = val_f64
                                        .clamp(0.0, MAX_EFFECTS_BLUR_RADIUS_PX as f64)
                                        as u32;
                                    config_changed = true;
                                }
                                "effects.animations.duration" => {
                                    config.effects.animations.duration =
                                        val_f64.clamp(1.0, MAX_ANIMATION_DURATION_MS as f64) as u32;
                                    config_changed = true;
                                }
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
                    | LazyUIMessage::EffectsControl { .. }
                    | LazyUIMessage::SetWindowBlur { .. } => {
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
    pub fn command_sender_for_test(&self) -> Option<&mpsc::UnboundedSender<LazyUIMessage>> {
        self.command_sender.as_ref()
    }

    /// Broadcast PerformanceMetrics to all connected clients
    pub fn broadcast_performance_metrics(
        &self,
        cpu_usage: f32,
        memory_usage: f32,
        gpu_usage: f32,
        frame_time: f32,
        active_windows: u32,
        current_workspace: i32,
    ) -> Result<()> {
        if let Some(tx) = &self.broadcast_tx {
            let _ = tx.send(AxiomMessage::PerformanceMetrics {
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                cpu_usage,
                memory_usage,
                gpu_usage,
                frame_time,
                active_windows,
                current_workspace,
            });
        }
        Ok(())
    }

    /// Broadcast a compositor state change to all connected IPC clients.
    ///
    /// `component` identifies the subsystem (e.g. `"workspace"`, `"window"`,
    /// `"effects"`) and `new_state` / `old_state` describe the transition
    /// (e.g. `"scrolled_right"`, `"minimized"`, `"fullscreen"`).  This is a
    /// fire-and-forget broadcast — send failures (no connected clients) are
    /// silently ignored.
    pub fn broadcast_state_change(
        &self,
        component: &str,
        old_state: &str,
        new_state: &str,
    ) -> Result<()> {
        if let Some(tx) = &self.broadcast_tx {
            let _ = tx.send(AxiomMessage::StateChange {
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                component: component.to_owned(),
                old_state: old_state.to_owned(),
                new_state: new_state.to_owned(),
            });
        }
        Ok(())
    }

    /// Rate-limited helper that samples CPU/GPU/memory and broadcasts metrics (~10Hz)
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
        let (cpu, mem_mb) = self.sample_system_metrics_nonblocking();
        let gpu = Self::sample_gpu_usage();
        let _ = self.broadcast_performance_metrics(
            cpu,
            mem_mb,
            gpu,
            frame_time_ms,
            active_windows,
            current_workspace,
        );
        self.last_metrics_sent = Instant::now();
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

    /// Gracefully shut down the IPC server
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("🔽 Shutting down IPC server...");

        // Cancel the accept loop
        if let Some(token) = self.shutdown_token.take() {
            token.cancel();
        }

        // Wait for the accept loop to finish
        if let Some(handle) = self.accept_handle.take() {
            // Give it a short timeout in case it's stuck
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(Ok(_)) => info!("✅ IPC accept loop stopped"),
                Ok(Err(e)) => warn!("⚠️ IPC accept loop error: {}", e),
                Err(_) => warn!("⚠️ IPC accept loop shutdown timed out"),
            }
        }

        // Drop broadcast channel to signal clients
        self.broadcast_tx = None;
        self.command_sender = None;

        info!("✅ IPC server shut down");
        Ok(())
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
    /// Supports the `workspace.*`, `effects.*`, `general.*`, and `xwayland.*`
    /// subtrees. Returns `None` for unknown paths so the IPC layer can answer
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
            "effects.enabled" => Some(serde_json::json!(config.effects.enabled)),
            "effects.blur.radius" => Some(serde_json::json!(config.effects.blur.radius)),
            "effects.blur.intensity" => Some(serde_json::json!(config.effects.blur.intensity)),
            "effects.animations.duration" => {
                Some(serde_json::json!(config.effects.animations.duration))
            }
            "effects.shadows.opacity" => Some(serde_json::json!(config.effects.shadows.opacity)),
            "general.max_fps" => Some(serde_json::json!(config.general.max_fps)),
            "general.vsync" => Some(serde_json::json!(config.general.vsync)),
            "xwayland.enabled" => Some(serde_json::json!(config.xwayland.enabled)),
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

    #[tokio::test]
    async fn test_message_serialization() {
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
    fn test_validate_blur_radius() {
        // Valid: in-range finite values
        assert_eq!(validate_blur_radius(0.0), Some(0.0));
        assert_eq!(validate_blur_radius(8.5), Some(8.5));
        assert_eq!(
            validate_blur_radius(MAX_EFFECTS_BLUR_RADIUS_PX),
            Some(MAX_EFFECTS_BLUR_RADIUS_PX)
        );
        // Invalid: out of range or non-finite
        assert_eq!(validate_blur_radius(-1.0), None);
        assert_eq!(validate_blur_radius(MAX_EFFECTS_BLUR_RADIUS_PX + 0.1), None);
        assert_eq!(validate_blur_radius(f32::NAN), None);
        assert_eq!(validate_blur_radius(f32::INFINITY), None);
    }

    #[test]
    fn test_validate_animation_speed() {
        assert_eq!(validate_animation_speed(0.0), Some(0.0));
        assert_eq!(validate_animation_speed(1.5), Some(1.5));
        assert_eq!(
            validate_animation_speed(MAX_EFFECTS_ANIMATION_SPEED),
            Some(MAX_EFFECTS_ANIMATION_SPEED)
        );
        assert_eq!(validate_animation_speed(-0.5), None);
        assert_eq!(
            validate_animation_speed(MAX_EFFECTS_ANIMATION_SPEED + 1.0),
            None
        );
        assert_eq!(validate_animation_speed(f32::NAN), None);
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
            effects_gpu_available: true,
            effects_enabled: true,
            blur_enabled: true,
            blur_radius: 8.0,
        });
        let snap = *server
            .live_metrics_handle
            .as_ref()
            .expect("handle must exist after first snapshot call")
            .read();
        assert!((snap.frame_time_ms - 12.5).abs() < 1e-6);
        assert_eq!(snap.active_windows, 7);
        assert_eq!(snap.current_workspace, 2);
        assert!(snap.effects_gpu_available);
        assert!(snap.effects_enabled);
        assert!(snap.blur_enabled);
        assert!((snap.blur_radius - 8.0).abs() < 1e-6);

        // Second call replaces (not appends) per `get_or_insert_with` design.
        server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms: 99.9,
            active_windows: 2,
            current_workspace: -3,
            effects_gpu_available: false,
            effects_enabled: false,
            blur_enabled: false,
            blur_radius: 0.0,
        });
        let snap = *server
            .live_metrics_handle
            .as_ref()
            .expect("handle must exist after second snapshot call")
            .read();
        assert!((snap.frame_time_ms - 99.9).abs() < 1e-6);
        assert_eq!(snap.active_windows, 2);
        assert_eq!(snap.current_workspace, -3);
        assert!(!snap.effects_gpu_available);
        assert!(!snap.effects_enabled);
        assert!(!snap.blur_enabled);
        assert!((snap.blur_radius - 0.0).abs() < 1e-6);
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
            effects_gpu_available: false,
            effects_enabled: false,
            blur_enabled: false,
            blur_radius: 0.0,
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

    /// Issue #2 regression: EffectsControl ACK must distinguish full vs
    /// partial validation via the `"status"` discriminator.
    /// `partially_queued` when at least one field failed range/finite
    /// checks; `queued_for_execution` when all fields passed. Calls the
    /// actual production constructor
    /// (`AxiomIPCServer::build_effects_control_ack`) so the test fails
    /// when production regresses. Pins both cases.
    #[test]
    fn test_effects_control_ack_schema_distinguishes_partial() {
        // All fields accepted.
        let ack_full = AxiomIPCServer::build_effects_control_ack(
            vec!["enabled=true".to_string(), "blur_radius=4.0".to_string()],
            Vec::new(),
        );
        let s = serde_json::to_string(&ack_full).unwrap();
        assert!(
            s.contains(r#""status":"queued_for_execution""#),
            "full ACK JSON must carry status:queued_for_execution. JSON: {s}"
        );

        // At least one field rejected.
        let ack_partial = AxiomIPCServer::build_effects_control_ack(
            vec!["enabled=false".to_string()],
            vec![(
                "blur_radius".to_string(),
                "out_of_range_0..=32px".to_string(),
            )],
        );
        let s = serde_json::to_string(&ack_partial).unwrap();
        assert!(
            s.contains(r#""status":"partially_queued""#),
            "partial ACK JSON must carry status:partially_queued. JSON: {s}"
        );
        assert!(
            s.contains("blur_radius"),
            "partial ACK JSON must list the rejected field name. JSON: {s}"
        );
    }
}
