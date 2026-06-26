//! Smithay 0.7 Backend for Axiom Compositor
//!
//! This module implements the Wayland compositor backend using Smithay 0.7's
//! handler trait pattern. It manages the Wayland display, protocol state,
//! input routing, and GL/WGPU rendering.
//!
//! ## Phase 6 completions
//! - 6.2: Wire toplevel state and window lifecycle
//! - 6.3: Route winit input events through InputManager for global keybindings
//! - 6.4: GL scissor-based window placeholder rendering at correct workspace positions

use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::renderer::AxiomRenderer;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use anyhow::Result;
use log::{debug, info, warn};

use std::collections::{HashMap, HashSet};
use std::os::unix::io::OwnedFd;
use std::sync::Arc;

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
            PointerButtonEvent,
        },
        renderer::{gles::GlesRenderer, utils::on_commit_buffer_handler},
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
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
            SelectionHandler, SelectionSource, SelectionTarget,
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

mod shm_upload;
mod xwayland_dispatch;
pub mod xwm;
use self::xwm::AxiomXwm;

// Type alias to reduce complexity of the Rc<RefCell<Option<...>>> pattern
// used for passing buffer data out of the SHM commit closure.
type CachedBufferData = std::rc::Rc<std::cell::RefCell<Option<(Vec<u8>, i32, i32)>>>;

// ============================================================================
// Surface Data
// ============================================================================

/// Surface data for tracking Wayland surfaces
#[derive(Debug, Clone)]
pub struct SurfaceData {
    pub window_id: Option<u64>,
    pub title: String,
    pub app_id: Option<String>,
    /// Actual buffer dimensions (updated when client commits a buffer).
    pub size: (i32, i32),
    pub committed: bool,
    pub surface: Option<WlSurface>,
}

/// State for tracking an XDG popup surface (menu, tooltip, etc.).
pub struct PopupState {
    /// Protocol ID of the parent toplevel or popup surface.
    pub parent_surface_id: u32,
    /// Popup position relative to parent.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Whether the surface has been committed (mapped).
    pub committed: bool,
    /// The popup surface handle.
    pub surface: PopupSurface,
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
    pub renderer: Option<Arc<parking_lot::RwLock<AxiomRenderer>>>,

    // Tracking
    pub surfaces: HashMap<u32, SurfaceData>,
    pub window_map: HashMap<u64, u32>,
    pub next_window_id: u64,

    /// Maps X11 window IDs to Axiom window IDs for XWayland clients.
    pub x11_window_map: HashMap<u32, u64>,

    // Outputs
    pub outputs: Vec<Output>,

    // XWayland (optional)
    pub xwm: Option<AxiomXwm>,

    /// Server-side decoration manager for titlebar/button rendering.
    /// Shared with [`AxiomCompositor`](crate::compositor::AxiomCompositor).
    pub decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,

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

    /// Tracks whether we've sent the initial configure for a surface.
    /// Used to throttle redundant configure events when layout hasn't changed.
    pub configured_sizes: HashMap<u32, (i32, i32)>,

    /// Surfaces with an outstanding (unacknowledged) xdg_toplevel.configure.
    /// New configures are deferred until the client acks the current one.
    pub pending_configure: HashSet<u32>,

    /// Active XDG popup surfaces (menus, tooltips, etc.).
    pub popups: HashMap<u32, PopupState>,

    /// If set, a popup grab is active — clicks outside this popup
    /// surface ID will dismiss it via popup_done().
    pub active_popup_grab: Option<u32>,

    /// Cached Wayland clipboard data for X11 selection bridging.
    /// Populated when a Wayland client sets the selection; served to X11
    /// apps that request clipboard contents.
    pub clipboard_cache: Option<Vec<u8>>,

    /// Active Wayland selection source (when a client owns the clipboard).
    /// Stored so we can serve data to X11 and re-offer to other Wayland clients.
    ///
    /// ## Clipboard bridging (Wayland → X11)
    ///
    /// In Smithay 0.7, `SelectionSource` is created by the Wayland client
    /// with a callback — there is no `send()` method to call from the
    /// compositor side. Extracting text/plain data from a Wayland source
    /// requires the compositor to **act as its own Wayland client** and
    /// request the selection via `wl_data_device.data_offer.receive`.
    /// This needs a protocol round-trip through the event loop and is
    /// deferred to a follow-up PR (tracked in Phase 3 protocol work).
    ///
    /// The `clipboard_cache` can still be populated via the compositor's
    /// own IPC path (`AxiomSmithayBackendReal::set_clipboard_data`) for
    /// the user-facing direction (compositor → X11).
    pub clipboard_source: Option<SelectionSource>,
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

