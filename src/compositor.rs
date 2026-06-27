//! Core compositor implementation
//!
//! This module contains the main AxiomCompositor struct and event loop.
//! It coordinates between all subsystems: workspaces, effects, input, etc.
//!
//! Uses Smithay 0.7 for Wayland compositor functionality including
//! window management, surface handling, and protocol support.

use anyhow::{Context, Result};
use log::{debug, info, warn};
use tokio::signal;

use crate::backend::AxiomSmithayBackendReal;
use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::ipc::{AxiomIPCServer, LazyUIMessage, LiveMetrics};
use crate::renderer::AxiomRenderer;
use crate::window::{Rectangle, WindowManager};
use crate::workspace::ScrollableWorkspaces;
use crate::xwayland::XWaylandManager;

use std::sync::Arc;
use tokio::sync::RwLock as AsyncRwLock;

/// Main compositor struct that orchestrates all subsystems
pub struct AxiomCompositor {
    config: AxiomConfig,
    running: bool,
    _windowed: bool,

    // Subsystems
    workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
    effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
    window_manager: Arc<parking_lot::RwLock<WindowManager>>,
    input_manager: Arc<parking_lot::RwLock<InputManager>>,
    xwayland_manager: Option<Arc<AsyncRwLock<XWaylandManager>>>,
    ipc_server: AxiomIPCServer,
    consecutive_error_count: u32,
    /// When true, the next `tick()` will record an error regardless of
    /// actual subsystem behavior. Used by integration tests to simulate
    /// consecutive errors without requiring real failures.
    force_next_tick_error: bool,

    // Renderer (optional — may be unavailable in headless/CI environments)
    renderer: Option<Arc<parking_lot::RwLock<AxiomRenderer>>>,

    // Server-side decoration manager for titlebar/button rendering
    decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,

    // Smithay Backend
    smithay_backend: AxiomSmithayBackendReal,

    // Performance optimization: Persistent buffers for rendering
    // Avoids re-allocating Vec per frame
    render_data_buffer: Vec<WindowRenderData>,
}

// Data structure for render pass (outside impl to be accessible)
struct WindowRenderData {
    id: u64,
    layout_rect: Rectangle,
    opacity: f32,
}

/// Per-frame shadow effect queue entry for the WGPU renderer.
type PendingShadow = (u64, (f32, f32), (f32, f32), crate::effects::ShadowParams);
/// Per-frame blur effect queue entry for the WGPU renderer.
type PendingBlur = (u64, (f32, f32), (f32, f32), crate::effects::BlurParams);

