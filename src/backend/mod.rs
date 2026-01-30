//! Smithay 0.7 Backend for Axiom Compositor
#![allow(missing_docs)]
//!
//! This module implements the Wayland compositor backend using Smithay 0.7's
//! handler trait pattern.

use crate::config::AxiomConfig;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::renderer::AxiomRenderer;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use anyhow::Result;
use log::{debug, info};

use std::collections::HashMap;
use std::os::unix::io::OwnedFd;
use std::sync::Arc;

use smithay::{
    backend::{
        input::{InputEvent, KeyboardKeyEvent, PointerButtonEvent},
        renderer::{gles::GlesRenderer, utils::on_commit_buffer_handler},
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
    input::{
        keyboard::{FilterResult, XkbConfig},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::{protocol::wl_seat, Display, ListeningSocket},
    utils::{Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            with_states, BufferAssignment, CompositorClientState, CompositorHandler,
            CompositorState, SurfaceAttributes,
        },
        output::OutputHandler,
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{with_buffer_contents, ShmHandler, ShmState},
    },
};

use wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{wl_buffer, wl_surface::WlSurface},
    Client, Resource,
};

use wayland_protocols::xdg::shell::server::xdg_toplevel;

pub mod xwm;
use self::xwm::AxiomXwm;

// ============================================================================
// Surface Data
// ============================================================================

/// Surface data for tracking Wayland surfaces
#[derive(Debug, Clone)]
pub struct SurfaceData {
    pub window_id: Option<u64>,
    pub title: String,
    pub app_id: Option<String>,
    pub size: (i32, i32),
    pub committed: bool,
    pub surface: Option<WlSurface>,
}

// ============================================================================
// Client State (per-client data)
// ============================================================================

struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

// ============================================================================
// Compositor State
// ============================================================================

/// Main compositor state holding all subsystems
pub struct State {
    // Smithay protocol states
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,

    // Seat
    pub seat: Seat<Self>,

    // Axiom subsystems
    pub config: AxiomConfig,
    pub window_manager: Arc<parking_lot::RwLock<WindowManager>>,
    pub workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
    pub effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
    pub input_manager: Arc<parking_lot::RwLock<InputManager>>,
    pub renderer: Arc<parking_lot::RwLock<AxiomRenderer>>,

    // Tracking
    pub surfaces: HashMap<u32, SurfaceData>,
    pub window_map: HashMap<u64, u32>,
    pub next_window_id: u64,

    // Outputs
    pub outputs: Vec<Output>,

    // XWayland (optional)
    pub xwm: Option<AxiomXwm>,

    // Running state
    pub running: bool,
    pub needs_redraw: bool,
}

impl State {
    /// Create a new Axiom window from a surface
    pub fn create_window_from_surface(
        &mut self,
        surface_id: u32,
        title: String,
        app_id: Option<String>,
        surface: WlSurface,
    ) -> u64 {
        info!(
            "🪟 Creating window from surface {} (title: {})",
            surface_id, title
        );

        // Use WindowManager as source of truth for Window IDs
        let window_id = self.window_manager.write().add_window(title.clone());
        self.workspace_manager.write().add_window(window_id);

        let surface_data = SurfaceData {
            window_id: Some(window_id),
            title,
            app_id,
            size: (640, 480),
            committed: false,
            surface: Some(surface),
        };
        self.surfaces.insert(surface_id, surface_data);
        self.window_map.insert(window_id, surface_id);

        window_id
    }

    pub fn destroy_window(&mut self, surface_id: u32) {
        if let Some(data) = self.surfaces.remove(&surface_id) {
            if let Some(window_id) = data.window_id {
                info!("🗑️ Destroying window {}", window_id);
                self.window_map.remove(&window_id);
                self.window_manager.write().remove_window(window_id);
                self.workspace_manager.write().remove_window(window_id);
            }
        }
    }
}