        // Trigger window open animation (spring-physics scale + fade-in)
        self.effects_engine.write().animate_window_open(window_id);

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

        // Register with DecorationManager using real window geometry.
        self.decoration_manager.write().add_window(
            window_id,
            String::from("Wayland Client"),
            /* prefers_server_side */ true,
            640,
        );

        window_id
    }

    /// Create a new Axiom window for an X11 client (via XWayland).
    pub fn create_window_from_x11(&mut self, x11_window_id: u32, title: String) -> u64 {
        info!(
            "Creating window from X11 window {} (title: \"{}\")",
            x11_window_id, title
        );

        let window_id = self.window_manager.write().add_window(title.clone());
        self.workspace_manager.write().add_window(window_id);
        self.effects_engine.write().animate_window_open(window_id);

        self.decoration_manager.write().add_window(
            window_id,
            title.clone(),
            /* prefers_server_side */ true,
            640,
        );

        self.x11_window_map.insert(x11_window_id, window_id);

        // TODO: mint ForeignToplevelHandle via foreign_toplevel_list_state
        // when Smithay ≥0.8 (delegate_foreign_toplevel_list! macro).

        window_id
    }

    pub fn destroy_window(&mut self, surface_id: u32) {
        // TODO: send_closed + remove_toplevel on the ForeignToplevelHandle
        // when Smithay ≥0.8 (delegate_foreign_toplevel_list! macro).
        // Release the toplevel handle to prevent memory leaks
        self.toplevels.remove(&surface_id);

        // Clean up configure tracking
        self.configured_sizes.remove(&surface_id);
        self.pending_configure.remove(&surface_id);

        if let Some(data) = self.surfaces.remove(&surface_id) {
            if let Some(window_id) = data.window_id {
                info!(
                    "Destroying window {} (was: \"{}\")",
                    window_id, data.title
                );
                self.window_map.remove(&window_id);
                self.window_manager.write().remove_window(window_id);
                self.workspace_manager.write().remove_window(window_id);
                self.decoration_manager.write().remove_window(window_id);
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
        self.surfaces.get(&surface_id).and_then(|s| s.window_id)
    }

    /// Prune surfaces and toplevels whose WlSurface is no longer alive
    /// (e.g. the Wayland client disconnected). Returns count of cleaned entries.
    pub fn prune_dead_surfaces(&mut self) -> usize {
        let dead_surface_ids: Vec<u32> = self
            .surfaces
            .iter()
            .filter(|(_, data)| data.surface.as_ref().is_none_or(|s| !s.is_alive()))
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
            info!(
                "🧹 Pruned {} dead surfaces from disconnected clients",
                count
            );
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

        // Mark surface as committed (toplevels and popups)
        if let Some(surface_data) = self.surfaces.get_mut(&surface_id) {
            surface_data.committed = true;
        }
        if let Some(popup) = self.popups.get_mut(&surface_id) {
            popup.committed = true;
        }

        // Upload SHM buffer to wgpu renderer and cache raw data for GL upload
        let window_id =
            self.window_map
                .iter()
                .find_map(|(&wid, &sid)| if sid == surface_id { Some(wid) } else { None });

        if let Some(wid) = window_id {
            let renderer = self.renderer.clone();
            let buffer_cache_sid = surface_id;

            // Use Rc<RefCell> to share mutable state with the closure without
            // conflicting with self's borrow
            let cached_data: CachedBufferData =
                CachedBufferData::new(std::cell::RefCell::new(None));
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
                            // SAFETY: Smithay's `with_buffer_contents` callback
                            // guarantees that `ptr` points to `len` bytes of
                            // valid SHM buffer data for the duration of this
                            // closure. The slice is immediately copied (to_vec)
                            // before the closure returns, so no aliasing occurs.
                            let data = unsafe { std::slice::from_raw_parts(ptr, len) };

                            // Upload to wgpu (if renderer is available)
                            if stride as u32 >= width * 4 && len >= (height as usize * stride) {
                                if let Some(ref renderer) = renderer {
                                    renderer
                                        .write()
                                        .update_window_texture(wid, width, height, data);
                                }
                            }

                            // Cache for GL upload
                            cached_clone.borrow_mut().replace((
                                data.to_vec(),
                                spec.width,
                                spec.height,
                            ));
                        }
                    });
                }
            });

            // Transfer cached data into self's buffer_cache
            let taken = cached_data.borrow_mut().take();
            if let Some((buf_data, w, h)) = taken {
                // Update SurfaceData.size to reflect actual buffer dimensions
                if let Some(sd) = self.surfaces.get_mut(&buffer_cache_sid) {
                    sd.size = (w, h);
                }
                self.buffer_cache.insert(buffer_cache_sid, buf_data);
                self.buffer_cache_dimensions
                    .insert(buffer_cache_sid, (w, h));
            }
        } else if self.popups.contains_key(&surface_id) {
            // Popup buffer upload — GL-only, no WGPU since popups render via GL pass
            let buffer_cache_sid = surface_id;
            let cached_data: CachedBufferData =
                CachedBufferData::new(std::cell::RefCell::new(None));
            let cached_clone = cached_data.clone();

            with_states(surface, move |states| {
                let mut attrs = states.cached_state.get::<SurfaceAttributes>();
                let buffer = &attrs.current().buffer;
                if let Some(BufferAssignment::NewBuffer(wl_buffer)) = buffer {
                    let _ = with_buffer_contents(wl_buffer, |ptr, len, spec| {
                        if len > 0 {
                            // SAFETY: Same as the toplevel window buffer path above.
                            // Smithay guarantees ptr+len are valid for the closure.
                            let data = unsafe { std::slice::from_raw_parts(ptr, len) };
                            cached_clone.borrow_mut().replace((
                                data.to_vec(),
                                spec.width,
                                spec.height,
                            ));
                        }
                    });
                }
            });

            let taken = cached_data.borrow_mut().take();
            if let Some((buf_data, w, h)) = taken {
                self.buffer_cache.insert(buffer_cache_sid, buf_data);
                self.buffer_cache_dimensions
                    .insert(buffer_cache_sid, (w, h));
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
                self.window_manager.write().focus_window(window_id);
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

        // Track the initial configure so render() doesn't immediately re-configure
        self.configured_sizes.insert(surface_id, (1024, 720));
        self.pending_configure.insert(surface_id);

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

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        let surface_id = surface.wl_surface().id().protocol_id();
        let parent_id = surface
            .get_parent_surface()
            .map(|s| s.id().protocol_id())
            .unwrap_or(0);

        // Compute popup geometry from the positioner relative to parent
        let rect = positioner.get_geometry();

        surface.with_pending_state(|state| {
            state.geometry = rect;
        });
        if let Err(e) = surface.send_configure() {
            warn!(
                "⚠️ Popup configure failed for surface {}: {:?}",
                surface_id, e
            );
        }

        info!(
            "💬 New XDG popup: surface={} parent={} pos=({},{}) size={}x{}",
            surface_id, parent_id, rect.loc.x, rect.loc.y, rect.size.w, rect.size.h
        );

        self.popups.insert(
            surface_id,
            PopupState {
                parent_surface_id: parent_id,
                x: rect.loc.x,
                y: rect.loc.y,
                width: rect.size.w,
                height: rect.size.h,
                committed: false,
                surface,
            },
        );
    }

    fn ack_configure(
        &mut self,
        surface: WlSurface,
        _configure: smithay::wayland::shell::xdg::Configure,
    ) {
        let surface_id = surface.id().protocol_id();
        self.pending_configure.remove(&surface_id);
        debug!("✅ Client ack'd configure for surface {}", surface_id);
    }

    fn grab(&mut self, surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        let surface_id = surface.wl_surface().id().protocol_id();
        info!("🤚 Popup grab activated for surface {}", surface_id);
        self.active_popup_grab = Some(surface_id);
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let surface_id = surface.wl_surface().id().protocol_id();
        let rect = positioner.get_geometry();

        if let Some(popup) = self.popups.get_mut(&surface_id) {
            popup.x = rect.loc.x;
            popup.y = rect.loc.y;
            popup.width = rect.size.w;
            popup.height = rect.size.h;
        }

        surface.with_pending_state(|state| {
            state.geometry = rect;
        });
        surface.send_repositioned(token);
        if let Err(e) = surface.send_configure() {
            warn!("⚠️ Popup reposition configure failed: {:?}", e);
        }

        debug!(
            "🔄 Popup repositioned: surface={} pos=({},{}) size={}x{}",
            surface_id, rect.loc.x, rect.loc.y, rect.size.w, rect.size.h
        );
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        match ty {
            SelectionTarget::Clipboard => {
                if let Some(ref src) = source {
                    let mime_types = src.mime_types();
                    debug!(
                        "📋 Wayland clipboard updated with {} MIME types: {:?}",
                        mime_types.len(),
                        mime_types
                    );
                    // Claim X11 clipboard ownership so X11 apps query us (not stale X11 owners)
                    if let Some(xwm) = self.xwm.as_ref() {
                        if let Err(e) = xwm.own_selection() {
                            warn!("⚠️ Failed to claim X11 clipboard ownership: {}", e);
                        }
                    }
                    // Store the source so X11 clipboard requests can extract
                    // text/plain on demand (see State::extract_wayland_clipboard).
                    self.clipboard_source = Some(src.clone());
                } else {
                    debug!("📋 Wayland clipboard cleared");
                    self.clipboard_source = None;
                    self.clipboard_cache = None;
                }
            }
            SelectionTarget::Primary => {
                debug!("📋 Wayland primary selection updated");
            }
        }
    }
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
    /// The `renderer` parameter is optional — pass `None` in headless/CI environments.
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_test(
        config: AxiomConfig,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
        renderer: Option<Arc<parking_lot::RwLock<AxiomRenderer>>>,
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
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
            x11_window_map: HashMap::new(),
            outputs: Vec::new(),
            xwm: None,
            decoration_manager: decoration_manager.clone(),
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
            configured_sizes: HashMap::new(),
            pending_configure: HashSet::new(),
            popups: HashMap::new(),
            active_popup_grab: None,
            clipboard_cache: None,
            clipboard_source: None,
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
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
    ) -> Result<Self> {
        info!("Initializing Smithay 0.7 Backend...");

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
        // NOTE: output.create_global::<State>(&dh) requires Dispatch<WlOutput, OutputData>
        // which needs the delegate_output! macro (not available in Smithay 0.7.0).
        // The output struct still functions for internal tracking; only the Wayland
        // global for client discovery is deferred to a future protocol wiring PR.

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
            renderer: Some(renderer),
            surfaces: HashMap::new(),
            window_map: HashMap::new(),
            next_window_id: 1,
            x11_window_map: HashMap::new(),
            outputs: vec![output],
            xwm: None,
            decoration_manager: decoration_manager.clone(),
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
            configured_sizes: HashMap::new(),
            pending_configure: HashSet::new(),
            popups: HashMap::new(),
            active_popup_grab: None,
            clipboard_cache: None,
            clipboard_source: None,
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

        // Poll X11 events from XWayland (if XWM is wired).
        // The X11 selection/clipboard dispatch lives in xwayland_dispatch.rs.
        self::xwayland_dispatch::poll_and_dispatch_events(&mut self.state)?;

        // Dispatch Wayland client events
        self.display.dispatch_clients(&mut self.state)?;
        self.display.flush_clients()?;

        // After dispatch + flush, any new Wayland clipboard source is
        // stored in `state.clipboard_source`. Full extraction requires
        // acting as a Wayland client (wl_data_device.receive round-trip);
        // see the doc comment on `State::clipboard_source` for details.

        // Update animations after dispatch so newly-created windows (which
        // trigger animate_window_open() during dispatch) get their first
        // integration step before the render pass reads effect states.
        self.state.workspace_manager.write().update_animations();
        let _ = self.state.effects_engine.write().update();

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
                    let pressed = event.state() == smithay::backend::input::KeyState::Pressed;

                    let input_manager = self.state.input_manager.clone();
                    let pending_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
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
                                    let key_name = xkbcommon::xkb::keysym_get_name(*keysym);

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
                                        format!("{}+{}", mod_names.join("+"), key_name)
                                    };

                                    let axiom_event = crate::input::InputEvent::Keyboard {
                                        key: key_combo.clone(),
                                        modifiers: mod_names,
                                        pressed: true,
                                    };

                                    let actions =
                                        input_manager.write().process_input_event(axiom_event);

                                    if !actions.is_empty() {
                                        debug!("⌨️ Global shortcut: {}", key_combo);
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
                                self.state.surfaces.get(surface_id).and_then(|sd| {
                                    sd.surface.as_ref().and_then(|s| {
                                        if s.is_alive() {
                                            Some(s.clone())
                                        } else {
                                            None
                                        }
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

                // Dismiss active popup grab on any button press outside the popup
                if let Some(popup_id) = self.state.active_popup_grab {
                    let dismiss = if let Some(p) = self.state.popups.get(&popup_id) {
                        let px = self.state.pointer_x as i32;
                        let py = self.state.pointer_y as i32;
                        // Find the popup's absolute position by locating its parent window
                        let (abs_x, abs_y) = self
                            .state
                            .window_map
                            .iter()
                            .find_map(|(&wid, &sid)| {
                                if sid == p.parent_surface_id {
                                    self.state
                                        .window_manager
                                        .read()
                                        .get_window(wid)
                                        .map(|w| (w.window.position.0, w.window.position.1))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or((0, 0));
                        let popup_x = abs_x + p.x;
                        let popup_y = abs_y + p.y;
                        px < popup_x
                            || px > popup_x + p.width
                            || py < popup_y
                            || py > popup_y + p.height
                    } else {
                        true
                    };

                    if dismiss {
                        if let Some(p) = self.state.popups.remove(&popup_id) {
                            info!("🗑️ Dismissing popup surface {}", popup_id);
                            p.surface.send_popup_done();
                            self.state.needs_redraw = true;
                        }
                        self.state.active_popup_grab = None;
                    }
                }

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
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        info!("🗑️  Input: Close window {}", window_id);
                        if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                            self.state.destroy_window(surface_id);
                            self.state.needs_redraw = true;
                        }
                    }
                }
                CompositorAction::ToggleFullscreen => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        self.state
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
                    window.window.set_position(layout_rect.x, layout_rect.y);
                    window
                        .window
                        .set_size(layout_rect.width, layout_rect.height);
                }
            }
        }

        // Send xdg_toplevel.configure events for windows whose tiling size changed.
        // This notifies Wayland clients to resize their buffers to match the layout.
        // Done before bind() so we don't hold GL context while doing Wayland protocol work.
        for (window_id, rect) in &layouts {
            if let Some(&surface_id) = self.state.window_map.get(window_id) {
                if let Some(toplevel) = self.state.toplevels.get(&surface_id) {
                    let new_w = (rect.width as i32).max(1);
                    let new_h = (rect.height as i32).max(1);

                    let needs_configure = self
                        .state
                        .configured_sizes
                        .get(&surface_id)
                        .is_none_or(|&(cw, ch)| cw != new_w || ch != new_h);

                    // Skip if client hasn't acknowledged the previous configure yet.
                    let pending = self.state.pending_configure.contains(&surface_id);

                    if needs_configure && !pending {
                        toplevel.with_pending_state(|state| {
                            state.size = Some((new_w, new_h).into());
                        });
                        toplevel.send_configure();
                        self.state
                            .configured_sizes
                            .insert(surface_id, (new_w, new_h));
                        self.state.pending_configure.insert(surface_id);

                        debug!(
                            "📐 Configured surface {} to {}x{}",
                            surface_id, new_w, new_h
                        );
                    }
                }
            }
        }

        // Bind OpenGL context
        backend.bind()?;

        // Delete any GPU textures queued for cleanup
        // SAFETY: An active GL context is guaranteed by `backend.bind()` which
        // is called above. Deleting texture handles that are no longer
        // referenced is safe and idempotent once the context is current.
        shm_upload::delete_textures(&mut self.state.dead_tex_handles);

        // Lazily compile the GLES 2.0 shader program if not yet ready
        let shader_prog = shm_upload::ensure_shader_program(&mut self.shader_program);

        // Drain the SHM buffer cache into a batched upload list, then upload
        // each. Helper consolidates the filter + clone + clear logic that
        // would otherwise re-clone the renderer/surface assumptions inline.
        let pending_uploads = shm_upload::collect_pending_uploads(
            &mut self.state.buffer_cache,
            &mut self.state.buffer_cache_dimensions,
            &self.state.texture_cache,
        );

        for (surface_id, data, (w, h)) in &pending_uploads {
            shm_upload::upload_gl_texture(&mut self.state.texture_cache, *surface_id, data, *w, *h);
        }
        if !pending_uploads.is_empty() {
            shm_upload::gl_check_error("texture uploads");
        }

        // Render using GL
        // SAFETY: GL context is current (backend.bind() succeeded above).
        // ClearColor + Clear are always safe to call with a bound context.
        unsafe {
            gl::ClearColor(0.08, 0.08, 0.12, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }
        shm_upload::gl_check_error("render clear");

        // SAFETY: GL context is current. All GL scissor/draw calls operate on
        // the currently bound framebuffer which Smithay owns. `layouts` entries
        // have been validated to be non-zero size before this block is entered.
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
                    shm_upload::draw_textured_quad(
                        shader_prog,
                        tex_id,
                        &shm_upload::TexQuadParams {
                            x: rect.x,
                            y,
                            w,
                            h,
                            screen_w: ww,
                            screen_h: wh,
                            alpha: 1.0,
                        },
                    );
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
        shm_upload::gl_check_error("window rendering");

        // Render popups on top of windows
        for (popup_id, popup) in &self.state.popups {
            if !popup.committed {
                continue;
            }
            // Find parent window position for absolute popup placement
            let (parent_x, parent_y) = self
                .state
                .window_map
                .iter()
                .find_map(|(&wid, &sid)| {
                    if sid == popup.parent_surface_id {
                        self.state
                            .window_manager
                            .read()
                            .get_window(wid)
                            .map(|w| (w.window.position.0, w.window.position.1))
                    } else {
                        None
                    }
                })
                .unwrap_or((0, 0));

            let popup_x = parent_x + popup.x;
            let popup_y = parent_y + popup.y;
            let popup_w = popup.width.max(1);
            let popup_h = popup.height.max(1);

            // Flip Y for OpenGL
            let gl_y = (wh as i32)
                .saturating_sub(popup_y)
                .saturating_sub(popup_h)
                .max(0);

            let tex = self.state.texture_cache.get(popup_id).copied();

            // SAFETY: GL context is current. `tex_id` is a valid GL texture
            // handle obtained from `texture_cache`; scissor coordinates are
            // bounds-checked above (popup_w/popup_h > 0). All state is
            // restored (SCISSOR_TEST disabled) before the block ends.
            unsafe {
                if let Some(tex_id) = tex {
                    shm_upload::draw_textured_quad(
                        shader_prog,
                        tex_id,
                        &shm_upload::TexQuadParams {
                            x: popup_x,
                            y: gl_y,
                            w: popup_w,
                            h: popup_h,
                            screen_w: ww,
                            screen_h: wh,
                            alpha: 1.0,
                        },
                    );
                } else {
                    gl::Enable(gl::SCISSOR_TEST);
                    gl::Scissor(popup_x, gl_y, popup_w, popup_h);
                    gl::ClearColor(0.2, 0.2, 0.25, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT);
                    gl::Disable(gl::SCISSOR_TEST);
                }
            }
        }
        shm_upload::gl_check_error("popup rendering");

        // ── WGPU Post-Process: shadow / blur compositing ───────────────────
        // Read the GL framebuffer, run GPU effects via the headless WGPU
        // target, then blit the result back as a fullscreen GL quad.
        // The per-frame effect queues were pre-populated by
        // compositor::prepare_frame_data() before process_events().
        //
        // Uses `drain_post_process` (preferred over the deprecated
        // `composite_effects_on_buffer`) — when no shadows/blurs are queued
        // it returns `Ok(None)` and the GL framebuffer is left untouched,
        // avoiding the GL→CPU readback allocation on every no-effects frame.
        if let Some(ref renderer) = self.state.renderer {
            let gl_pixels = shm_upload::read_gl_framebuffer(ww, wh);
            match renderer.write().drain_post_process(ww, wh, gl_pixels) {
                Ok(Some(processed)) => {
                    // Upload processed pixels to a temporary GL texture and
                    // draw a fullscreen quad to replace the framebuffer.
                    let mut tex: gl::types::GLuint = 0;
                    // SAFETY: GL context is current. GenTextures + TexImage2D
                    // are called with a fresh tex name and valid RGBA pixel
                    // data from wgpu's staging readback. The texture is bound
                    // exclusively within this block and unbound on cleanup.
                    unsafe {
                        gl::GenTextures(1, &mut tex);
                        gl::BindTexture(gl::TEXTURE_2D, tex);
                        gl::TexParameteri(
                            gl::TEXTURE_2D,
                            gl::TEXTURE_MIN_FILTER,
                            gl::LINEAR as i32,
                        );
                        gl::TexParameteri(
                            gl::TEXTURE_2D,
                            gl::TEXTURE_MAG_FILTER,
                            gl::LINEAR as i32,
                        );
                        gl::TexParameteri(
                            gl::TEXTURE_2D,
                            gl::TEXTURE_WRAP_S,
                            gl::CLAMP_TO_EDGE as i32,
                        );
                        gl::TexParameteri(
                            gl::TEXTURE_2D,
                            gl::TEXTURE_WRAP_T,
                            gl::CLAMP_TO_EDGE as i32,
                        );
                        gl::TexImage2D(
                            gl::TEXTURE_2D,
                            0,
                            gl::RGBA as i32,
                            ww as i32,
                            wh as i32,
                            0,
                            gl::RGBA,
                            gl::UNSIGNED_BYTE,
                            processed.as_ptr() as *const std::ffi::c_void,
                        );
                    }
                    shm_upload::gl_check_error("effects upload");

                    // Overwrite the framebuffer by drawing a fullscreen quad.
                    // This replaces the GL-rendered content with the
                    // WGPU-composited result (shadows, blurs on top).
                    shm_upload::draw_fullscreen_quad(shader_prog, tex);
                    shm_upload::gl_check_error("effects fullscreen quad");

                    unsafe {
                        gl::DeleteTextures(1, &tex);
                    }
                }
                Ok(None) => {
                    // No effects queued this frame — `drain_post_process` already
                    // cleared the per-frame queues and dropped the readback bytes;
                    // the GL framebuffer we already drew is the final image.
                    debug!(
                        "🎨 No WGPU post-process this frame ({}x{}, no queued shadows/blurs)",
                        ww, wh
                    );
                }
                Err(e) => {
                    warn!(
                        "⚠️ WGPU effects composite failed: {} (showing unprocessed GL frame)",
                        e
                    );
                }
            }
        }
        // ── end WGPU post-process ─────────────────────────────────────────

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

    /// Process events (for compositor integration)
    pub fn process_events(&mut self) -> Result<()> {
        self.run_one_cycle()
    }

    /// Wire an `AxiomXwm` instance into the backend for XWayland clipboard
    /// bridging and X11 window management. Call this once XWayland has spun
    /// up and the X11 connection stream is available.
    pub fn set_xwm(&mut self, xwm: AxiomXwm) {
        info!("🔗 Wiring XWM into backend for X11 clipboard bridging");
        self.state.xwm = Some(xwm);
    }

    /// Populate the clipboard cache with data from an external source
    /// (e.g., Lazy UI IPC, compositor-managed text). When X11 apps request
    /// clipboard contents, this data is served back to them.
    pub fn set_clipboard_data(&mut self, data: Vec<u8>) {
        debug!("📋 Clipboard cache populated ({} bytes)", data.len());
        self.state.clipboard_cache = Some(data);
        // If the XWM is active, claim X11 clipboard ownership so X11 apps
        // come to us for selection data rather than stale X11 owners.
        if let Some(xwm) = self.state.xwm.as_ref() {
            if let Err(e) = xwm.own_selection() {
                warn!("⚠️ Failed to claim X11 clipboard: {}", e);
            }
        }
    }

    /// Shutdown the backend
    pub fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Shutting down Smithay backend");
        self.state.running = false;
        Ok(())
    }
}