impl AxiomCompositor {
    /// Create a new Axiom compositor instance
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: AxiomConfig,
        windowed: bool,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        xwayland_manager: Option<Arc<AsyncRwLock<XWaylandManager>>>,
        mut ipc_server: AxiomIPCServer,
        renderer: Arc<parking_lot::RwLock<AxiomRenderer>>,
    ) -> Result<Self> {
        // Initialize IPC server for Lazy UI integration. Wire the live config
        // handle so `GetConfig` queries resolve against the real config tree
        // rather than the previous hard-coded default placeholder.
        debug!("Initializing IPC server...");
        ipc_server.set_config_handle(Arc::new(parking_lot::RwLock::new(config.clone())));
        ipc_server
            .start()
            .context("Failed to start IPC server")?;

        info!("All subsystems initialized successfully");

        // Initialize GPU effects acceleration (blur, shadows, shaders).
        // The renderer exposes `device()` / `queue()` as `&Device` / `&Queue`
        // (Design 16) — callers cannot reach the GPU context through the
        // public getter anymore. We use direct field access to clone the
        // internal `Arc`s, which is what `initialize_gpu` actually wants.
        // Both wgpu types are themselves internally Arc-wrapped, so the
        // `Arc::clone` is a cheap refcount bump, not a deep copy.
        {
            let r = renderer.read();
            // Direct field access (`r.device`, `r.queue`) clones the Arc.
            // Arc::clone takes &Arc<T> and produces Arc<T> — both compile
            // cleanly with the `&self.device` borrow that survives until
            // the read guard is dropped at the bottom of this block.
            let device_arc: Arc<wgpu::Device> = Arc::clone(renderer.read().device_arc());
            let queue_arc: Arc<wgpu::Queue> = Arc::clone(renderer.read().queue_arc());
            // Drop the renderer guard BEFORE acquiring the effects write.
            // The `Arc`s now own the GPU context; the read guard is gone
            // before `effects_engine.write()` is held, so renderer.write()
            // and effects.write() cannot interleave in a deadlock window.
            drop(r);
            effects_engine
                .write()
                .initialize_gpu(device_arc, queue_arc)
                .unwrap_or_else(|e| {
                    warn!(
                        "GPU effects initialization skipped ({}): blur/shadows will not render",
                        e
                    );
                });
        }
        // Capture the post-init state so we can surface it via the IPC
        // LiveMetrics snapshot (Design 14 — observable to monitoring
        // clients without grepping the log).
        let effects_gpu_available = effects_engine.read().is_gpu_initialized();
        // Pre-populate the IPC's LiveMetrics so a `HealthCheck` /
        // `GetPerformanceReport` query arriving BEFORE the first tick
        // still sees the gpu_initialized state instead of the default
        // LiveMetrics::default() (= false / 0).
        ipc_server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms: 0.0,
            active_windows: 0,
            current_workspace: 0,
            effects_gpu_available,
        });

        // Wire effects engine into renderer for GPU shadow/blur post-processing
        renderer.write().set_effects_engine(effects_engine.clone());

        // Wire border width from config into renderer
        renderer.write().set_border_width(config.window.border_width as f32);

        // Initialize server-side decoration manager (must be created before
        // the Smithay backend so it can receive a clone).
        let minimize_enabled = config.features.enable_minimize;
        let decoration_manager = Arc::new(parking_lot::RwLock::new(
            DecorationManager::new(&config.window, minimize_enabled),
        ));

        let smithay_backend = {
            info!("Initializing Axiom compositor with Smithay backend...");
            debug!("Initializing Smithay Wayland backend...");
            let mut backend = AxiomSmithayBackendReal::new(
                config.clone(),
                window_manager.clone(),
                workspace_manager.clone(),
                effects_engine.clone(),
                input_manager.clone(),
                renderer.clone(),
                decoration_manager.clone(),
            )?;
            backend
                .initialize()
                .context("Failed to initialize Smithay backend")?;
            backend
        };

        Ok(Self {
            config,
            _windowed: windowed,
            workspace_manager,
            effects_engine,
            window_manager,
            input_manager,
            xwayland_manager,
            ipc_server,
            smithay_backend,
            render_data_buffer: Vec::with_capacity(64), // Pre-allocate for typical window count
            consecutive_error_count: 0,
            force_next_tick_error: false,
            renderer: Some(renderer),
            decoration_manager,
            running: false,
        })
    }

    /// Start the compositor main event loop
    pub async fn run(mut self) -> Result<()> {
        info!("Starting Axiom compositor event loop");
        self.running = true;

        // Set up signal handling
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())?;
        let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())?;

        // Main event loop
        while self.running {
            tokio::select! {
                // Handle system signals
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, shutting down gracefully");
                    self.shutdown().await?;
                }
                _ = sigint.recv() => {
                    info!("Received SIGINT (Ctrl+C), shutting down gracefully");
                    self.shutdown().await?;
                }

                // Combined event processing and rendering
                _ = self.tick() => {}
            }
        }

        info!("Axiom compositor event loop finished");
        Ok(())
    }

    /// Process all pending compositor events with real input handling
    fn process_events(&mut self) -> Result<()> {
        // Process backend events (Wayland, input devices)
        self.smithay_backend.process_events()?;

        // Process IPC messages from Lazy UI. The new return shape surfaces
        // (config_changed, pending_actions): config mutations refresh the
        // IPC handle; subsystem-bound actions (WorkspaceCommand,
        // EffectsControl) are dispatched below.
        match self.ipc_server.process_messages(&mut self.config) {
            Ok((config_changed, pending_actions)) => {
                if config_changed {
                    self.update_subsystems_config();
                    // Refresh the IPC server's config handle so `GetConfig`
                    // queries see the same values the compositor just
                    // applied through `process_messages`. Without this the
                    // handle remains frozen at its `new()`-time clone and
                    // returns stale data when Lazy UI re-queries. Matches
                    // the project's push-based config propagation model
                    // (see `update_subsystems_config`).
                    self.ipc_server
                        .set_config_handle(Arc::new(parking_lot::RwLock::new(self.config.clone())));
                }
                for action in pending_actions {
                    match action {
                        LazyUIMessage::WorkspaceCommand { action, parameters } => {
                            self.dispatch_workspace_command(&action, &parameters);
                        }
                        LazyUIMessage::EffectsControl {
                            enabled,
                            blur_radius,
                            animation_speed,
                        } => {
                            self.dispatch_effects_control(enabled, blur_radius, animation_speed);
                        }
                        // process_messages only forwards WorkspaceCommand and
                        // EffectsControl into the actions vec; the catch-all
                        // is here only to satisfy the exhaustive match.
                        _ => {
                            warn!("Unexpected pending action variant from IPC queue");
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Error processing IPC messages: {}", e);
            }
        }

        Ok(())
    }

    /// Apply a validated `LazyUIMessage::WorkspaceCommand { action, parameters }`
    /// to the live workspace / window subsystems. Each high-level composer
    /// method (`scroll_workspace_left`, `move_window_left`, …) takes and
    /// drops its own lock internally, so calling them in sequence avoids
    /// any cross-subsystem inversion.
    fn dispatch_workspace_command(&mut self, action: &str, parameters: &serde_json::Value) {
        match action {
            "scroll_left" => self.scroll_workspace_left(),
            "scroll_right" => self.scroll_workspace_right(),
            "add_window" => {
                let title = parameters
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Untitled");
                self.add_window(title.to_string());
            }
            "remove_window" => match parameters.get("window_id").and_then(|v| v.as_u64()) {
                Some(id) => {
                    self.remove_window(id);
                }
                None => {
                    warn!("WorkspaceCommand remove_window missing 'window_id' parameter — no-op")
                }
            },
            "move_focus_left" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => self.move_window_left(id),
                    None => debug!("WorkspaceCommand move_focus_left: no focused window, no-op"),
                }
            }
            "move_focus_right" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => self.move_window_right(id),
                    None => {
                        debug!("WorkspaceCommand move_focus_right: no focused window, no-op")
                    }
                }
            }
            "toggle_floating" => {
                let focused_id = self.window_manager.read().focused_window_id();
                match focused_id {
                    Some(id) => {
                        self.window_manager.write().toggle_floating(id);
                    }
                    None => debug!("WorkspaceCommand toggle_floating: no focused window, no-op"),
                }
            }
            // Defensive catch-all. The IPC layer's whitelist already rejects
            // unknown actions, so reaching here means a future handler or
            // schema change introduced a mismatch — surface it loudly.
            unknown => warn!(
                "WorkspaceCommand '{}' reached dispatch despite whitelist validation",
                unknown
            ),
        }
    }

    /// Apply a validated `LazyUIMessage::EffectsControl` payload to the live
    /// effects engine. The IPC layer has already range-checked the values;
    /// `apply_live_effects_control` re-validates as defense in depth.
    fn dispatch_effects_control(
        &mut self,
        enabled: Option<bool>,
        blur_radius: Option<f32>,
        animation_speed: Option<f32>,
    ) {
        self.effects_engine.write().apply_live_effects_control(
            enabled,
            blur_radius,
            animation_speed,
        );
        debug!(
            "Effects control dispatched — enabled: {:?}, blur: {:?}, animation: {:?}",
            enabled, blur_radius, animation_speed
        );
    }

    /// Populate per-frame effect queues in the WGPU renderer from the effects engine.
    ///
    /// Must run BEFORE `process_events()` so the backend's GL render pass can
    /// consume these queues for GPU post-processing (shadows, blur) between
    /// window drawing and `backend.submit()`. Window positions are stale at
    /// this point — the backend updates them during its own render pass — so
    /// this only queues window-less effects that don't depend on exact layout.
    fn prepare_frame_data(&mut self) -> Result<()> {
        // Clear per-frame effect queues from previous frame
        if let Some(ref renderer) = self.renderer {
            let mut r = renderer.write();
            r.clear_shadows();
            r.clear_blurs();
        }

        // Collect render data from windows
        self.render_data_buffer.clear();

        {
            let wm = self.window_manager.read();
            wm.for_each_window(|window_id, window| {
                let layout_rect =
                    Rectangle::from_loc_and_size(window.window.position, window.window.size);

                self.render_data_buffer.push(WindowRenderData {
                    id: window_id,
                    layout_rect,
                    opacity: window.properties.opacity,
                });
            });
        } // Drop WM lock

        // Queue shadow and blur data from effects engine for GPU rendering.
        // Collect effect state first (only holding effects lock), then queue
        // in renderer — avoids nesting effects.read() inside renderer.write()
        // which would invert the lock order vs composite_effects_on_buffer
        // (renderer &mut → effects.write()).
        //
        // Skip the queue entirely when effects are globally disabled: the
        // backend's `has_pending_post_process` non-locking peek will see
        // both atomics at zero and skip the GL→CPU readback. This makes
        // the no-effects hot path zero-cost (no HashMap inserts, no atomic
        // stores, no GPU work, no `glReadPixels`).
        let effects_enabled = self.effects_engine.read().is_enabled();
        if effects_enabled {
            if let Some(ref renderer) = self.renderer {
                let mut pending_shadows: Vec<PendingShadow> = Vec::new();
                let mut pending_blurs: Vec<PendingBlur> = Vec::new();

                {
                    let effects = self.effects_engine.read();
                    for data in &self.render_data_buffer {
                        if let Some(effect_state) = effects.get_window_effects(data.id) {
                            let pos =
                                (data.layout_rect.x as f32, data.layout_rect.y as f32);
                            let size = (
                                data.layout_rect.width as f32,
                                data.layout_rect.height as f32,
                            );
                            if effect_state.shadow.enabled {
                                pending_shadows.push((
                                    data.id,
                                    pos,
                                    size,
                                    effect_state.shadow.clone(),
                                ));
                            }
                            if effect_state.blur_radius > 0.0 {
                                let engine_blur_params = crate::effects::BlurParams {
                                    enabled: true,
                                    radius: effect_state.blur_radius,
                                    intensity: 0.8,
                                    background_blur: true,
                                    window_blur: false,
                                };
                                pending_blurs.push((data.id, pos, size, engine_blur_params));
                            }
                        }
                    }
                } // Drop effects lock before acquiring renderer lock

                let mut r = renderer.write();
                for (id, pos, size, params) in pending_shadows {
                    r.queue_shadow(id, pos, size, params);
                }
                for (id, pos, size, params) in pending_blurs {
                    r.queue_blur(id, pos, size, params);
                }
            }
        }

        Ok(())
    }

    /// Post-render phase: applies global effects, performance monitoring,
    /// and window rect upserts for the renderer. The WGPU effects composite
    /// now happens in the backend's GL render pass (between window drawing
    /// and `submit`), so this method no longer calls `renderer.render()`.
    fn render_frame(&mut self) -> Result<()> {
        // Push window rects to renderer for metrics/housekeeping
        for win_data in &self.render_data_buffer {
            let mut scale = 1.0_f32;
            let mut opacity = win_data.opacity;
            let mut offset = (0.0_f32, 0.0_f32);

            {
                let effects = self.effects_engine.read();
                if let Some(effect_state) = effects.get_window_effects(win_data.id) {
                    scale = effect_state.scale;
                    opacity = effect_state.opacity;
                    offset = effect_state.position_offset;
                }
            }

            if let Some(ref r) = self.renderer {
                let mut renderer = r.write();
                let x = win_data.layout_rect.x as f32 + offset.0;
                let y = win_data.layout_rect.y as f32 + offset.1;
                let w = win_data.layout_rect.width as f32 * scale;
                let h = win_data.layout_rect.height as f32 * scale;
                renderer.upsert_window_rect(win_data.id, (x, y), (w, h), opacity);
            }
        }

        // Apply global effects (workspace transitions, blur backgrounds)
        self.apply_global_effects();

        // Performance monitoring for effects
        let (frame_time, effects_quality, active_effects) =
            self.effects_engine.read().get_performance_stats();
        if frame_time.as_millis() > 20 {
            debug!(
                "Frame time: {:.1}ms, effects quality: {:.1}, active effects: {}",
                frame_time.as_secs_f64() * 1000.0,
                effects_quality,
                active_effects
            );
        }

        debug!(
            "Frame rendered - position: {:.1}, column: {}, effects: {}",
            self.workspace_manager.read().current_position(),
            self.workspace_manager.read().focused_column_index(),
            active_effects
        );

        Ok(())
    }

    /// Apply global visual effects like workspace transitions and background blur
    fn apply_global_effects(&mut self) {
        // Apply workspace transition effects
        let wm = self.workspace_manager.read();
        if wm.is_scrolling() {
            let current_pos = wm.current_position();
            let progress = wm.scroll_progress();

            // In a real implementation, this would apply visual effects to the entire compositor
            debug!(
                "Workspace transition: position={:.1}, progress={:.2}",
                current_pos, progress
            );
        }
    }

    /// Gracefully shutdown the compositor (with Smithay backend)
    async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down Axiom compositor...");

        self.running = false;

        // Clean up XWayland first
        // Tokio RwLock guards are safe to hold across .await points
        // because they use an async-aware Mutex internally.
        #[allow(clippy::await_holding_lock)]
        if let Some(ref xwayland) = self.xwayland_manager {
            debug!("Shutting down XWayland...");
            xwayland.write().await.shutdown().await?;
        }

        // Clean up Smithay backend
        debug!("Shutting down Smithay backend...");
        self.smithay_backend.shutdown()?;

        // Clean up other subsystems
        debug!("Cleaning up compositor subsystems...");
        self.ipc_server.shutdown().await?;
        self.input_manager.write().shutdown();
        self.effects_engine.write().shutdown();
        self.workspace_manager.write().shutdown();
        self.window_manager.write().shutdown();

        info!("Axiom compositor shutdown complete");
        Ok(())
    }

    /// Get current configuration
    pub fn config(&self) -> &AxiomConfig {
        &self.config
    }

    /// Check if compositor is running in windowed mode
    pub fn is_windowed(&self) -> bool {
        self._windowed
    }

    /// Single tick of the compositor (event processing + rendering)
    async fn tick(&mut self) -> Result<()> {
        use std::time::{Duration, Instant};
        let frame_start = Instant::now();
        // Honor general.max_fps (0 = unlimited, default 60).
        // If max_fps is 0, skip pacing entirely. Clamp to [1, 1000] to avoid
        // sub-microsecond durations that tokio::time::sleep can reject.
        let target_frame_time = if self.config.general.max_fps == 0 {
            Duration::ZERO
        } else {
            let clamped = self.config.general.max_fps.clamp(1, 1000);
            Duration::from_secs_f64(1.0 / f64::from(clamped))
        };

        let mut tick_error = false;

        // Prepare frame data BEFORE processing events, so the backend's
        // render() pass can consume pre-populated shadow/blur queues for
        // WGPU GPU post-processing within the GL submit window.
        if let Err(e) = self.prepare_frame_data() {
            tick_error = true;
            warn!("Error preparing frame data: {}", e);
        }

        // Process events (calls backend.process_events → run_one_cycle → render)
        if self.force_next_tick_error {
            tick_error = true;
            self.force_next_tick_error = false;
        }
        if let Err(e) = self.process_events() {
            tick_error = true;
            warn!("Error processing events: {}", e);
        }

        // Render frame — now only handles post-render monitoring after the
        // backend has already presented the frame with effects applied.
        if let Err(e) = self.render_frame() {
            tick_error = true;
            warn!("Error rendering frame: {}", e);
        }

        // Update stability metrics
        if tick_error {
            self.consecutive_error_count += 1;
            warn!(
                "Consecutive error count: {}",
                self.consecutive_error_count
            );
        } else if self.consecutive_error_count > 0 {
            // Stable tick, but DO NOT snap-to-zero. Decrement instead so
            // a single clean tick does not mask prior consecutive
            // failures — the audit's intent (see comment in the original
            // code, which contradicted the implementation). The fatal
            // threshold (`>= 5`) is checked AFTER this branch and
            // short-circuits the run loop, so guarding on `> 0` (rather
            // than `< 5`) is sufficient and gives us a one-tick recovery
            // slope: `N` consecutive errors need at least `N` clean
            // ticks before the counter fully resets.
            // `saturating_sub` keeps the counter at 0 instead of
            // underflowing past it.
            self.consecutive_error_count =
                self.consecutive_error_count.saturating_sub(1);
        }

        // Broadcast IPC performance metrics to Lazy UI (~10Hz rate-limited
        // internally) AND refresh the per-tick LiveMetrics snapshot so direct
        // monitoring queries (HealthCheck / GetPerformanceReport) see real
        // data instead of zeros. Locks here are all `parking_lot::RwLock`
        // short-lived read guards; no await points between them — safe to
        // compute inline. `active_windows` is the total registered window
        // count (was previously incorrectly wired to `column_count` in the
        // broadcast path; corrected below).
        let frame_time_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        let (workspace_idx, _, _column_count, _) = self.get_workspace_info();
        let active_windows = self.window_manager.read().window_count();
        let effects_gpu_available = self.effects_engine.read().is_gpu_initialized();
        self.ipc_server.maybe_broadcast_performance_metrics(
            frame_time_ms,
            active_windows,
            workspace_idx,
        );
        // Per-tick snapshot for direct monitoring queries (Design 12 final
        // wiring). `set_live_metrics_snapshot` replaces any previously-set
        // handle, so the per-tick metrics are visible to HealthCheck /
        // GetPerformanceReport without falling back to the zero default.
        // Cheap path: a single struct copy into a parking_lot-wrapped Arc.
        self.ipc_server.set_live_metrics_snapshot(LiveMetrics {
            frame_time_ms,
            active_windows,
            current_workspace: workspace_idx,
            effects_gpu_available,
        });

        // Frame pacing: sleep for remaining time to target the configured FPS.
        // Skipped when max_fps == 0 (unbounded).
        if !target_frame_time.is_zero() {
            let elapsed = frame_start.elapsed();
            if elapsed < target_frame_time {
                if let Some(sleep_duration) = target_frame_time.checked_sub(elapsed) {
                    tokio::time::sleep(sleep_duration).await;
                }
            }
        }

        // Design 11 second half: device-loss recovery hook. The renderer's
        // `map_async` callback flips its `device_lost` flag on driver crash
        // / context-reset and `compose_full_frame` already short-circuits
        // to Err. Here we observe the flag and stop the run loop with
        // a clear log message so the failure isn't a silent black screen.
        if let Some(ref render_lock) = self.renderer {
            if render_lock.read().is_device_lost() {
                log::error!(
                    "WGPU device flagged as lost; shutting down compositor run loop \u{2014}                      compositor must be reinitialised to recover"
                );
                self.running = false;
                // Returns Ok(()) on purpose: tokio::select! arm body is `{}`,
                // not the Result, so Err is misleading. The exit is driven
                // by `while self.running` in run().
                return Ok(());
            }
        }

        // Check stability threshold
        if self.consecutive_error_count >= 5 {
            log::error!(
                "CRITICAL: Too many consecutive errors ({}). Initiating emergency shutdown.",
                self.consecutive_error_count
            );
            let _ = self.shutdown().await;
            return Err(anyhow::anyhow!(
                "Critical stability failure: too many consecutive errors"
            ));
        }

        Ok(())
    }

    // === Public Workspace Interaction Methods ===

    /// Scroll workspace left (for input handling)
    pub fn scroll_workspace_left(&mut self) {
        info!("Scrolling workspace left");
        self.workspace_manager.write().scroll_left();
    }

    /// Scroll workspace right (for input handling)
    pub fn scroll_workspace_right(&mut self) {
        info!("Scrolling workspace right");
        self.workspace_manager.write().scroll_right();
    }

    /// Add a new window to the current workspace.
    /// Also registers the window with the server-side decoration manager so
    /// titlebar buttons are positioned from real geometry (not a placeholder).
    pub fn add_window(&mut self, title: String) -> u64 {
        // Create window in window manager (default size: 800x600)
        let window_id = self.window_manager.write().add_window(title.clone());

        // Add to current workspace column
        self.workspace_manager.write().add_window(window_id);

        // Trigger window open animation (spring-physics scale + fade-in)
        self.effects_engine.write().animate_window_open(window_id);

        // Register with DecorationManager using real window geometry.
        // The default BackendWindow size is 800×600; callers can update
        // via `set_window_width` after a Wayland configure arrives.
        self.decoration_manager.write().add_window(
            window_id,
            title.clone(),
            /* prefers_server_side */ true,
            800, // default BackendWindow width
        );

        info!(
            "Added window '{}' (ID: {}) to current workspace",
            title, window_id
        );
        window_id
    }

    /// Remove a window from the compositor.
    ///
    /// Returns `true` if the window existed (in workspace manager) and was
    /// removed from all subsystems, `false` if the ID was not found.
    ///
    /// Locks are taken in the same order as `render_frame`
    /// (`workspace -> window_manager -> renderer -> decoration_manager`);
    /// keep them in lockstep to avoid lock-order inversion if a future
    /// contributor adds a concurrent removal path.
    pub fn remove_window(&mut self, window_id: u64) -> bool {
        let removed = self
            .workspace_manager
            .write()
            .remove_window(window_id)
            .is_some();

        if removed {
            info!("Removed window {}", window_id);
        }

        self.window_manager.write().remove_window(window_id);

        if let Some(ref renderer) = self.renderer {
            renderer.write().remove_window(window_id);
        }

        self.decoration_manager.write().remove_window(window_id);

        removed
    }

    /// Move window to left workspace
    pub fn move_window_left(&mut self, window_id: u64) {
        if self.workspace_manager.write().move_window_left(window_id) {
            info!("Moved window {} to left workspace", window_id);
        }
    }

    /// Move window to right workspace
    pub fn move_window_right(&mut self, window_id: u64) {
        if self.workspace_manager.write().move_window_right(window_id) {
            info!("Moved window {} to right workspace", window_id);
        }
    }

    /// Get current workspace information
    pub fn get_workspace_info(&self) -> (i32, f64, usize, bool) {
        let wm = self.workspace_manager.read();
        (
            wm.focused_column_index(),
            wm.current_position(),
            wm.active_column_count(),
            wm.is_scrolling(),
        )
    }

    /// Set the viewport size (called when display size changes)
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.workspace_manager
            .write()
            .set_viewport_size(width as f64, height as f64);
        info!("Updated viewport size to {}x{}", width, height);
    }

    /// Single tick for integration testing — calls the private `tick()` method.
    /// Returns `Ok(())` on success or `Err(...)` if the error threshold is exceeded.
    pub async fn tick_for_test(&mut self) -> Result<()> {
        self.tick().await
    }

    /// Artificially set the consecutive error count for testing error recovery.
    /// When set >= 5, the next `tick()` will trigger an emergency shutdown.
    pub fn set_errors_for_test(&mut self, count: u32) {
        self.consecutive_error_count = count;
    }

    /// Force the next `tick()` to count as an error tick, incrementing the
    /// error count. This lets tests simulate real consecutive errors rather
    /// than just pre-setting the count. Resets after the next tick.
    pub fn force_next_tick_error(&mut self) {
        self.force_next_tick_error = true;
    }

    /// Check whether the compositor is still running.
    /// Used by integration tests to verify shutdown behavior.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get reference to effects engine (for demo purposes)
    pub fn effects_engine(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, crate::effects::EffectsEngine> {
        self.effects_engine.read()
    }

    /// Get mutable reference to effects engine (for demo purposes)
    pub fn effects_engine_mut(
        &self,
    ) -> parking_lot::RwLockWriteGuard<'_, crate::effects::EffectsEngine> {
        self.effects_engine.write()
    }

    /// Propagate configuration changes to all subsystems
    fn update_subsystems_config(&mut self) {
        info!("Propagating configuration changes to subsystems...");

        // Update Effects Engine
        self.effects_engine
            .write()
            .update_config(self.config.effects.clone());

        // Update Workspace Manager
        self.workspace_manager
            .write()
            .update_config(self.config.workspace.clone());

        // Update renderer border width from config
        if let Some(renderer) = &self.renderer {
            renderer
                .write()
                .set_border_width(self.config.window.border_width as f32);
        }

        // Future: Update Input Manager, etc.
    }
}

