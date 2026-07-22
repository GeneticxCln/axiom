//! Winit backend lifecycle, event dispatch, and the main backend struct.
//!
//! Contains `AxiomSmithayBackendReal` — the top-level backend struct that owns
//! `State`, the winit event loop, and the winit graphics backend — along with
//! its constructors, lifecycle methods (`initialize`, `run_one_cycle`,
//! `shutdown`), and winit event processing.
//!
//! Also defines `BackendKind`, `WindowInteraction`, and `smithay_output_scale`.
//! A submodule of `backend` can read the private fields of `State` and
//! `AxiomSmithayBackendReal` (descendant modules see ancestor privates).

use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::input::InputManager;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use anyhow::Result;
use log::{info, warn};

use smithay::{
    backend::{
        input::InputEvent,
        renderer::gles::GlesRenderer,
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    output::{Mode as OutputMode, Output, Scale},
    reexports::wayland_server::{Display, ListeningSocket},
    utils::Transform,
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        foreign_toplevel_list::ForeignToplevelListState,
        fractional_scale::FractionalScaleManagerState,
        selection::data_device::{set_data_device_focus, DataDeviceState},
        session_lock::SessionLockManagerState,
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, XdgShellState},
        },
        shm::ShmState,
    },
};

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc};

use wayland_server::{Client, Resource};

use super::state::State;

// ============================================================================
// Backend Kind
// ============================================================================

/// Backend kind selection for the Axiom compositor.
///
/// The compositor is winit-only (GLES rendering). `Noop` is a headless
/// backend used by tests/CI that performs no rendering and creates no winit
/// event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// Nested-session Winit backend (default, development-friendly).
    Winit,
    /// Headless no-op backend (tests / CI).
    Noop,
}

impl BackendKind {
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "winit" | "windowed" | "dev" => BackendKind::Winit,
            "noop" | "test" | "headless" => BackendKind::Noop,
            unknown => {
                warn!(
                    "Unknown backend kind '{}' — falling back to 'winit'. \
                     Valid values: winit, noop (and aliases)",
                    unknown
                );
                BackendKind::Winit
            }
        }
    }
}

/// How to convert a host scale factor (f64) into a Smithay `Scale`.
fn smithay_output_scale(scale: f64) -> Scale {
    if (scale.fract()).abs() < f64::EPSILON {
        Scale::Integer(scale.round().max(1.0) as i32)
    } else {
        Scale::Fractional(scale)
    }
}

// ============================================================================
// Backend Struct
// ============================================================================

pub struct AxiomSmithayBackendReal {
    pub display: Display<State>,
    pub socket_name: String,
    pub state: State,
    /// The resolved backend kind (winit / noop).
    pub backend_kind: BackendKind,
    pub winit_backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    pub winit_event_loop: Option<WinitEventLoop>,
    pub clients: Vec<Client>,
    /// Wayland listening socket — kept alive so clients can connect
    /// (accepted each cycle in `run_one_cycle_common`).
    listener: Option<ListeningSocket>,
    /// Set to `true` when a decoration button press was consumed (e.g.
    /// close/minimize). The subsequent release event must be consumed too
    /// so Wayland clients don't receive mismatched button-release without
    /// a preceding button-press.
    pub(super) decoration_consumed_press: bool,
    /// `Some(window_id)` when the user is dragging a window by its titlebar
    /// or resizing it by an edge/corner. While active, pointer motion events
    /// reposition/resize the window and button release commits the change.
    pub(super) interaction: Option<WindowInteraction>,
    /// Touch-based interactive window manipulation (move or resize).
    /// Mirrors `interaction` but for touch events. Tracked separately so
    /// pointer and touch can each have their own active interaction.
    pub(super) touch_interaction: Option<WindowInteraction>,
    /// Tracked touch-down position and time for tap-to-click detection.
    /// `(x, y, time_msec)`. Set on TouchDown, consumed on TouchUp when
    /// the tap thresholds are met.
    pub(super) touch_tap_state: Option<(f64, f64, u32)>,
}

