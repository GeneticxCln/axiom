//! Smithay 0.7 Backend for Axiom Compositor
#![allow(missing_docs)]
//!
//! This module implements the Wayland compositor backend using Smithay 0.7's
//! handler trait pattern.
//!
//! Phase 6 completions:
//! - 6.2: Wire toplevel state and window lifecycle
//! - 6.3: Route winit input events through InputManager for global keybindings,
//!        forward non-binding keys to Wayland clients via the seat
//! - 6.4: GL scissor-based window placeholder rendering at correct workspace positions

use crate::config::AxiomConfig;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::renderer::AxiomRenderer;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use anyhow::Result;
use log::{debug, info, warn};

use std::collections::HashMap;
use std::os::unix::io::OwnedFd;
use std::sync::Arc;

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
            PointerButtonEvent, Axis,
        },
        renderer::{gles::GlesRenderer, utils::on_commit_buffer_handler},
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm,
    delegate_xdg_shell,
    input::{
        keyboard::{FilterResult, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::{protocol::wl_seat, Display, ListeningSocket},
    utils::{Point, Serial, Transform, SERIAL_COUNTER},
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

    // Keep ToplevelSurface handles alive (they get destroyed when dropped)
    pub toplevels: HashMap<u32, ToplevelSurface>,

    // Running state
    pub running: bool,
    pub needs_redraw: bool,

    // Current window/viewport size (updated via Resized events after dispatch)
    pub window_width: u32,
    pub window_height: u32,

    // Pointer tracking for input routing
    pub pointer_x: f64,
    pub pointer_y: f64,

    // GL texture cache for client SHM buffers (surface_id → GL texture handle)
    pub texture_cache: HashMap<u32, gl::types::GLuint>,

    // Raw SHM buffer data cache for pending GL texture uploads
    // (stored in commit handler, consumed in render() after backend.bind())
    pub buffer_cache: HashMap<u32, Vec<u8>>,
    pub buffer_cache_dimensions: HashMap<u32, (i32, i32)>,

    // GL texture handles pending deletion (cleaned up in render() with GL context)
    pub dead_tex_handles: Vec<gl::types::GLuint>,
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
            "🪟 Creating window from surface {} (title: \"{}\", app_id: {:?})",
            surface_id, title, app_id
        );

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
        // Release the toplevel handle to prevent memory leaks
        self.toplevels.remove(&surface_id);

        if let Some(data) = self.surfaces.remove(&surface_id) {
            if let Some(window_id) = data.window_id {
                info!("🗑️ Destroying window {} (was: \"{}\")", window_id, data.title);
                self.window_map.remove(&window_id);
                self.window_manager.write().remove_window(window_id);
                self.workspace_manager.write().remove_window(window_id);
            }
        }
    }

    /// Check if a window (by Axiom window ID) has a committed surface
    pub fn window_has_buffer(&self, window_id: u64) -> bool {
        self.window_map
            .get(&window_id)
            .and_then(|surface_id| self.surfaces.get(surface_id))
            .map(|s| s.committed)
            .unwrap_or(false)
    }

    /// Find the window ID for a given WlSurface
    pub fn window_id_for_surface(&self, surface: &WlSurface) -> Option<u64> {
        let surface_id = surface.id().protocol_id();
        self.surfaces
            .get(&surface_id)
            .and_then(|s| s.window_id)
    }

    /// Prune surfaces and toplevels whose WlSurface is no longer alive
    /// (e.g. the Wayland client disconnected). Returns count of cleaned entries.
    pub fn prune_dead_surfaces(&mut self) -> usize {
        let dead_surface_ids: Vec<u32> = self
            .surfaces
            .iter()
            .filter(|(_, data)| {
                data.surface
                    .as_ref()
                    .map_or(true, |s| !s.is_alive())
            })
            .map(|(id, _)| *id)
            .collect();

        let count = dead_surface_ids.len();
        for surface_id in dead_surface_ids {
            // Queue GL texture for deferred deletion (no GL context here)
            if let Some(tex) = self.texture_cache.remove(&surface_id) {
                self.dead_tex_handles.push(tex);
            }
            // Drop cached buffer data
            self.buffer_cache.remove(&surface_id);
            self.buffer_cache_dimensions.remove(&surface_id);
            self.destroy_window(surface_id);
        }

        if count > 0 {
            info!("🧹 Pruned {} dead surfaces from disconnected clients", count);
        }
        count
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
        &client
            .get_data::<ClientState>()
            .expect("client state not initialized")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.needs_redraw = true;

        let surface_id = surface.id().protocol_id();

        // Mark surface as committed
        if let Some(surface_data) = self.surfaces.get_mut(&surface_id) {
            surface_data.committed = true;
        }

        // Upload SHM buffer to wgpu renderer and cache raw data for GL upload
        let window_id = self
            .window_map
            .iter()
            .find_map(|(&wid, &sid)| if sid == surface_id { Some(wid) } else { None });

        if let Some(wid) = window_id {
            let renderer = self.renderer.clone();
            let buffer_cache_sid = surface_id;

            // Use Rc<RefCell> to share mutable state with the closure without
            // conflicting with self's borrow
            let cached_data: std::rc::Rc<std::cell::RefCell<Option<(Vec<u8>, i32, i32)>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
            let cached_clone = cached_data.clone();

            with_states(surface, move |states| {
                let mut attrs = states.cached_state.get::<SurfaceAttributes>();
                let buffer = &attrs.current().buffer;

                if let Some(BufferAssignment::NewBuffer(wl_buffer)) = buffer {
                    let _ = with_buffer_contents(wl_buffer, |ptr, len, spec| {
                        let width = spec.width as u32;
                        let height = spec.height as u32;
                        let stride = spec.stride as usize;

                        if len > 0 {
                            let data = unsafe { std::slice::from_raw_parts(ptr, len) };

                            // Upload to wgpu
                            if stride as u32 >= width * 4 && len >= (height as usize * stride) {
                                renderer
                                    .write()
                                    .update_window_texture(wid, width, height, data);
                            }

                            // Cache for GL upload
                            cached_clone.borrow_mut().replace((data.to_vec(), spec.width, spec.height));
                        }
                    });
                }
            });

            // Transfer cached data into self's buffer_cache
            let taken = cached_data.borrow_mut().take();
            if let Some((buf_data, w, h)) = taken {
                self.buffer_cache.insert(buffer_cache_sid, buf_data);
                self.buffer_cache_dimensions.insert(buffer_cache_sid, (w, h));
            }
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

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {
        if let Some(surface) = _focused {
            if let Some(window_id) = self.window_id_for_surface(surface) {
                let _ = self.window_manager.write().focus_window(window_id);
                debug!("🎯 Wayland focus changed to window {}", window_id);
            }
        }
    }

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
        let wl_surface = surface.wl_surface().clone();
        let surface_id = wl_surface.id().protocol_id();

        // Activate the surface
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Activated);
        });

        // Send initial configure (1024x720 default)
        surface.with_pending_state(|state| {
            state.size = Some((1024, 720).into());
        });
        surface.send_configure();

        // Keep the ToplevelSurface alive — it is destroyed when dropped
        self.toplevels.insert(surface_id, surface.clone());

        info!("🪟 New XDG toplevel: surface={}", surface_id);

        self.create_window_from_surface(
            surface_id,
            String::from("Wayland Client"),
            None,
            wl_surface,
        );
        self.needs_redraw = true;
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn ack_configure(&mut self, _surface: WlSurface, _configure: smithay::wayland::shell::xdg::Configure) {}

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
    /// GLES 2.0 shader program for rendering client textures
    pub shader_program: Option<gl::types::GLuint>,
    /// Wayland listening socket — kept alive so clients can connect
    /// (Smithay's display.dispatch_clients polls it internally)
    #[allow(dead_code)]
    listener: Option<ListeningSocket>,
}