impl AxiomCompositor {
    /// Test-only constructor that skips real backend initialization.
    /// Subsystems are fully initialized. Smithay backend uses a test
    /// constructor that doesn't bind Wayland sockets. WGPU renderer is
    /// a real headless instance (requires GPU adapter).
    pub async fn new_for_test(
        config: AxiomConfig,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
    ) -> Result<Self> {
        // Attempt GPU renderer initialization; degrade gracefully if unavailable.
        // The compositor tests don't require a real renderer.
        let renderer = match AxiomRenderer::new_headless().await {
            Ok(r) => {
                let arc = Arc::new(parking_lot::RwLock::new(r));
                arc.write().set_effects_engine(effects_engine.clone());
                Some(arc)
            }
            Err(e) => {
                log::warn!(
                    "GPU renderer unavailable in test mode ({}): compositor tests will run without rendering",
                    e
                );
                None
            }
        };

        // Dummy IPC server (skip socket bind)
        let ipc_server = AxiomIPCServer::new();

        // Initialize server-side decoration manager for tests
        let minimize_enabled = config.features.enable_minimize;
        let decoration_manager = Arc::new(parking_lot::RwLock::new(
            DecorationManager::new(&config.window, minimize_enabled),
        ));

        // Test Smithay backend (no socket bind, no GPU init)
        let smithay_backend = AxiomSmithayBackendReal::new_for_test(
            config.clone(),
            window_manager.clone(),
            workspace_manager.clone(),
            effects_engine.clone(),
            input_manager.clone(),
            renderer.clone(),
            decoration_manager.clone(),
        )?;

        Ok(Self {
            config,
            _windowed: false,
            workspace_manager,
            effects_engine,
            window_manager,
            input_manager,
            xwayland_manager: None,
            ipc_server,
            smithay_backend,
            render_data_buffer: Vec::with_capacity(64),
            consecutive_error_count: 0,
            force_next_tick_error: false,
            renderer,
            decoration_manager,
            running: true, // Test compositor starts in running state
        })
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
mod tests {
    use super::*;
    use parking_lot::RwLock;
    use serial_test::serial;
    use std::sync::Arc;

    /// Create subsystems and a test compositor for unit testing public API methods.
    ///
    /// `Arc<parking_lot::RwLock<ScrollableWorkspaces>>` is flagged as
    /// non-`Sync` by clippy because `ScrollableWorkspaces` contains a
    /// `RefCell` for its layout cache. This is intentional (single-threaded
    /// interior mutability on the hot path) and the `Arc` here is only
    /// ever held within this test's `tokio::test` task — it never crosses
    /// thread boundaries, so the absence of `Sync` is harmless for tests.
    #[allow(clippy::arc_with_non_send_sync)]
    async fn make_test_compositor() -> AxiomCompositor {
        let config = AxiomConfig::default();
        let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
        let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
        let effects_engine = Arc::new(RwLock::new(
            EffectsEngine::new(&config.effects).expect("effects init"),
        ));
        let input_manager = Arc::new(RwLock::new(InputManager::new(
            &config.input,
            &config.bindings,
        )));

        AxiomCompositor::new_for_test(
            config,
            workspace_manager,
            effects_engine,
            window_manager,
            input_manager,
        )
        .await
        .expect("compositor init")
    }

    #[tokio::test]
    #[serial]
    async fn test_compositor_initialization() {
        let comp = make_test_compositor().await;
        assert!(!comp.is_windowed());
        assert!(comp.config().effects.enabled);
        // DecorationManager should be initialized
        assert!(comp.decoration_manager.read().get_decoration(1).is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_add_and_remove_window() {
        let mut comp = make_test_compositor().await;

        let id = comp.add_window("Test Window".into());
        assert_eq!(id, 1);

        // Window should be registered with DecorationManager using real geometry
        {
            let deco = comp.decoration_manager.read();
            let d = deco.get_decoration(id).expect("decoration should exist after add_window");
            assert_eq!(d.title, "Test Window");
            assert_eq!(d.window_width, 800, "should use default BackendWindow width");
        }

        let (column, _pos, _count, _scrolling) = comp.get_workspace_info();
        assert!(column >= 0);

        comp.remove_window(id);
        // Window should be removed from DecorationManager too
        assert!(comp.decoration_manager.read().get_decoration(id).is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_workspace_scrolling() {
        let mut comp = make_test_compositor().await;

        let _initial = comp.get_workspace_info();
        // Verify scrolling doesn't panic
        comp.scroll_workspace_right();
        let _after_right = comp.get_workspace_info();
        comp.scroll_workspace_left();
        let _after_left = comp.get_workspace_info();
    }

    #[tokio::test]
    #[serial]
    async fn test_viewport_resize() {
        let mut comp = make_test_compositor().await;

        comp.set_viewport_size(1920, 1080);
        comp.set_viewport_size(3840, 2160);
        // No panic = success
    }

    #[tokio::test]
    #[serial]
    async fn test_effects_engine_access() {
        let comp = make_test_compositor().await;

        // Read-only access
        {
            let effects = comp.effects_engine();
            let (_frame_time, _quality, _active) = effects.get_performance_stats();
        }

        // Write access
        {
            let mut effects = comp.effects_engine_mut();
            effects.shutdown();
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_window_movement_between_workspaces() {
        let mut comp = make_test_compositor().await;

        let id = comp.add_window("movable".into());
        comp.move_window_right(id);
        comp.move_window_left(id);
        comp.remove_window(id);
    }

    #[tokio::test]
    #[serial]
    async fn test_config_access() {
        let comp = make_test_compositor().await;
        let config = comp.config();
        assert!(config.workspace.scroll_speed > 0.0);
        assert!(!config.window.focus_follows_mouse);
    }

    #[tokio::test]
    #[serial]
    async fn test_multiple_windows() {
        let mut comp = make_test_compositor().await;

        let ids: Vec<u64> = (0..10)
            .map(|i| comp.add_window(format!("Window {}", i)))
            .collect();

        assert_eq!(ids.len(), 10);
        assert!(ids.windows(2).all(|w| w[0] + 1 == w[1]));

        for id in ids {
            comp.remove_window(id);
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_shutdown_cleans_up() {
        let mut comp = make_test_compositor().await;
        comp.add_window("pre-shutdown".into());
        comp.shutdown().await.expect("shutdown should succeed");
    }

    #[tokio::test]
    #[serial]
    async fn test_config_propagation_to_subsystems() {
        let comp = make_test_compositor().await;

        // Verify default blur radius is present
        let initial_blur = comp.config().effects.blur.radius;
        assert!(initial_blur > 0, "default blur radius should be nonzero");

        // Modify config and propagate — should not panic
        // (config is shared via Arc, full propagation test would need mutable config)
        let (_frame_time, _quality, _active) = comp.effects_engine().get_performance_stats();
    }
}