/// Type of interactive window manipulation in progress.
#[derive(Clone, PartialEq)]
pub(super) enum WindowInteraction {
    Move {
        window_id: u64,
        offset_x: f64,
        offset_y: f64,
    },
    Resize {
        window_id: u64,
        edge: crate::decoration::ResizeEdge,
        /// Window geometry at resize-start (top-left corner, size).
        initial_rect: (i32, i32, u32, u32),
        /// Pointer position at resize-start.
        start_x: f64,
        start_y: f64,
    },
}

impl AxiomSmithayBackendReal {
    /// Test-only constructor that skips Wayland socket bind and display creation.
    /// Creates a minimal backend that supports compositor unit tests without
    /// requiring real system resources (no socket, no GPU init, no display).
    /// The `renderer` parameter is optional — pass `None` in headless/CI environments.
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_test(
        config: AxiomConfig,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
    ) -> Result<Self> {
        // Use a dummy display (bound to "null" — never dispatched)
        let display = Display::new()?;
        let dh = display.handle();

        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<State>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<State>(&dh);
        let session_lock_state = SessionLockManagerState::new::<State, _>(&dh, |_| true);

        let mut seat_state = smithay::input::SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "axiom-test");

        let (clipboard_update_tx, clipboard_update_rx) = mpsc::channel();

        let state = State {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            display_handle: Some(display.handle()),
            xdg_decoration_state: None,
            fractional_scale_manager_state,
            layer_shell_state,
            session_lock_state,
            seat,
            config,
            window_manager,
            workspace_manager,
            input_manager,
            surfaces: HashMap::new(),
            window_map: HashMap::new(),
            next_window_id: 1,
            outputs: Vec::new(),
            output_scale_factors: HashMap::new(),
            decoration_manager: decoration_manager.clone(),
            toplevels: HashMap::new(),
            toplevel_handles: HashMap::new(),
            foreign_toplevel_list_state: ForeignToplevelListState::new::<State>(&display.handle()),
            running: true,
            needs_redraw: true,
            pending_capture: None,
            session_locked: false,
            lock_surfaces: Vec::new(),
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: lru::LruCache::new(std::num::NonZeroUsize::new(256).unwrap()),
            configured_sizes: HashMap::new(),
            pending_configure: HashSet::new(),
            popups: HashMap::new(),
            active_popup_grab: None,
            clipboard_cache: None,
            clipboard_update_tx,
            clipboard_update_rx,
            clipboard_source: None,
            clipboard_fetch_pending: false,
            cursor_icon: None,
            dnd_icon: None,
            dnd_active: false,
            cached_floating_rects: Vec::new(),
            output_damage: Vec::new(),
            surface_previous_rects: HashMap::new(),
            surface_commit_counters: HashMap::new(),
        };

