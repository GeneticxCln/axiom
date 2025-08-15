//! IPC (Inter-Process Communication) module for Axiom-Lazy UI integration
//!
//! This module provides communication between the Axiom compositor (Rust) and
//! Lazy UI optimization system (Python) using Unix sockets and JSON messages.

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

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
    sys: Option<System>,
    last_metrics_sent: Instant,
}

impl Default for AxiomIPCServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AxiomIPCServer {
    /// Create a new IPC server
    pub fn new() -> Self {
        let socket_path = PathBuf::from("/tmp/axiom-lazy-ui.sock");

        Self {
            socket_path,
            listener: None,
            broadcast_tx: None,
            message_sender: None,
            command_receiver: None,
            sys: None,
            last_metrics_sent: Instant::now(),
        }
    }

    /// Start the IPC server
    pub async fn start(&mut self) -> Result<()> {
        // Remove existing socket file
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).with_context(|| {
                format!("Failed to remove existing socket: {:?}", self.socket_path)
            })?;
        }

        // Create Unix socket listener
        let listener = UnixListener::bind(&self.socket_path)
            .with_context(|| format!("Failed to bind Unix socket: {:?}", self.socket_path))?;

        info!("🔗 Axiom IPC server listening on: {:?}", self.socket_path);

        // Start accepting connections in a separate task
        tokio::spawn(Self::accept_connections_static(listener));

        Ok(())
    }

    /// Accept incoming connections from Lazy UI (static version)
async fn accept_connections_static(listener: UnixListener, tx: broadcast::Sender<AxiomMessage>) -> Result<()> {
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
    async fn accept_connections(&mut self) -> Result<()> {
        let listener = self
            .listener
            .take()
            .ok_or_else(|| anyhow::anyhow!("IPC server not started"))?;

        Self::accept_connections_static(listener).await
    }

    /// Handle a single client connection
async fn handle_client(stream: UnixStream, mut rx: broadcast::Receiver<AxiomMessage>) -> Result<()> {
let (reader, writer) = stream.into_split();
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

        // Process incoming messages
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }

            debug!("📨 Received IPC message: {}", line);

            match serde_json::from_str::<LazyUIMessage>(&line) {
                Ok(message) => {
                    if let Err(e) = Self::process_lazy_ui_message(message, &mut writer).await {
                        warn!("⚠️ Error processing message: {}", e);
                    }
                }
                // Outgoing broadcast message to client
                msg = rx.recv() => {
                    match msg {
                        Ok(message) => {
                            let mut w = writer.lock().await;
                            if let Err(e) = Self::send_message(&mut w, &message).await {
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

                for (key, value) in changes {
                    debug!("  📝 Setting {}: {:?}", key, value);
                    // TODO: Actually apply configuration changes to Axiom
                }
            }

            LazyUIMessage::GetConfig { key } => {
                debug!("📋 Config query: {}", key);

                // TODO: Get actual configuration value from Axiom
                let response = AxiomMessage::ConfigResponse {
                    key: key.clone(),
                    value: serde_json::Value::String("default_value".to_string()),
                };

                Self::send_message(writer, &response).await?;
            }

            LazyUIMessage::SetConfig { key, value } => {
                info!("⚙️ Setting config: {} = {:?}", key, value);
                // TODO: Actually set configuration in Axiom
            }

            LazyUIMessage::WorkspaceCommand { action, parameters } => {
                info!(
                    "🖥️ Workspace command: {} with params: {:?}",
                    action, parameters
                );
                // TODO: Execute workspace command
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
                // TODO: Apply effects changes
            }

            LazyUIMessage::HealthCheck => {
                debug!("🏥 Health check request");

                // Send performance metrics as health response
                let metrics = AxiomMessage::PerformanceMetrics {
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)?
                        .as_secs(),
                    cpu_usage: 15.5, // TODO: Get real metrics
                    memory_usage: 32.1,
                    gpu_usage: 8.3,
                    frame_time: 16.67,
                    active_windows: 0,
                    current_workspace: 0,
                };

                Self::send_message(writer, &metrics).await?;
            }

            LazyUIMessage::GetPerformanceReport => {
                debug!("📊 Performance report request");
                // TODO: Generate comprehensive performance report
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
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
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

    /// Rate-limited helper that samples CPU/memory and broadcasts metrics (~10Hz)
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
        let (cpu, mem_mb) = self.sample_system_metrics();
        let _ = self.broadcast_performance_metrics(
            cpu,
            mem_mb,
            0.0, // GPU TBD
            frame_time_ms,
            active_windows,
            current_workspace,
        );
        self.last_metrics_sent = Instant::now();
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
}