// ============================================================================
// Handler Trait Implementations
// ============================================================================

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.needs_redraw = true;

        // Try to update rendering texture if this surface belongs to a window
        let surface_id = surface.id().protocol_id();
        if let Some(window_id) =
            self.window_map
                .iter()
                .find_map(|(&wid, &sid)| if sid == surface_id { Some(wid) } else { None })
        {
            // Clone renderer handle to pass into closure
            let renderer = self.renderer.clone();

            with_states(surface, move |states| {
                let mut attrs = states.cached_state.get::<SurfaceAttributes>();
                // BufferAssignment might not be Clone, so access by ref
                let buffer = &attrs.current().buffer;

                if let Some(BufferAssignment::NewBuffer(wl_buffer)) = buffer {
                    // Access SHM data
                    let _ = with_buffer_contents(wl_buffer, |ptr, len, spec| {
                        let width = spec.width as u32;
                        let height = spec.height as u32;
                        let stride = spec.stride as usize;

                        // Simple check for 4-byte format
                        if stride as u32 >= width * 4 && len >= (height as usize * stride) {
                            let data = unsafe { std::slice::from_raw_parts(ptr, len) };
                            renderer
                                .write()
                                .update_window_texture(window_id, width, height, data);
                        }
                    });
                }
            });
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Activate the surface
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });
        surface.send_configure();

        // Create window in Axiom
        let wl_surface = surface.wl_surface().clone();
        let surface_id = wl_surface.id().protocol_id();
        // Note: title and app_id are on the toplevel, not pending state in 0.7
        let title = String::from("Untitled"); // TODO: Get from toplevel data map
        let app_id = None; // TODO: Get from toplevel data map

        self.create_window_from_surface(surface_id, title, app_id, wl_surface);
        self.needs_redraw = true;
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // TODO: Track popups
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl OutputHandler for State {}

// Delegate macros
delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_xdg_shell!(State);
delegate_data_device!(State);
smithay::delegate_output!(State);

// ============================================================================
// Backend Struct
// ============================================================================

pub struct AxiomSmithayBackendReal {
    pub display: Display<State>,
    pub socket_name: String,
    pub state: State,
    pub winit_backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    pub winit_event_loop: Option<WinitEventLoop>,
    pub clients: Vec<Client>,
}