impl AxiomSmithayBackendReal {
    /// Test-only constructor that skips Wayland socket bind and display creation.
    /// Creates a minimal backend that supports compositor unit tests without
    /// requiring real system resources (no socket, no GPU init, no display).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_test(
        config: AxiomConfig,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        renderer: Arc<parking_lot::RwLock<AxiomRenderer>>,
    ) -> Result<Self> {
        // Use a dummy display (bound to "null" — never dispatched)
        let display = Display::new()?;
        let dh = display.handle();

        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "axiom-test");

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
            outputs: Vec::new(),
            xwm: None,
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            dead_tex_handles: Vec::new(),
        };

        Ok(Self {
            display,
            socket_name: String::from("axiom-test-dummy"),
            state,
            winit_backend: None,
            winit_event_loop: None,
            clients: Vec::new(),
            shader_program: None,
            listener: None,
        })
    }

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

        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "axiom");

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
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            dead_tex_handles: Vec::new(),
        };

        let socket_name = format!("wayland-axiom-{}", std::process::id());
        let listener = ListeningSocket::bind(&socket_name)?;
        info!("📡 Wayland socket: {}", socket_name);

        Ok(Self {
            display,
            socket_name,
            state,
            winit_backend: None,
            winit_event_loop: None,
            clients: Vec::new(),
            shader_program: None,
            listener: Some(listener),
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

        let _keyboard = self
            .state
            .seat
            .add_keyboard(XkbConfig::default(), 200, 200)?;

        self.state.seat.add_pointer();

        info!("✅ Input devices registered with seat");

        // Compile GLES 2.0 shader program for texture rendering (deferred until first render)
        // The GL context isn't active yet — compilation happens lazily in render()
        info!("🎨 GLES 2.0 shader will be compiled on first render");

        Ok(())
    }

    /// Run one cycle of the event loop
    pub fn run_one_cycle(&mut self) -> Result<()> {
        let Some(winit_event_loop) = self.winit_event_loop.as_mut() else {
            return Ok(());
        };

        // Collect events that need post-dispatch processing
        let mut input_events: Vec<InputEvent<winit::WinitInput>> = Vec::new();
        let mut resized_to: Option<(u32, u32)> = None;
        let mut close_requested = false;

        winit_event_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, .. } => {
                // Size<i32, Physical> — use .w and .h
                resized_to = Some((size.w as u32, size.h as u32));
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
        if let Some((w, h)) = resized_to {
            info!("📐 Window resized to {}x{}", w, h);
            self.state.window_width = w;
            self.state.window_height = h;
            self.state.needs_redraw = true;
        }

        // Process close
        if close_requested {
            info!("📨 Close requested");
            self.state.running = false;
        }

        // Process collected input events
        for event in input_events {
            self.handle_input_event(event);
        }

        // ALWAYS update animations every cycle (not just on redraw)
        // so spring physics, workspace transitions, and effects progress smoothly
        let _ = self.state.workspace_manager.write().update_animations();
        let _ = self.state.effects_engine.write().update();

        // Dispatch Wayland client events
        self.display.dispatch_clients(&mut self.state)?;
        self.display.flush_clients()?;

        // Prune dead surfaces from disconnected clients
        self.state.prune_dead_surfaces();

        // Render if needed
        if self.state.needs_redraw {
            self.render()?;
            self.state.needs_redraw = false;
        }

        Ok(())
    }

    /// Process a single winit input event
    fn handle_input_event(&mut self, event: InputEvent<winit::WinitInput>) {
        use smithay::backend::input::Event;

        match event {
            InputEvent::Keyboard { event } => {
                if let Some(keyboard) = self.state.seat.get_keyboard() {
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = Event::time_msec(&event);
                    let pressed =
                        event.state() == smithay::backend::input::KeyState::Pressed;

                    let input_manager = self.state.input_manager.clone();
                    let pending_actions =
                        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                    let pending_clone = pending_actions.clone();

                    keyboard.input::<(), _>(
                        &mut self.state,
                        event.key_code(),
                        event.state(),
                        serial,
                        time,
                        |_, modifiers, handle| {
                            if pressed {
                                let syms = handle.modified_syms();
                                if let Some(keysym) = syms.first() {
                                    let key_name =
                                        xkbcommon::xkb::keysym_get_name(*keysym);

                                    let mut mod_names: Vec<String> = Vec::new();
                                    if modifiers.ctrl {
                                        mod_names.push("Ctrl".to_string());
                                    }
                                    if modifiers.alt {
                                        mod_names.push("Alt".to_string());
                                    }
                                    if modifiers.logo {
                                        mod_names.push("Super".to_string());
                                    }
                                    if modifiers.shift {
                                        mod_names.push("Shift".to_string());
                                    }

                                    let key_combo = if mod_names.is_empty() {
                                        key_name.to_lowercase()
                                    } else {
                                        format!(
                                            "{}+{}",
                                            mod_names.join("+"),
                                            key_name
                                        )
                                    };

                                    let axiom_event =
                                        crate::input::InputEvent::Keyboard {
                                            key: key_combo.clone(),
                                            modifiers: mod_names,
                                            pressed: true,
                                        };

                                    let actions = input_manager
                                        .write()
                                        .process_input_event(axiom_event);

                                    if !actions.is_empty() {
                                        debug!(
                                            "⌨️ Global shortcut: {}",
                                            key_combo
                                        );
                                        *pending_clone.borrow_mut() = actions;
                                        return FilterResult::Intercept(());
                                    }
                                }
                            }
                            FilterResult::Forward
                        },
                    );

                    // Process any actions that were intercepted
                    let actions: Vec<_> = pending_actions.borrow_mut().drain(..).collect();
                    if !actions.is_empty() {
                        self.process_actions(actions);
                    }
                }
            }

            InputEvent::PointerMotionAbsolute { event } => {
                let (x, y) = (event.x(), event.y());
                self.state.pointer_x = x;
                self.state.pointer_y = y;

                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                if let Some(pointer) = self.state.seat.get_pointer() {
                    // Find the surface under the pointer and forward motion
                    // Skip dead surfaces (from disconnected clients)
                    let under = self.state.workspace_manager.read().element_under(x, y);
                    let focus = under.and_then(|(window_id, (sx, sy))| {
                        self.state
                            .window_map
                            .get(&window_id)
                            .and_then(|surface_id| {
                                self.state
                                    .surfaces
                                    .get(surface_id)
                                    .and_then(|sd| {
                                        sd.surface.as_ref().and_then(|s| {
                                            if s.is_alive() { Some(s.clone()) } else { None }
                                        })
                                    })
                            })
                            .map(|surface| (surface, Point::from((sx, sy))))
                    });

                    let motion_event = MotionEvent {
                        serial,
                        time,
                        location: Point::from((x, y)),
                    };
                    pointer.motion(&mut self.state, focus, &motion_event);
                }
            }

            InputEvent::PointerButton { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                if let Some(pointer) = self.state.seat.get_pointer() {
                    // Convert MouseButton to u32 button code
                    let button_code = match event.button() {
                        Some(smithay::backend::input::MouseButton::Left) => 0x110,
                        Some(smithay::backend::input::MouseButton::Right) => 0x111,
                        Some(smithay::backend::input::MouseButton::Middle) => 0x112,
                        None => 0,
                        _ => 0,
                    };
                    let button_event = ButtonEvent {
                        serial,
                        time,
                        button: button_code,
                        state: event.state(),
                    };
                    pointer.button(&mut self.state, &button_event);
                }
            }

            InputEvent::PointerAxis { event } => {
                // Forward axis/scroll events via seat with actual axis values
                let time = Event::time_msec(&event);

                if let Some(pointer) = self.state.seat.get_pointer() {
                    let mut axis_frame = AxisFrame::new(time);

                    // Extract and forward horizontal/vertical scroll amounts
                    // Using the `input` crate's Axis enum (Horizontal/Vertical)
                    if let Some(amount) = event.amount(Axis::Horizontal) {
                        if amount.abs() > 0.0 {
                            axis_frame = axis_frame.value(Axis::Horizontal, amount);
                        }
                    }
                    if let Some(amount) = event.amount(Axis::Vertical) {
                        if amount.abs() > 0.0 {
                            axis_frame = axis_frame.value(Axis::Vertical, amount);
                        }
                    }

                    pointer.axis(&mut self.state, axis_frame);
                    pointer.frame(&mut self.state);

                    // Workspace navigation: large horizontal scrolls scroll the workspace
                    if let Some(amount) = event.amount(Axis::Horizontal) {
                        if amount > 5.0 {
                            self.state.workspace_manager.write().scroll_right();
                            self.state.needs_redraw = true;
                        } else if amount < -5.0 {
                            self.state.workspace_manager.write().scroll_left();
                            self.state.needs_redraw = true;
                        }
                    }
                }
            }

            _ => {}
        }
    }

    /// Process actions generated by InputManager
    fn process_actions(&mut self, actions: Vec<crate::input::CompositorAction>) {
        use crate::input::CompositorAction;
        for action in actions {
            match action {
                CompositorAction::ScrollWorkspaceLeft => {
                    info!("⬅️  Input: Scroll workspace left");
                    self.state.workspace_manager.write().scroll_left();
                    self.state.needs_redraw = true;
                }
                CompositorAction::ScrollWorkspaceRight => {
                    info!("➡️  Input: Scroll workspace right");
                    self.state.workspace_manager.write().scroll_right();
                    self.state.needs_redraw = true;
                }
                CompositorAction::Quit => {
                    info!("💼 Input: Quit compositor");
                    self.state.running = false;
                }
                CompositorAction::CloseWindow => {
                    let focused_id =
                        self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        info!("🗑️  Input: Close window {}", window_id);
                        if let Some(&surface_id) =
                            self.state.window_map.get(&window_id)
                        {
                            self.state.destroy_window(surface_id);
                            self.state.needs_redraw = true;
                        }
                    }
                }
                CompositorAction::ToggleFullscreen => {
                    let focused_id =
                        self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        let _ = self
                            .state
                            .window_manager
                            .write()
                            .toggle_fullscreen(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::MoveWindowLeft => {
                    let windows = self
                        .state
                        .workspace_manager
                        .read()
                        .get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.state
                            .workspace_manager
                            .write()
                            .move_window_left(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::MoveWindowRight => {
                    let windows = self
                        .state
                        .workspace_manager
                        .read()
                        .get_focused_column_windows();
                    if let Some(&window_id) = windows.first() {
                        self.state
                            .workspace_manager
                            .write()
                            .move_window_right(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                _ => {}
            }
        }
    }

    /// Render the current frame using GL scissor-based window placeholders
    fn render(&mut self) -> Result<()> {
        let Some(backend) = self.winit_backend.as_mut() else {
            return Ok(());
        };

        let ww = self.state.window_width;
        let wh = self.state.window_height;

        // Calculate layouts from workspace manager (animations already updated in run_one_cycle)
        let layouts = self
            .state
            .workspace_manager
            .read()
            .calculate_workspace_layouts();

        // Update window positions in window manager
        {
            let mut wm = self.state.window_manager.write();
            for (window_id, layout_rect) in &layouts {
                if let Some(window) = wm.get_window_mut(*window_id) {
                    window
                        .window
                        .set_position(layout_rect.x, layout_rect.y);
                    window
                        .window
                        .set_size(layout_rect.width, layout_rect.height);
                }
            }
        }

        // Bind OpenGL context
        backend.bind()?;

        // Delete any GPU textures queued for cleanup
        unsafe {
            for tex in self.state.dead_tex_handles.drain(..) {
                gl::DeleteTextures(1, &tex);
            }
        }
        Self::gl_check_error("texture cleanup");

        // Lazily compile the GLES 2.0 shader program if not yet ready
        let shader_prog = Self::ensure_shader_program_static(&mut self.shader_program);

        // Upload pending SHM buffer data to GL textures
        // (must access state before backend borrow)
        // Only upload buffers that haven't been uploaded yet or have changed
        let pending_uploads: Vec<(u32, Vec<u8>, (i32, i32))> = self
            .state
            .buffer_cache
            .iter()
            .filter(|(&sid, _)| {
                // Skip if we already have a GL texture for this surface
                !self.state.texture_cache.contains_key(&sid)
            })
            .map(|(&sid, data)| {
                let dims = self.state.buffer_cache_dimensions.get(&sid).copied().unwrap_or((640, 480));
                (sid, data.clone(), dims)
            })
            .collect();
        self.state.buffer_cache.clear();
        self.state.buffer_cache_dimensions.clear();

        for (surface_id, data, (w, h)) in &pending_uploads {
            Self::upload_gl_texture_static(&mut self.state.texture_cache, *surface_id, data, *w, *h);
        }
        if !pending_uploads.is_empty() {
            Self::gl_check_error("texture uploads");
        }

        // Render using GL
        unsafe {
            gl::ClearColor(0.08, 0.08, 0.12, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }
        Self::gl_check_error("render clear");

        unsafe {

            for (window_id, rect) in &layouts {
                let w = (rect.width as i32).max(0);
                let h = (rect.height as i32).max(0);
                if w == 0 || h == 0 {
                    continue;
                }

                // Flip Y for OpenGL (origin at bottom-left)
                let y = (wh as i32)
                    .saturating_sub(rect.y)
                    .saturating_sub(rect.height as i32);
                let y = y.max(0);

                let surface_id = self.state.window_map.get(window_id).copied();
                let tex = surface_id.and_then(|sid| self.state.texture_cache.get(&sid).copied());
                let has_buffer = surface_id
                    .and_then(|sid| self.state.surfaces.get(&sid))
                    .map(|s| s.committed)
                    .unwrap_or(false);

                if let Some(tex_id) = tex {
                    // Draw actual client texture using GLES 2.0 shader
                    Self::draw_textured_quad_static(shader_prog, tex_id, rect.x, y, w, h, ww, wh);
                } else {
                    // Scissor-based colored placeholder
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(rect.x, y, w, h);
                    if has_buffer {
                        gl::ClearColor(0.15, 0.15, 0.18, 1.0);
                    } else {
                        gl::ClearColor(0.12, 0.12, 0.15, 1.0);
                    }
                    gl::Clear(gl::COLOR_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                }
            }
        }
        Self::gl_check_error("window rendering");

        backend.submit(None)?;

        let textured_count = self.state.texture_cache.len();
        debug!(
            "🎨 Rendered {} windows ({} with GL textures) at {}x{}",
            layouts.len(),
            textured_count,
            ww,
            wh
        );

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

        /// Check for OpenGL errors and log any found.
    fn gl_check_error(context: &str) {
        unsafe {
            loop {
                let err = gl::GetError();
                if err == gl::NO_ERROR {
                    break;
                }
                let err_name = match err {
                    gl::INVALID_ENUM => "GL_INVALID_ENUM",
                    gl::INVALID_VALUE => "GL_INVALID_VALUE",
                    gl::INVALID_OPERATION => "GL_INVALID_OPERATION",
                    gl::OUT_OF_MEMORY => "GL_OUT_OF_MEMORY",
                    other => {
                        warn!("⚠️ GL error ({}) in {}: code 0x{:X}", context, context, other);
                        continue;
                    }
                };
                warn!("⚠️ GL error ({}) in {}", err_name, context);
            }
        }
    }

    /// Upload raw SHM buffer data to an OpenGL texture for a surface.
    /// Static method that doesn't borrow self — works with just the texture cache.
    fn upload_gl_texture_static(
        texture_cache: &mut HashMap<u32, gl::types::GLuint>,
        surface_id: u32,
        data: &[u8],
        width: i32,
        height: i32,
    ) {
        unsafe {
            let tex_id = texture_cache.get(&surface_id).copied().unwrap_or_else(|| {
                let mut tex: gl::types::GLuint = 0;
                gl::GenTextures(1, &mut tex);
                debug!("🖼️ Created GL texture {} for surface {}", tex, surface_id);
                tex
            });

            gl::BindTexture(gl::TEXTURE_2D, tex_id);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                width,
                height,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const std::ffi::c_void,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::BindTexture(gl::TEXTURE_2D, 0);

            texture_cache.insert(surface_id, tex_id);
        }
    }

    /// Compile a simple GLES 2.0 shader.
    fn compile_shader(shader_type: gl::types::GLenum, source: &str) -> Option<gl::types::GLuint> {
        unsafe {
            let shader = gl::CreateShader(shader_type);
            if shader == 0 {
                return None;
            }
            gl::ShaderSource(
                shader,
                1,
                &(source.as_ptr() as *const gl::types::GLchar),
                &(source.len() as gl::types::GLint),
            );
            gl::CompileShader(shader);
            let mut compiled: gl::types::GLint = 0;
            gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut compiled);
            if compiled == 0 {
                let mut len = 0;
                gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
                let mut buf = vec![0u8; len as usize];
                gl::GetShaderInfoLog(
                    shader,
                    len,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr() as *mut gl::types::GLchar,
                );
                let log = String::from_utf8_lossy(&buf);
                warn!("Shader compile failed: {}", log);
                gl::DeleteShader(shader);
                return None;
            }
            Some(shader)
        }
    }

    /// Ensure the GLES 2.0 shader program for texture rendering is compiled and linked.
    /// Static version that doesn't borrow self, to work with the backend borrow.
    fn ensure_shader_program_static(shader_program: &mut Option<gl::types::GLuint>) -> Option<gl::types::GLuint> {
        if let Some(prog) = *shader_program {
            return Some(prog);
        }

        let vert_src = r#"
            attribute vec2 a_position;
            attribute vec2 a_texcoord;
            varying vec2 v_texcoord;
            void main() {
                gl_Position = vec4(a_position, 0.0, 1.0);
                v_texcoord = a_texcoord;
            }
        "#;

        let frag_src = r#"
            precision mediump float;
            varying vec2 v_texcoord;
            uniform sampler2D u_texture;
            void main() {
                gl_FragColor = texture2D(u_texture, v_texcoord);
            }
        "#;

        unsafe {
            let vs = Self::compile_shader(gl::VERTEX_SHADER, vert_src);
            let fs = Self::compile_shader(gl::FRAGMENT_SHADER, frag_src);

            if let (Some(vs), Some(fs)) = (vs, fs) {
                let prog = gl::CreateProgram();
                gl::AttachShader(prog, vs);
                gl::AttachShader(prog, fs);
                gl::LinkProgram(prog);

                let mut linked: gl::types::GLint = 0;
                gl::GetProgramiv(prog, gl::LINK_STATUS, &mut linked);
                if linked != 0 {
                    info!("🎨 GLES 2.0 texture shader compiled successfully (program {})", prog);
                    *shader_program = Some(prog);
                } else {
                    let mut len = 0;
                    gl::GetProgramiv(prog, gl::INFO_LOG_LENGTH, &mut len);
                    let mut buf = vec![0u8; len as usize];
                    gl::GetProgramInfoLog(
                        prog,
                        len,
                        std::ptr::null_mut(),
                        buf.as_mut_ptr() as *mut gl::types::GLchar,
                    );
                    warn!("Shader link failed: {}", String::from_utf8_lossy(&buf));
                    gl::DeleteProgram(prog);
                }

                gl::DeleteShader(vs);
                gl::DeleteShader(fs);
                // Return the newly created program
                return *shader_program;
            }

            // vs or fs failed to compile; both may already be deleted in compile_shader
            // on failure, but we clean up any remaining handles
            if let Some(v) = vs { gl::DeleteShader(v); }
            if let Some(f) = fs { gl::DeleteShader(f); }
        }

        None
    }



    /// Draw a textured quad using the GLES 2.0 shader program.
    /// Static version that takes shader_program explicitly to avoid borrowing self.
    /// Coordinates are in pixel space; this method converts to NDC.
    fn draw_textured_quad_static(
        shader_program: Option<gl::types::GLuint>,
        tex_id: gl::types::GLuint,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        screen_w: u32,
        screen_h: u32,
    ) {
        let Some(prog) = shader_program else {
            return;
        };

        // Convert pixel coordinates to NDC [-1, 1]
        let sw = screen_w as f32;
        let sh = screen_h as f32;
        let x1 = (x as f32 / sw) * 2.0 - 1.0;
        let y1 = (y as f32 / sh) * 2.0 - 1.0;
        let x2 = ((x + w) as f32 / sw) * 2.0 - 1.0;
        let y2 = ((y + h) as f32 / sh) * 2.0 - 1.0;

        #[rustfmt::skip]
        let vertices: [f32; 16] = [
            x1, y1, 0.0, 0.0,  // top-left    (flip V: SHM top-left → GL bottom-left)
            x2, y1, 1.0, 0.0,  // top-right
            x1, y2, 0.0, 1.0,  // bottom-left
            x2, y2, 1.0, 1.0,  // bottom-right
        ];

        unsafe {
            gl::UseProgram(prog);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, tex_id);

            let pos_loc = gl::GetAttribLocation(
                prog,
                std::ffi::CStr::from_bytes_with_nul(b"a_position\0")
                    .unwrap()
                    .as_ptr(),
            );
            let tex_loc = gl::GetAttribLocation(
                prog,
                std::ffi::CStr::from_bytes_with_nul(b"a_texcoord\0")
                    .unwrap()
                    .as_ptr(),
            );

            let stride = (4 * std::mem::size_of::<f32>()) as gl::types::GLsizei;

            if pos_loc >= 0 {
                gl::EnableVertexAttribArray(pos_loc as gl::types::GLuint);
                gl::VertexAttribPointer(
                    pos_loc as gl::types::GLuint,
                    2,
                    gl::FLOAT,
                    gl::FALSE,
                    stride,
                    vertices.as_ptr() as *const std::ffi::c_void,
                );
            }
            if tex_loc >= 0 {
                gl::EnableVertexAttribArray(tex_loc as gl::types::GLuint);
                gl::VertexAttribPointer(
                    tex_loc as gl::types::GLuint,
                    2,
                    gl::FLOAT,
                    gl::FALSE,
                    stride,
                    vertices.as_ptr().add(2) as *const std::ffi::c_void,
                );
            }

            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

            if pos_loc >= 0 {
                gl::DisableVertexAttribArray(pos_loc as gl::types::GLuint);
            }
            if tex_loc >= 0 {
                gl::DisableVertexAttribArray(tex_loc as gl::types::GLuint);
            }

            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::UseProgram(0);
        }
    }
}
