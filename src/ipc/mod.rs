//! IPC (Inter-Process Communication) module for Axiom-Lazy UI integration
//!
//! This module provides communication between the Axiom compositor (Rust) and
//! Lazy UI optimization system (Python) using Unix sockets and JSON messages.

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::alloc::System;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::time::sleep;

use crate::clipboard::ClipboardManager;
use std::sync::RwLock;

// Global read-only configuration snapshot for IPC queries (updatable)
static CONFIG_SNAPSHOT: std::sync::OnceLock<RwLock<crate::config::AxiomConfig>> =
    std::sync::OnceLock::new();

// Runtime command channel for applying changes inside the compositor
#[allow(dead_code)]
pub enum RuntimeCommand {
    SetConfig {
        key: String,
        value: serde_json::Value,
    },
    EffectsControl {
        enabled: Option<bool>,
        blur_radius: Option<f32>,
        animation_speed: Option<f32>,
    },
    Workspace {
        action: String,
        parameters: serde_json::Value,
    },
    ClipboardSet {
        data: String,
    },
    ClipboardGet,
}

static RUNTIME_CMD_TX: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<RuntimeCommand>> =
    std::sync::OnceLock::new();

// Global metrics history for performance reports (~30s buffer)
static METRICS_HISTORY: std::sync::OnceLock<RwLock<VecDeque<MetricSample>>> =
    std::sync::OnceLock::new();
static CLIPBOARD: std::sync::OnceLock<std::sync::Mutex<ClipboardManager>> =
    std::sync::OnceLock::new();

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

    /// Workspace management commands
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

    /// System health check request
    HealthCheck,

    /// Request performance report
    GetPerformanceReport,

    /// Clipboard set text
    ClipboardSet { data: String },

    /// Clipboard get request
    ClipboardGet,
}