impl AxiomSmithayBackendReal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AxiomConfig,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        renderer: Arc<parking_lot::RwLock<AxiomRenderer>>,
    ) -> Result<Self> {
        info!("🚀 Initializing Smithay 0.7 Backend...");

        let display: Display<State> = Display::new()?;
        let dh = display.handle();

        // Initialize Smithay globals
        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "axiom");

        // Create output
        let output = Output::new(
            "Axiom-Output-0".into(),
            PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: Subpixel::Unknown,
                make: "Axiom".into(),
                model: "Virtual".into(),
            },
        );
        let mode = OutputMode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), Some(Transform::Normal), None, None);
        output.create_global::<State>(&dh);

        let state = State {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            seat,
            config,
            window_manager,
            workspace_manager,
            effects_engine,
            input_manager,
            renderer,
            surfaces: HashMap::new(),
            window_map: HashMap::new(),
            next_window_id: 1,
            outputs: vec![output],
            xwm: None,
            running: true,
            needs_redraw: true,
        };

        // Create listening socket
        let socket_name = format!("wayland-axiom-{}", std::process::id());
        let _listener = ListeningSocket::bind(&socket_name)?;
        info!("📡 Wayland socket: {}", socket_name);

        // Accept clients from the socket (we won't use calloop for now, manual poll)
        // TODO: Integrate with calloop for proper event loop

        Ok(Self {
            display,
            socket_name,
            state,
            winit_backend: None,
            winit_event_loop: None,
            clients: Vec::new(),
        })
    }

    /// Initialize winit backend for windowed mode
    pub fn initialize(&mut self) -> Result<()> {
        info!("🖼️ Initializing Winit backend...");

        let (backend, event_loop) = winit::init::<GlesRenderer>()
            .map_err(|e| anyhow::anyhow!("Winit init failed: {:?}", e))?;

        info!("✅ Winit backend initialized");

        self.winit_backend = Some(backend);
        self.winit_event_loop = Some(event_loop);

        // Add keyboard to seat
        let _keyboard = self
            .state
            .seat
            .add_keyboard(XkbConfig::default(), 200, 200)?;

        Ok(())
    }

    /// Run one cycle of the event loop
    pub fn run_one_cycle(&mut self) -> Result<()> {
        let Some(winit_event_loop) = self.winit_event_loop.as_mut() else {
            return Ok(());
        };

        let mut running = true;
        let mut needs_redraw = false;

        winit_event_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { .. } | WinitEvent::Redraw => {
                needs_redraw = true;
            }
            WinitEvent::Input(_input_event) => {
                needs_redraw = true;
            }
            WinitEvent::Focus(_) => {}
            WinitEvent::CloseRequested => {
                running = false;
            }
        });

        self.state.running = running;
        self.state.needs_redraw |= needs_redraw;

        // Dispatch Wayland events
        self.display.dispatch_clients(&mut self.state)?;
        self.display.flush_clients()?;

        // Render if needed
        if self.state.needs_redraw {
            self.render()?;
            self.state.needs_redraw = false;
        }

        Ok(())
    }

    fn handle_input(&mut self, event: InputEvent<winit::WinitInput>) {
        use smithay::backend::input::Event;

        match event {
            InputEvent::Keyboard { event } => {
                if let Some(keyboard) = self.state.seat.get_keyboard() {
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = Event::time_msec(&event);
                    keyboard.input::<(), _>(
                        &mut self.state,
                        event.key_code(),
                        event.state(),
                        serial,
                        time,
                        |_, _modifiers, _handle| {
                            // TODO: Map keysyms to strings for InputManager
                            // For now, we forward everything to clients
                            FilterResult::Forward
                        },
                    );
                }
            }

            InputEvent::PointerMotionAbsolute { event } => {
                use smithay::backend::input::AbsolutePositionEvent;
                let (x, y) = (event.x(), event.y());
                debug!("Pointer motion: ({}, {})", x, y);
            }
            InputEvent::PointerButton { event } => {
                let _serial = SERIAL_COUNTER.next_serial();
                debug!("Pointer button: {:?}", event.state());
            }
            InputEvent::PointerAxis { event: _ } => {
                // FIXME: Axis type mismatch in Smithay 0.7. Disabling scroll mapping for now.
                /*
                let source = event.source();
                let horizontal_amount = event
                    .amount(wl_pointer::Axis::HorizontalScroll)
                    .unwrap_or(0.0);
                let vertical_amount = event
                    .amount(wl_pointer::Axis::VerticalScroll)
                    .unwrap_or(0.0);

                // Map to Axiom Scroll event
                let axiom_event = crate::input::InputEvent::Scroll {
                    x: 0.0, // TODO: Get current pointer pos
                    y: 0.0,
                    delta_x: horizontal_amount,
                    delta_y: vertical_amount,
                };

                // Process via InputManager
                let actions = self.state.input_manager.write().process_input_event(axiom_event);
                self.process_actions(actions);
                */
                debug!("Pointer axis event (input mapping disabled pending Smithay 0.7 fix)");

                // Forward to client as well
                if let Some(_ptr) = self.state.seat.get_pointer() {
                    // Forwarding logic would go here
                    // For now just logging
                    // debug!("Pointer axis: h={}, v={}, source={:?}", horizontal_amount, vertical_amount, source);
                    debug!("Pointer axis event (forwarding disabled)");
                }
            }
            _ => {}
        }
    }

    /// Process actions generated by `InputManager`
    fn process_actions(&mut self, actions: Vec<crate::input::CompositorAction>) {
        use crate::input::CompositorAction;
        for action in actions {
            match action {
                CompositorAction::ScrollWorkspaceLeft => {
                    self.state.workspace_manager.write().scroll_left();
                    self.state.needs_redraw = true;
                }
                CompositorAction::ScrollWorkspaceRight => {
                    self.state.workspace_manager.write().scroll_right();
                    self.state.needs_redraw = true;
                }
                CompositorAction::Quit => {
                    self.state.running = false;
                }
                // Add more actions as needed
                _ => {}
            }
        }
    }

    fn render(&mut self) -> Result<()> {
        let Some(backend) = self.winit_backend.as_mut() else {
            return Ok(());
        };

        // Bind and submit a blank frame
        // TODO: Implement proper surface rendering with GlesFrame
        backend.bind()?;

        // For now, just submit without rendering - the backend clears to black by default
        backend.submit(None)?;

        Ok(())
    }

    /// Async process events (for compositor integration)
    #[allow(clippy::unused_async)]
    pub async fn process_events(&mut self) -> Result<()> {
        self.run_one_cycle()
    }

    /// Shutdown the backend
    pub fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Shutting down Smithay backend");
        self.state.running = false;
        Ok(())
    }
}