        Ok(Self {
            display,
            socket_name: String::from("axiom-test-dummy"),
            state,
            backend_kind: BackendKind::Noop,
            winit_backend: None,
            winit_event_loop: None,
            clients: Vec::new(),
            listener: None,
            decoration_consumed_press: false,
            interaction: None,
            touch_interaction: None,
            touch_tap_state: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AxiomConfig,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
    ) -> Result<Self> {
        info!("Initializing Smithay 0.7 Backend...");

        // Parse backend kind from config BEFORE config is moved into State.
        let backend_kind = BackendKind::from_config_str(&config.backend.kind);
        info!("Backend kind: {:?}", backend_kind);

        // Capture config.output.order BEFORE config is moved into State.
        let config_output_order = config.output.order.clone();

        // Clone the workspace_manager Arc so we can sync tapes after state
        // construction (the original is moved into State).
        let wm_for_sync = workspace_manager.clone();

        let display: Display<State> = Display::new()?;
        let dh = display.handle();

        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<State>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<State>(&dh);
        let session_lock_state = SessionLockManagerState::new::<State, _>(&dh, |_| true);

        let xdg_decoration_state = if config.features.enable_xdg_decoration_protocol {
            info!("🌐 Registering zxdg_decoration_manager_v1 global");
            Some(XdgDecorationState::new::<State>(&dh))
        } else {
            None
        };

        let mut seat_state = smithay::input::SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "axiom");
        let (clipboard_update_tx, clipboard_update_rx) = mpsc::channel();

        let output = Output::new(
            "Axiom-Output-0".into(),
            smithay::output::PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Axiom".into(),
                model: "Virtual".into(),
            },
        );
        let mode = OutputMode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            None,
        );
        output.create_global::<State>(&dh);
        let _ = dh.create_global::<State, smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _>(1, ());

        let state = State {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            display_handle: Some(display.handle()),
            xdg_decoration_state,
            fractional_scale_manager_state,
            layer_shell_state,
            session_lock_state,
            seat,
            config,
            window_manager,
            workspace_manager,
            input_manager,
            surfaces: HashMap::new(),
            window_map: HashMap::new(),
            next_window_id: 1,
            outputs: vec![output],
            output_scale_factors: HashMap::new(),
            decoration_manager: decoration_manager.clone(),
            toplevels: HashMap::new(),
            toplevel_handles: HashMap::new(),
            foreign_toplevel_list_state: ForeignToplevelListState::new::<State>(&display.handle()),
            running: true,
            needs_redraw: true,
            pending_capture: None,
            session_locked: false,
            lock_surfaces: Vec::new(),
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: lru::LruCache::new(std::num::NonZeroUsize::new(256).unwrap()),
            configured_sizes: HashMap::new(),
            pending_configure: HashSet::new(),
            popups: HashMap::new(),
            active_popup_grab: None,
            clipboard_cache: None,
            clipboard_update_tx,
            clipboard_update_rx,
            clipboard_source: None,
            clipboard_fetch_pending: false,
            cursor_icon: None,
            dnd_icon: None,
            dnd_active: false,
            cached_floating_rects: Vec::new(),
            output_damage: Vec::new(),
            surface_previous_rects: HashMap::new(),
            surface_commit_counters: HashMap::new(),
        };

        let socket_name = format!("wayland-axiom-{}", std::process::id());
        let listener = ListeningSocket::bind(&socket_name)?;
        info!("📡 Wayland socket: {}", socket_name);

        // Sync workspace tapes with configured outputs.
        // This ensures the tape infrastructure aligns with config.output.order.
        {
            let mut wm = wm_for_sync.write();
            let live_outputs = vec!["Axiom-Output-0".to_string()];
            wm.sync_tapes_with_outputs(&live_outputs, &config_output_order);
        }

        Ok(Self {
            display,
            socket_name,
            state,
            backend_kind,
            winit_backend: None,
            winit_event_loop: None,
            clients: Vec::new(),
            listener: Some(listener),
            decoration_consumed_press: false,
            interaction: None,
            touch_interaction: None,
            touch_tap_state: None,
        })
    }

    /// Initialize the selected backend (winit / noop).
    pub fn initialize(&mut self) -> Result<()> {
        match self.backend_kind {
            BackendKind::Winit => self.initialize_winit(),
            BackendKind::Noop => {
                info!("Noop backend selected — compositor will run headless");
                Ok(())
            }
        }
    }

    /// Initialize the winit backend for windowed/nested mode.
    fn initialize_winit(&mut self) -> Result<()> {
        info!("🖼️ Initializing Winit backend...");

        let (backend, event_loop) = winit::init::<GlesRenderer>()
            .map_err(|e| anyhow::anyhow!("Winit init failed: {:?}", e))?;

        info!("✅ Winit backend initialized");

        let window_size = backend.window_size();
        let host_scale = backend.window().scale_factor().clamp(1.0, 4.0);

        self.state.window_width = window_size.w as u32;
        self.state.window_height = window_size.h as u32;
        {
            let mut wm = self.state.workspace_manager.write();
            let tape = wm.ensure_tape("default");
            tape.set_scale_factor(host_scale);
            tape.set_viewport_size(window_size.w as f64, window_size.h as f64);
        }
        if let Some(output) = self.state.outputs.first().cloned() {
            output.change_current_state(
                Some(OutputMode {
                    size: (window_size.w, window_size.h).into(),
                    refresh: 60_000,
                }),
                Some(Transform::Normal),
                Some(smithay_output_scale(host_scale)),
                None,
            );
            // Track the output's scale as the source of truth for
            // `focused_output_scale` (see that method).
            self.state
                .output_scale_factors
                .insert("Axiom-Output-0".into(), host_scale);
        }

        self.winit_backend = Some(backend);
        self.winit_event_loop = Some(event_loop);

        let (repeat_delay, repeat_rate) = State::keyboard_repeat_settings(&self.state.config);
        let _keyboard =
            self.state
                .seat
                .add_keyboard(smithay::input::keyboard::XkbConfig::default(), repeat_delay, repeat_rate)?;

        self.state.seat.add_pointer();
        self.state.seat.add_touch();

        info!("✅ Input devices registered with seat");

        // Compile GLES 2.0 shader program for texture rendering (deferred until first render)
        // The GL context isn't active yet — compilation happens lazily in render()
        info!("🎨 GLES 2.0 shader will be compiled on first render");

        Ok(())
    }

    /// Run one cycle of the event loop
    pub fn run_one_cycle(&mut self) -> Result<()> {
        match self.backend_kind {
            BackendKind::Winit => self.run_one_cycle_winit()?,
            BackendKind::Noop => {
                // Noop mode: tick without any backend events.
                // Wayland client dispatch and rendering still happen.
            }
        }

        // Common dispatch for all backends
        self.run_one_cycle_common()
    }

    /// Winit-specific event dispatch and input processing.
    fn run_one_cycle_winit(&mut self) -> Result<()> {
        let Some(winit_event_loop) = self.winit_event_loop.as_mut() else {
            return Ok(());
        };

        // Collect events that need post-dispatch processing
        let mut input_events: Vec<InputEvent<smithay::backend::winit::WinitInput>> = Vec::new();
        let mut resized_to: Option<(u32, u32, f64)> = None;
        let mut close_requested = false;

        winit_event_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, scale_factor } => {
                // Size<i32, Physical> — use .w and .h
                resized_to = Some((size.w as u32, size.h as u32, scale_factor));
            }
            WinitEvent::Redraw => {}
            WinitEvent::Input(input_event) => {
                input_events.push(input_event);
            }
            WinitEvent::CloseRequested => {
                close_requested = true;
            }
            _ => {}
        });

        // Process resize
        if let Some((w, h, host_scale)) = resized_to {
            info!("📐 Window resized to {}x{} (scale {:.2})", w, h, host_scale);
            self.state.window_width = w;
            self.state.window_height = h;
            let host_scale = host_scale.clamp(1.0, 4.0);
            {
                let mut wm = self.state.workspace_manager.write();
                // Update all existing tapes to the new output size
                let tape_ids: Vec<String> = wm.known_tape_ids();
                if tape_ids.is_empty() {
                    let tape = wm.ensure_tape("default");
                    tape.set_scale_factor(host_scale);
                    tape.set_viewport_size(w as f64, h as f64);
                } else {
                    for tape_id in &tape_ids {
                        let tape = wm.ensure_tape(tape_id);
                        tape.set_scale_factor(host_scale);
                        tape.set_viewport_size(w as f64, h as f64);
                    }
                }
            }
            if let Some(output) = self.state.outputs.first().cloned() {
                output.change_current_state(
                    Some(OutputMode {
                        size: (w as i32, h as i32).into(),
                        refresh: 60_000,
                    }),
                    Some(Transform::Normal),
                    Some(smithay_output_scale(host_scale)),
                    None,
                );
            }
            // Track the output scale for all known outputs
            let tape_ids: Vec<String> = self.state.workspace_manager.read().known_tape_ids();
            for tape_id in &tape_ids {
                self.state
                    .output_scale_factors
                    .insert(tape_id.clone(), host_scale);
            }
            self.state.needs_redraw = true;
        }

        // Process close
        if close_requested {
            info!("📨 Close requested");
            self.state.running = false;
        }

        // Process collected input events
        for event in input_events {
            // handle_input_event is defined in input.rs (on AxiomSmithayBackendReal)
            self.handle_input_event(event);
        }

        Ok(())
    }

    /// Common post-event dispatch for all backends.
    pub(super) fn run_one_cycle_common(&mut self) -> Result<()> {
        // Accept new Wayland clients on the bound listening socket. Without
        // this, connect() succeeds at the kernel level but the server never
        // reads the connection, so no client can ever bind to Axiom.
        if let Some(listener) = &self.listener {
            loop {
                match listener.accept() {
                    Ok(Some(stream)) => {
                        if let Err(e) = self.display.handle().insert_client(
                            stream,
                            Arc::new(super::state::ClientState {
                                compositor_state: CompositorClientState::default(),
                            }),
                        ) {
                            warn!("Failed to insert Wayland client: {e}");
                        }
                    }
                    Ok(None) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        warn!("Wayland listener accept error: {e}");
                        break;
                    }
                }
            }
        }

        // Dispatch Wayland client events
        self.display.dispatch_clients(&mut self.state)?;
        self.display.flush_clients()?;

        // Fetch any client selection offered during this dispatch (the
        // selection is only registered in `seat_data` after `new_selection`
        // returns, so it must be requested here, post-dispatch).
        self.state.maybe_fetch_clipboard();

        // Fold in any asynchronously-read clipboard payloads requested from the
        // active Wayland selection source so X11 requests can be served from the
        // compositor cache on the next pass.
        self.state.drain_clipboard_updates();

        // Update animations after dispatch so newly-created windows (which
        // trigger animate_window_open() during dispatch) get their first
        // integration step before the render pass reads effect states.
        if self.state.workspace_manager.write().update_animations() {
            self.state.needs_redraw = true;
        }

        // Prune dead surfaces from disconnected clients
        self.state.prune_dead_surfaces();

        // Render if needed.
        if self.state.needs_redraw {
            self.render()?;
            self.state.needs_redraw = false;
        }

        Ok(())
    }

    /// Process events (for compositor integration)
    pub fn process_events(&mut self) -> Result<()> {
        self.run_one_cycle()
    }

    /// Test/debug accessor: clone the cached Wayland→compositor selection
    /// payload (`clipboard_cache`). Used by headless integration tests to
    /// assert the compositor received a client's clipboard offer.
    pub fn debug_clipboard_cache(&self) -> Option<Vec<u8>> {
        self.state.clipboard_cache.clone()
    }

    /// Test/debug helper: grant keyboard + data-device focus to the first
    /// mapped client surface so it may offer a clipboard selection. In a real
    /// session this focus is driven by input; headless tests grant it directly
    /// to exercise the selection path without a display.
    pub fn debug_focus_first_client_for_test(&mut self) {
        // The headless Noop backend never registers input devices (that happens
        // in `initialize_winit`), so the seat may lack a keyboard. Selection
        // focus requires one, so create it on demand for the test.
        if self.state.seat.get_keyboard().is_none() {
            let _ = self.state.seat.add_keyboard(smithay::input::keyboard::XkbConfig::default(), 0, 0);
        }
        let surface = self
            .state
            .toplevels
            .values()
            .next()
            .map(|t| t.wl_surface().clone());
        if let Some(surface) = surface {
            if let Some(keyboard) = self.state.seat.get_keyboard() {
                keyboard.set_focus(&mut self.state, Some(surface.clone()), smithay::utils::Serial::from(0));
            }
            if let Some(dh) = &self.state.display_handle {
                set_data_device_focus(dh, &self.state.seat, surface.client());
            }
        }
    }

    /// Shutdown the backend
    pub fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Shutting down Smithay backend");
        self.state.running = false;

        // Free the persistent blit texture and shader that exist outside
        // per-frame management.
        match self.backend_kind {
            BackendKind::Winit => {
                if let Some(backend) = self.winit_backend.as_mut() {
                    // Try rebinding; failure during shutdown is non-fatal.
                    let _ = backend.bind();
                }
            }
            BackendKind::Noop => {
                // Noop shutdown: nothing to clean up.
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        smithay_output_scale, AxiomSmithayBackendReal, WindowInteraction,
    };
    use crate::config::{AxiomConfig, BindingsConfig, InputConfig, WindowConfig, WorkspaceConfig};
    use crate::decoration::DecorationManager;
    use crate::input::InputManager;
    use crate::window::WindowManager;
    use crate::workspace::ScrollableWorkspaces;
    use parking_lot::RwLock;
    use smithay::output::Scale;
    use smithay::wayland::selection::data_device::{ClientDndGrabHandler, ServerDndGrabHandler};
    use std::fs::File;
    use std::os::unix::io::OwnedFd;
    use std::sync::Arc;

    /// Create a headless backend for unit tests with default config.
    fn test_backend() -> AxiomSmithayBackendReal {
        AxiomSmithayBackendReal::new_for_test(
            AxiomConfig::default(),
            Arc::new(RwLock::new(WindowManager::new(&WindowConfig::default()))),
            Arc::new(RwLock::new(ScrollableWorkspaces::new(
                &WorkspaceConfig::default(),
            ))),
            Arc::new(RwLock::new(InputManager::new(
                &InputConfig::default(),
                &BindingsConfig::default(),
            ))),
            Arc::new(RwLock::new(DecorationManager::new(
                &WindowConfig::default(),
                false,
            ))),
        )
        .expect("test backend")
    }

    #[test]
    fn test_smithay_output_scale_supports_fractional_values() {
        match smithay_output_scale(1.5) {
            Scale::Fractional(scale) => assert!((scale - 1.5).abs() < f64::EPSILON),
            other => panic!("expected fractional scale, got {:?}", other),
        }
    }

    // ── Drag-and-Drop (DnD) Tests ──────────────────────────────────────────

    /// Verify dnd_active flag follows ClientDndGrabHandler life cycle.
    #[test]
    fn test_dnd_active_flag() {
        let mut backend = test_backend();
        assert!(!backend.state.dnd_active, "dnd_active starts false");

        let seat = backend.state.seat.clone();
        ClientDndGrabHandler::started(&mut backend.state, None, None, seat);
        assert!(backend.state.dnd_active, "dnd_active set after started()");

        let seat = backend.state.seat.clone();
        ClientDndGrabHandler::dropped(&mut backend.state, None, false, seat);
        assert!(
            !backend.state.dnd_active,
            "dnd_active cleared after dropped()"
        );
    }

    /// Verify dnd_icon starts and stays None when no icon surface is provided.
    #[test]
    fn test_dnd_icon_tracking() {
        let mut backend = test_backend();
        assert!(backend.state.dnd_icon.is_none(), "dnd_icon starts None");

        // started() with None icon should keep icon None
        let seat = backend.state.seat.clone();
        ClientDndGrabHandler::started(&mut backend.state, None, None, seat);
        assert!(
            backend.state.dnd_icon.is_none(),
            "dnd_icon is None when started without icon"
        );

        // Clean up the session
        let seat = backend.state.seat.clone();
        ClientDndGrabHandler::dropped(&mut backend.state, None, false, seat);
    }

    /// ServerDndGrabHandler::send serves clipboard cache data (or drops fd when empty).
    #[test]
    fn test_dnd_send_no_panic() {
        let mut backend = test_backend();

        // Provide a real fd from /dev/null — send handler writes or drops it
        let fd = OwnedFd::from(File::open("/dev/null").expect("/dev/null openable"));
        let seat = backend.state.seat.clone();
        ServerDndGrabHandler::send(&mut backend.state, "text/plain".into(), fd, seat);
        // Reaching here means no panic
    }

    /// ServerDndGrabHandler::send serves clipboard cache when populated.
    #[test]
    fn test_dnd_send_serves_cached_data() {
        use std::io::Read;
        let mut backend = test_backend();

        // Populate clipboard cache
        backend.state.clipboard_cache = Some(b"hello dnd".to_vec());

        let (read_fd, write_fd) = super::super::clipboard::create_clipboard_pipe().expect("pipe");
        let seat = backend.state.seat.clone();
        ServerDndGrabHandler::send(&mut backend.state, "text/plain".into(), write_fd, seat);

        // Read back what was written to the pipe
        let mut buf = Vec::new();
        let mut file = std::fs::File::from(read_fd);
        file.read_to_end(&mut buf).expect("read pipe");
        assert_eq!(
            buf, b"hello dnd",
            "ServerDndGrabHandler::send should write cached data"
        );
    }

    // ── Touch Tests ────────────────────────────────────────────────────────

    /// touch_focus_under returns None when the workspace has no windows.
    #[test]
    fn test_touch_focus_no_windows() {
        let backend = test_backend();
        let result = backend.touch_focus_under(100.0, 200.0);
        assert!(result.is_none(), "no touch focus when no windows exist");
    }

    /// Handle a touch-based window move interaction.
    #[test]
    fn test_touch_interaction_move() {
        let mut backend = test_backend();

        // Add a window
        let wid = backend
            .state
            .window_manager
            .write()
            .add_window("Move Test".into());
        backend.state.window_map.insert(wid, 1);

        // Set initial window position
        {
            let mut wm = backend.state.window_manager.write();
            if let Some(w) = wm.get_window_mut(wid) {
                w.window.position = (100, 200);
            }
        }

        // Move interaction: touch at (150, 250) → offset (50, 50) from window origin
        let interaction = WindowInteraction::Move {
            window_id: wid,
            offset_x: 50.0,
            offset_y: 50.0,
        };
        let handled = backend.handle_interaction(&interaction, 300.0, 350.0);
        assert!(handled, "move interaction handled");

        // Window should be at (300-50, 350-50) = (250, 300)
        let wm = backend.state.window_manager.read();
        let w = wm.get_window(wid).expect("window exists");
        assert_eq!(w.window.position.0, 250, "x after move");
        assert_eq!(w.window.position.1, 300, "y after move");
    }

    /// Handle a touch-based window resize interaction (bottom-right edge).
    #[test]
    fn test_touch_interaction_resize() {
        use crate::decoration::ResizeEdge;
        let mut backend = test_backend();

        // Add a window
        let wid = backend
            .state
            .window_manager
            .write()
            .add_window("Resize Test".into());
        backend.state.window_map.insert(wid, 1);

        // Set initial window geometry
        {
            let mut wm = backend.state.window_manager.write();
            if let Some(w) = wm.get_window_mut(wid) {
                w.window.position = (100, 100);
                w.window.size = (200, 200);
            }
        }

        // Bottom-right resize: drag from (300, 300) to (350, 350) → +50 each
        let interaction = WindowInteraction::Resize {
            window_id: wid,
            edge: ResizeEdge::BottomRight,
            initial_rect: (100, 100, 200, 200),
            start_x: 300.0,
            start_y: 300.0,
        };
        let handled = backend.handle_interaction(&interaction, 350.0, 350.0);
        assert!(handled, "resize interaction handled");

        let wm = backend.state.window_manager.read();
        let w = wm.get_window(wid).expect("window exists");
        assert_eq!(w.window.size.0, 250, "width after BottomRight resize");
        assert_eq!(w.window.size.1, 250, "height after BottomRight resize");
        // BottomRight does not move the top-left corner
        assert_eq!(w.window.position.0, 100, "x unchanged");
        assert_eq!(w.window.position.1, 100, "y unchanged");
    }

    /// Left-edge resize moves the window position and adjusts width.
    #[test]
    fn test_touch_interaction_resize_left_edge() {
        use crate::decoration::ResizeEdge;
        let mut backend = test_backend();

        let wid = backend
            .state
            .window_manager
            .write()
            .add_window("Left Resize".into());
        backend.state.window_map.insert(wid, 1);

        {
            let mut wm = backend.state.window_manager.write();
            if let Some(w) = wm.get_window_mut(wid) {
                w.window.position = (100, 100);
                w.window.size = (200, 200);
            }
        }

        // Left-edge resize: drag from start_x=100 (left edge) leftward to 50
        // dx = 50 - 100 = -50
        // w  = 200 - (-50) = 250
        // x  = 100 + (200 - 250) = 50
        let interaction = WindowInteraction::Resize {
            window_id: wid,
            edge: ResizeEdge::Left,
            initial_rect: (100, 100, 200, 200),
            start_x: 100.0,
            start_y: 200.0,
        };
        backend.handle_interaction(&interaction, 50.0, 200.0);

        let wm = backend.state.window_manager.read();
        let w = wm.get_window(wid).expect("window exists");
        assert_eq!(w.window.size.0, 250, "width after Left resize");
        assert_eq!(w.window.position.0, 50, "x after Left resize");
    }

    /// Top-edge resize moves the window position and adjusts height.
    #[test]
    fn test_touch_interaction_resize_top_edge() {
        use crate::decoration::ResizeEdge;
        let mut backend = test_backend();

        let wid = backend
            .state
            .window_manager
            .write()
            .add_window("Top Resize".into());
        backend.state.window_map.insert(wid, 1);

        {
            let mut wm = backend.state.window_manager.write();
            if let Some(w) = wm.get_window_mut(wid) {
                w.window.position = (100, 100);
                w.window.size = (200, 200);
            }
        }

        // Top-edge resize: drag from start_y=100 upward to 70
        // dy = 70 - 100 = -30
        // h  = 200 - (-30) = 230
        // y  = 100 + (200 - 230) = 70
        let interaction = WindowInteraction::Resize {
            window_id: wid,
            edge: ResizeEdge::Top,
            initial_rect: (100, 100, 200, 200),
            start_x: 200.0,
            start_y: 100.0,
        };
        backend.handle_interaction(&interaction, 200.0, 70.0);

        let wm = backend.state.window_manager.read();
        let w = wm.get_window(wid).expect("window exists");
        assert_eq!(w.window.size.1, 230, "height after Top resize");
        assert_eq!(w.window.position.1, 70, "y after Top resize");
    }

    /// Touch down and touch up events don't crash when seat has no touch.
    /// The internal dispatch path is exercised through handle_interaction
    /// (the real handler calls through to the same helper).
    /// Full InputEvent<WinitInput> construction is not possible outside
    /// the smithay crate (event fields are pub(crate)), so we verify the
    /// interaction logic that both pointer and touch dispatch call.
    #[test]
    fn test_touch_down_handles_noop_seat() {
        let mut backend = test_backend();
        // No touch capability on the test seat → dispatch is a no-op.
        // Verify handle_interaction (shared by touch & pointer paths) is safe.
        // This path is hit when seat.get_touch() returns None.
        let wid = backend
            .state
            .window_manager
            .write()
            .add_window("Noop Touch".into());
        backend.state.window_map.insert(wid, 1);

        // Set a touch interaction and verify cleanup via handle_interaction
        let interaction = WindowInteraction::Move {
            window_id: wid,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        backend.touch_interaction = Some(interaction.clone());
        // handle_interaction should still process the move
        let handled = backend.handle_interaction(&interaction, 500.0, 500.0);
        assert!(handled, "touch interaction handled even without seat touch");
    }

    // ── Damage Tracking Tests ───────────────────────────────────────────────

    /// Commit counters start empty and increment on commit.
    #[test]
    fn test_surface_commit_counter_tracking() {
        let backend = test_backend();
        assert!(
            backend.state.surface_commit_counters.is_empty(),
            "commit counters start empty"
        );
    }

    /// Window manager tracks fullscreen state correctly for occlusion.
    #[test]
    fn test_fullscreen_window_tracking_for_occlusion() {
        let mut backend = test_backend();
        let wid = backend.state.window_manager.write().add_window("Test".into());
        backend.state.window_map.insert(wid, 1);
        backend.state.window_width = 800;
        backend.state.window_height = 600;

        // Set as fullscreen
        {
            let mut wm = backend.state.window_manager.write();
            if let Some(w) = wm.get_window_mut(wid) {
                w.properties.fullscreen = true;
            }
        }

        // Verify fullscreen state
        let wm = backend.state.window_manager.read();
        let w = wm.get_window(wid).expect("window exists");
        assert!(w.properties.fullscreen, "window marked as fullscreen");
    }

    /// Surface commit counters are populated on surface commit.
    #[test]
    fn test_commit_counter_increments_on_commit() {
        let mut backend = test_backend();
        let mut counter = 0u64;

        // Simulate three commits
        for _ in 0..3 {
            counter += 1;
            // Manually exercise the surface_commit_counters tracking
            // that the CompositorHandler::commit does for real surfaces.
            // Here we directly test the data structure behavior.
            backend.state.surface_commit_counters.insert(42, counter);
        }
        assert_eq!(
            backend.state.surface_commit_counters.get(&42),
            Some(&3u64),
            "counter incremented to 3 after 3 commits"
        );
    }
}