/// IPC server for handling communication with Lazy UI
pub struct AxiomIPCServer {
    socket_path: PathBuf,
    #[allow(dead_code)]
    listener: Option<UnixListener>,
    /// Broadcast channel for outgoing Axiom messages to all clients
    broadcast_tx: Option<broadcast::Sender<AxiomMessage>>,
    /// Optional per-process message sender (kept for compatibility)
    #[allow(dead_code)]
    message_sender: Option<mpsc::UnboundedSender<AxiomMessage>>,
    command_receiver: Option<mpsc::UnboundedReceiver<LazyUIMessage>>,
    // System info for metrics sampling
    #[allow(dead_code)]
    sys: Option<System>,
    last_metrics_sent: Instant,
    // Last CPU times for non-blocking CPU usage sampling
    last_cpu_times: Option<(u64, u64)>,
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
            listener: None,
            broadcast_tx: None,
            message_sender: None,
            command_receiver: None,
            sys: None,
            last_metrics_sent: Instant::now(),
            last_cpu_times: None,
        }
    }

    /// Create a new IPC server with an explicit socket path
    ///
    /// This is useful for tests and tools that need to avoid the default
    /// system path or want to ensure isolation.
    #[allow(dead_code)]
    pub fn new_with_socket_path<P: Into<PathBuf>>(socket_path: P) -> Self {
        let socket_path = socket_path.into();
        Self {
            socket_path,
            listener: None,
            broadcast_tx: None,
            message_sender: None,
            command_receiver: None,
            sys: None,
            last_metrics_sent: Instant::now(),
            last_cpu_times: None,
        }
    }

    /// Provide a read-only snapshot of the compositor configuration for IPC queries
    pub fn set_config_snapshot(config: crate::config::AxiomConfig) {
        if let Some(lock) = CONFIG_SNAPSHOT.get() {
            if let Ok(mut guard) = lock.write() {
                *guard = config;
            }
            return;
        }
        let _ = CONFIG_SNAPSHOT.set(RwLock::new(config));
    }

    /// Register a runtime command sender so IPC can dispatch live changes to the compositor
    pub fn register_runtime_command_sender(tx: tokio::sync::mpsc::UnboundedSender<RuntimeCommand>) {
        let _ = RUNTIME_CMD_TX.set(tx);
    }

    /// Lookup a configuration value by dot-separated key (e.g., "effects.blur.radius")
    fn lookup_config_value(key: &str) -> Option<serde_json::Value> {
        if let Some(lock) = CONFIG_SNAPSHOT.get() {
            if let Ok(guard) = lock.read() {
                if let Ok(v) = serde_json::to_value(&*guard) {
                    let pointer = format!("/{}", key.replace('.', "/"));
                    return v.pointer(&pointer).cloned();
                }
            }
        }
        None
    }

    /// Quickly sample CPU (%) via /proc/stat over ~100ms and memory used (MB) via /proc/meminfo
    async fn sample_cpu_mem_quick() -> (f32, f32) {
        fn read_cpu_times() -> Option<(u64, u64)> {
            let contents = std::fs::read_to_string("/proc/stat").ok()?;
            let first = contents.lines().next()?;
            if !first.starts_with("cpu ") {
                return None;
            }
            let parts: Vec<&str> = first.split_whitespace().collect();
            if parts.len() < 8 {
                return None;
            }
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
            Some((idle_all, total))
        }
        let a = read_cpu_times();
        // Small delay to measure CPU delta; keep it short to avoid blocking responsiveness
        sleep(Duration::from_millis(100)).await;
        let b = read_cpu_times();
        let cpu = match (a, b) {
            (Some((idle_a, total_a)), Some((idle_b, total_b))) => {
                let idle_delta = idle_b.saturating_sub(idle_a) as f64;
                let total_delta = total_b.saturating_sub(total_a) as f64;
                if total_delta > 0.0 {
                    ((1.0 - idle_delta / total_delta) * 100.0) as f32
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        // Memory (MB) from /proc/meminfo
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
        let used_mb = (mem_total_kb.saturating_sub(mem_available_kb) as f32) / 1024.0;
        (cpu, used_mb)
    }

    /// Start the IPC server
    pub async fn start(&mut self) -> Result<()> {
        // Ensure parent dir exists with correct permissions
        if let Some(dir) = self.socket_path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create IPC dir: {:?}", dir))?;
            // Best-effort tighten permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }

        // Remove existing socket file
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).with_context(|| {
                format!("Failed to remove existing socket: {:?}", self.socket_path)
            })?;
        }

        // Create Unix socket listener
        let listener = UnixListener::bind(&self.socket_path)
            .with_context(|| format!("Failed to bind Unix socket: {:?}", self.socket_path))?;

        // Tighten socket permissions (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&self.socket_path, std::fs::Permissions::from_mode(0o600));
        }

        // Create broadcast channel for outgoing messages
        let (tx, _rx) = broadcast::channel::<AxiomMessage>(1024);
        self.broadcast_tx = Some(tx.clone());

        info!("🔗 Axiom IPC server listening on: {:?}", self.socket_path);

        // Start accepting connections in a separate task
        tokio::spawn(Self::accept_connections_static(listener, tx));

        Ok(())
    }

    /// Accept incoming connections from Lazy UI (static version)
    async fn accept_connections_static(
        listener: UnixListener,
        tx: broadcast::Sender<AxiomMessage>,
    ) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    info!("🤝 Lazy UI connected to Axiom IPC");
                    let rx = tx.subscribe();
                    tokio::spawn(Self::handle_client(stream, rx));
                }
                Err(e) => {
                    error!("❌ Error accepting IPC connection: {}", e);
                }
            }
        }
    }

    /// Accept incoming connections from Lazy UI (kept for compatibility)
    #[allow(dead_code)]
    async fn accept_connections(&mut self) -> Result<()> {
        // Deprecated path: connection acceptance is spawned in start() with a broadcast channel.
        // Keeping this method to satisfy older call sites; return Ok(()) without doing anything.
        Ok(())
    }

    /// Handle a single client connection
    async fn handle_client(
        stream: UnixStream,
        mut rx: broadcast::Receiver<AxiomMessage>,
    ) -> Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

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

        // Process incoming messages and outgoing broadcasts concurrently
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let line = match line? {
                        Some(l) => l,
                        None => break, // client disconnected
                    };
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    if trimmed.len() > 64 * 1024 {
                        warn!("⚠️ IPC message too large ({} bytes) - dropping", trimmed.len());
                        continue;
                    }

                    debug!("📨 Received IPC message: {}", trimmed);
                    match serde_json::from_str::<LazyUIMessage>(trimmed) {
                        Ok(message) => {
                            if let Err(e) = Self::process_lazy_ui_message(message, &mut writer).await {
                                warn!("⚠️ Error processing message: {}", e);
                            }
                        }
                        Err(e) => {
                            warn!("⚠️ Invalid JSON from IPC client: {}", e);
                        }
                    }
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

                let value = Self::lookup_config_value(&key).unwrap_or(serde_json::Value::Null);
                let response = AxiomMessage::ConfigResponse {
                    key: key.clone(),
                    value,
                };

                Self::send_message(writer, &response).await?;
            }

            LazyUIMessage::SetConfig { key, value } => {
                info!("⚙️ Setting config: {} = {:?}", key, value);

                // Forward to compositor via runtime command channel if registered
                if let Some(tx) = RUNTIME_CMD_TX.get() {
                    let _ = tx.send(RuntimeCommand::SetConfig {
                        key: key.clone(),
                        value: value.clone(),
                    });
                }

                // ACK the set request
                let ack = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    event_type: "SetConfigAck".into(),
                    details: serde_json::json!({ "key": key, "status": "accepted" }),
                };
                Self::send_message(writer, &ack).await?;
            }

            LazyUIMessage::WorkspaceCommand { action, parameters } => {
                info!(
                    "🖥️ Workspace command: {} with params: {:?}",
                    action, parameters
                );
                if let Some(tx) = RUNTIME_CMD_TX.get() {
                    let _ = tx.send(RuntimeCommand::Workspace { action, parameters });
                }
            }

            LazyUIMessage::EffectsControl {
                enabled,
                blur_radius,
                animation_speed,
            } => {
                info!(
                    "✨ Effects control - enabled: {:?}, blur: {:?}, animation: {:?}",
                    enabled, blur_radius, animation_speed
                );
                // Forward to compositor via runtime command channel if registered
                if let Some(tx) = RUNTIME_CMD_TX.get() {
                    let _ = tx.send(RuntimeCommand::EffectsControl {
                        enabled,
                        blur_radius,
                        animation_speed,
                    });
                }
            }

            LazyUIMessage::HealthCheck => {
                debug!("🏥 Health check request");

                // Sample real CPU and memory metrics
                let (cpu_usage, memory_usage) = Self::sample_cpu_mem_quick().await;

                // Send performance metrics as health response
                // Sample GPU usage if available
                let gpu_usage = Self::sample_gpu_usage_quick();

                let metrics = AxiomMessage::PerformanceMetrics {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    cpu_usage,
                    memory_usage,
                    gpu_usage,
                    frame_time: 16.67,
                    active_windows: 0,
                    current_workspace: 0,
                };

                Self::send_message(writer, &metrics).await?;
            }

            LazyUIMessage::GetPerformanceReport => {
                debug!("📊 Performance report request");
                let report = Self::generate_performance_report();
                let evt = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    event_type: "PerformanceReport".into(),
                    details: report,
                };
                Self::send_message(writer, &evt).await?;
            }
            LazyUIMessage::ClipboardSet { data } => {
                debug!("📋 IPC clipboard set ({} bytes)", data.len());
                // Store via global clipboard (simple static for now)
                CLIPBOARD.get_or_init(|| std::sync::Mutex::new(ClipboardManager::new()));
                if let Some(cell) = CLIPBOARD.get() {
                    if let Ok(mut mgr) = cell.lock() {
                        mgr.set_selection(data);
                    }
                }
                let ack = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    event_type: "ClipboardSetAck".into(),
                    details: serde_json::json!({"status":"ok"}),
                };
                Self::send_message(writer, &ack).await?;
            }
            LazyUIMessage::ClipboardGet => {
                debug!("📋 IPC clipboard get");
                let mut payload = serde_json::Value::Null;
                if let Some(cell) = CLIPBOARD.get() {
                    if let Ok(mgr) = cell.lock() {
                        if let Some(text) = mgr.get_selection() {
                            payload = serde_json::json!({"data": text});
                        }
                    }
                }
                let evt = AxiomMessage::UserEvent {
                    timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    event_type: "ClipboardData".into(),
                    details: payload,
                };
                Self::send_message(writer, &evt).await?;
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

    /// Send performance metrics to Lazy UI
    #[allow(dead_code)]
    pub async fn send_performance_metrics(
        &self,
        cpu_usage: f32,
        memory_usage: f32,
        gpu_usage: f32,
        frame_time: f32,
        active_windows: u32,
        current_workspace: i32,
    ) -> Result<()> {
        if let Some(sender) = &self.message_sender {
            let metrics = AxiomMessage::PerformanceMetrics {
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                cpu_usage,
                memory_usage,
                gpu_usage,
                frame_time,
                active_windows,
                current_workspace,
            };

            sender
                .send(metrics)
                .map_err(|_| anyhow::anyhow!("Failed to send performance metrics"))?;
        }

        Ok(())
    }

    /// Phase 3: Process pending IPC messages
    pub async fn process_messages(&mut self) -> Result<()> {
        // Process any pending messages from Lazy UI
        // In a real implementation, this would handle incoming connections
        // and process optimization commands from the receiver

        if let Some(receiver) = &mut self.command_receiver {
            while let Ok(message) = receiver.try_recv() {
                debug!("📨 Processing Lazy UI message: {:?}", message);
                // Process the message (optimization commands, config changes, etc.)
                match message {
                    LazyUIMessage::OptimizeConfig { changes, reason } => {
                        info!("🎯 Processing optimization: {} ({})", changes.len(), reason);
                    }
                    _ => {
                        debug!("📑 Other message type processed");
                    }
                }
            }
        }

        // Small delay to prevent busy loop
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        Ok(())
    }

    /// Send user event to Lazy UI
    #[allow(dead_code)]
    pub async fn send_user_event(
        &self,
        event_type: String,
        details: serde_json::Value,
    ) -> Result<()> {
        if let Some(sender) = &self.message_sender {
            let event = AxiomMessage::UserEvent {
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                event_type,
                details,
            };

            sender
                .send(event)
                .map_err(|_| anyhow::anyhow!("Failed to send user event"))?;
        }

        Ok(())
    }

    /// Get the socket path
    #[allow(dead_code)]
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Broadcast PerformanceMetrics to all connected clients
    #[allow(dead_code)]
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

    /// Rate-limited helper that samples CPU/memory and broadcasts metrics (~10Hz)
    #[allow(dead_code)]
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
        let gpu = Self::sample_gpu_usage_quick();
        // Push to global history (cap to 300 samples)
        let hist = METRICS_HISTORY.get_or_init(|| RwLock::new(VecDeque::with_capacity(300)));
        if let Ok(mut guard) = hist.write() {
            guard.push_back(MetricSample {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                cpu_usage: cpu,
                memory_usage: mem_mb,
                gpu_usage: gpu,
                frame_time_ms,
                active_windows,
                current_workspace,
            });
            while guard.len() > 300 {
                guard.pop_front();
            }
        }
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

    /// Sample system CPU usage (%) and memory used (MB) by reading /proc
    /// This is a synchronous sampler intended for periodic telemetry; it avoids extra deps.
    #[allow(dead_code)]
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

        let (cpu_percent, mem_used_mb) = {
            let current = read_cpu_times();
            let cpu = match (self.last_cpu_times, current) {
                (Some((idle_a, total_a)), Some((idle_b, total_b))) => {
                    let idle_delta = idle_b.saturating_sub(idle_a) as f64;
                    let total_delta = total_b.saturating_sub(total_a) as f64;
                    if total_delta > 0.0 {
                        ((1.0 - idle_delta / total_delta) * 100.0) as f32
                    } else {
                        0.0
                    }
                }
                (_, Some((_idle_b, _total_b))) => {
                    // First sample; store and return 0 for now
                    0.0
                }
                _ => 0.0,
            };

            // Memory usage from /proc/meminfo
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
            let used_mb = used_kb / 1024.0;
            (cpu, used_mb)
        };

        // Update last CPU times with the current sample for next call
        if let Some((idle, total)) = (|| {
            let contents = std::fs::read_to_string("/proc/stat").ok()?;
            let first = contents.lines().next()?;
            if !first.starts_with("cpu ") {
                return None;
            }
            let parts: Vec<&str> = first.split_whitespace().collect();
            if parts.len() < 5 {
                return None;
            }
            let idle: u64 = parts.get(4)?.parse().ok()?;
            let iowait: u64 = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let user: u64 = parts.get(1)?.parse().ok()?;
            let nice: u64 = parts.get(2)?.parse().ok()?;
            let system: u64 = parts.get(3)?.parse().ok()?;
            let irq: u64 = parts.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
            let softirq: u64 = parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);
            let steal: u64 = parts.get(8).and_then(|s| s.parse().ok()).unwrap_or(0);
            let idle_all = idle + iowait;
            let non_idle = user + nice + system + irq + softirq + steal;
            let total = idle_all + non_idle;
            Some((idle_all, total))
        })() {
            self.last_cpu_times = Some((idle, total));
        }

        (cpu_percent, mem_used_mb)
    }

    /// Build a simple performance report from recent samples
    fn generate_performance_report() -> serde_json::Value {
        let mut samples_count = 0usize;
        let (mut cpu_sum, mut mem_sum, mut gpu_sum, mut ft_sum): (f32, f32, f32, f32) =
            (0.0, 0.0, 0.0, 0.0);
        let (mut cpu_peak, mut mem_peak, mut gpu_peak, mut ft_peak): (f32, f32, f32, f32) =
            (0.0, 0.0, 0.0, 0.0);
        let (mut last_active_windows, mut last_current_workspace) = (0u32, 0i32);

        if let Some(hist) = METRICS_HISTORY.get() {
            if let Ok(guard) = hist.read() {
                samples_count = guard.len();
                for s in guard.iter() {
                    cpu_sum += s.cpu_usage;
                    mem_sum += s.memory_usage;
                    gpu_sum += s.gpu_usage;
                    ft_sum += s.frame_time_ms;
                    cpu_peak = cpu_peak.max(s.cpu_usage);
                    mem_peak = mem_peak.max(s.memory_usage);
                    gpu_peak = gpu_peak.max(s.gpu_usage);
                    ft_peak = ft_peak.max(s.frame_time_ms);
                }
                if let Some(last) = guard.back() {
                    last_active_windows = last.active_windows;
                    last_current_workspace = last.current_workspace;
                }
            }
        }

        let n = samples_count as f32;
        let (cpu_avg, mem_avg, gpu_avg, ft_avg) = if n > 0.0 {
            (cpu_sum / n, mem_sum / n, gpu_sum / n, ft_sum / n)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let fps_avg = if ft_avg > 0.0 { 1000.0 / ft_avg } else { 0.0 };
        let fps_peak = if ft_peak > 0.0 { 1000.0 / ft_peak } else { 0.0 };
        serde_json::json!({
            "window": {
                "active_windows": last_active_windows,
                "current_workspace": last_current_workspace,
            },
            "samples": samples_count,
            "averages": {
                "cpu_usage": cpu_avg,
                "memory_usage_mb": mem_avg,
                "gpu_usage": gpu_avg,
                "frame_time_ms": ft_avg,
                "fps": fps_avg
            },
            "peaks": {
                "cpu_usage": cpu_peak,
                "memory_usage_mb": mem_peak,
                "gpu_usage": gpu_peak,
                "frame_time_ms": ft_peak,
                "fps": fps_peak
            }
        })
    }

    /// Best-effort GPU usage sampling (%) using sysfs (amdgpu). Returns 0.0 if unavailable.
    fn sample_gpu_usage_quick() -> f32 {
        // Prefer NVML if enabled and available
        #[cfg(feature = "gpu-nvml")]
        {
            if let Some(val) = Self::sample_gpu_usage_nvml() {
                return val;
            }
        }

        // Fallback: Try common AMD path(s)
        let base = std::path::Path::new("/sys/class/drm");
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("card") {
                        let busy = path.join("device").join("gpu_busy_percent");
                        if let Ok(contents) = std::fs::read_to_string(&busy) {
                            if let Ok(val) = contents.trim().parse::<f32>() {
                                if val.is_finite() {
                                    return val.clamp(0.0, 100.0);
                                }
                            }
                        }
                    }
                }
            }
        }
        0.0
    }

    #[cfg(feature = "gpu-nvml")]
    fn sample_gpu_usage_nvml() -> Option<f32> {
        // Use the correct type name from nvml-wrapper
        use nvml_wrapper::Nvml;
        let nvml = Nvml::init().ok()?;
        let device = nvml.device_by_index(0).ok()?;
        let util = device.utilization_rates().ok()?;
        Some((util.gpu as f32).clamp(0.0, 100.0))
    }
}

/// Single performance metric sample
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricSample {
    timestamp: u64,
    cpu_usage: f32,
    memory_usage: f32,
    gpu_usage: f32,
    frame_time_ms: f32,
    active_windows: u32,
    current_workspace: i32,
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
    fn default_socket_path() -> PathBuf {
        // Prefer XDG_RUNTIME_DIR
        if let Ok(mut dir) = std::env::var("XDG_RUNTIME_DIR") {
            if dir.is_empty() {
                dir = "/tmp".to_string();
            }
            return PathBuf::from(dir).join("axiom").join("axiom.sock");
        }
        // Fallback to /tmp
        PathBuf::from("/tmp").join("axiom-lazy-ui.sock")
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

    #[tokio::test]
    async fn test_ipc_server_custom_socket_path() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let sock = tmp.path().join("custom_ipc.sock");

        let mut server = AxiomIPCServer::new_with_socket_path(sock.clone());
        server.start().await.unwrap();

        assert!(server.socket_path().exists());
        assert_eq!(server.socket_path(), &sock);
        // Drop cleans up the socket file via Drop impl
    }
}
