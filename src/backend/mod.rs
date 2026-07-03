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

// Re-export BackendKind from the drm module so the rest of the compositor
// can reference `crate::backend::BackendKind` as documented in the config.
pub use self::drm::{BackendKind, DrmBackend, DrmEventCollector};

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, InputEvent, KeyboardKeyEvent,
            PointerAxisEvent, PointerButtonEvent,
        },
        renderer::{gles::GlesRenderer, utils::on_commit_buffer_handler},
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    delegate_compositor, delegate_data_device, delegate_seat, delegate_shm, delegate_xdg_shell,
    input::{
        keyboard::{FilterResult, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, MotionEvent},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
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
            decoration::{XdgDecorationHandler, XdgDecorationState},
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

pub mod drm;
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
    pub xdg_decoration_state: Option<XdgDecorationState>,

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

    /// Per-output DPI scale factors keyed by output name (e.g. "eDP-1" → 2.0).
    /// Populated by the DRM backend during `initialize_drm()`; empty in
    /// winit/noop mode where scale is implicitly 1.0.
    pub output_scale_factors: HashMap<String, f64>,

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

    // Raw SHM buffer data cache for pending WGPU texture uploads
    // (stored in commit handler, consumed in render() via renderer.upload_shm_to_wgpu)
    pub buffer_cache: HashMap<u32, Vec<u8>>,
    pub buffer_cache_dimensions: HashMap<u32, (i32, i32)>,

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

    /// Most recent cursor icon requested via `cursor_image()` callback.
    /// Applied to the winit window at the start of `render()`.
    pub cursor_icon: Option<CursorIcon>,
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
                info!("Destroying window {} (was: \"{}\")", window_id, data.title);
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

    /// Return the DPI scale factor for the currently focused output.
    /// Reads directly from the focused workspace tape to avoid a
    /// duplicate source of truth. Falls back to 1.0.
    pub fn focused_output_scale(&self) -> f64 {
        self.workspace_manager.read().active_tape().scale_factor()
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

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        match image {
            CursorImageStatus::Named(icon) => self.cursor_icon = Some(icon),
            _ => self.cursor_icon = None,
        }
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

        // Send initial configure scaled to logical pixels for the
        // current output's DPI scale factor. HiDPI-aware clients
        // multiply by buffer_scale to allocate their actual pixel buffers.
        let scale = self.focused_output_scale();
        let logical_w = ((1024.0 / scale).round() as i32).max(1);
        let logical_h = ((720.0 / scale).round() as i32).max(1);
        surface.with_pending_state(|state| {
            state.size = Some((logical_w, logical_h).into());
        });
        surface.send_configure();

        // Track the initial configure so render() doesn't immediately re-configure
        self.configured_sizes
            .insert(surface_id, (logical_w, logical_h));
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

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_configure();

        // Update our DecorationManager to match the client's preference.
        if let Some(window_id) = self.window_id_for_surface(toplevel.wl_surface()) {
            let dm_mode = match mode {
                Mode::ClientSide => crate::decoration::DecorationMode::ClientSide,
                Mode::ServerSide => crate::decoration::DecorationMode::ServerSide,
                _ => return,
            };
            self.decoration_manager
                .write()
                .set_decoration_mode(window_id, dm_mode);
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        // Revert to our default (ServerSide) when client unsets their preference.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_configure();

        if let Some(window_id) = self.window_id_for_surface(toplevel.wl_surface()) {
            self.decoration_manager
                .write()
                .set_decoration_mode(window_id, crate::decoration::DecorationMode::ServerSide);
        }
    }
}

// Delegate macros
delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_xdg_shell!(State);
delegate_data_device!(State);
smithay::delegate_xdg_decoration!(State);
smithay::delegate_output!(State);

// ============================================================================
// Backend Struct
// ============================================================================

pub struct AxiomSmithayBackendReal {
    pub display: Display<State>,
    pub socket_name: String,
    pub state: State,
    /// The resolved backend kind (winit / drm / noop).
    pub backend_kind: BackendKind,
    pub winit_backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    pub winit_event_loop: Option<WinitEventLoop>,
    /// DRM backend state (scaffolding; `Some` only when `backend_kind == Drm`).
    pub drm_backend: Option<DrmBackend>,
    pub clients: Vec<Client>,
    /// GLES 2.0 shader program for the final fullscreen blit (only GL state kept).
    blit_shader: Option<gl::types::GLuint>,
    /// Persistent GL texture for uploading the WGPU-composed frame.
    /// `(handle, width, height)` — reuses the same texture when dimensions unchanged.
    blit_texture: Option<(gl::types::GLuint, u32, u32)>,
    /// Wayland listening socket — kept alive so clients can connect
    /// (Smithay's display.dispatch_clients polls it internally)
    #[allow(dead_code)]
    listener: Option<ListeningSocket>,
    /// Set to `true` when a decoration button press was consumed (e.g.
    /// close/minimize). The subsequent release event must be consumed too
    /// so Wayland clients don't receive mismatched button-release without
    /// a preceding button-press.
    decoration_consumed_press: bool,
    /// `Some(window_id)` when the user is dragging a window by its titlebar
    /// or resizing it by an edge/corner. While active, pointer motion events
    /// reposition/resize the window and button release commits the change.
    interaction: Option<WindowInteraction>,
}

/// Type of interactive window manipulation in progress.
#[derive(Clone, PartialEq)]
enum WindowInteraction {
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
            xdg_decoration_state: None,
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
            output_scale_factors: HashMap::new(),
            xwm: None,
            decoration_manager: decoration_manager.clone(),
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            configured_sizes: HashMap::new(),
            pending_configure: HashSet::new(),
            popups: HashMap::new(),
            active_popup_grab: None,
            clipboard_cache: None,
            clipboard_source: None,
            cursor_icon: None,
        };

        Ok(Self {
            display,
            socket_name: String::from("axiom-test-dummy"),
            state,
            backend_kind: BackendKind::Noop,
            winit_backend: None,
            winit_event_loop: None,
            drm_backend: None,
            clients: Vec::new(),
            blit_shader: None,
            blit_texture: None,
            listener: None,
            decoration_consumed_press: false,
            interaction: None,
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

        // Parse backend kind from config BEFORE config is moved into State.
        let backend_kind = BackendKind::from_config_str(&config.backend.kind);
        info!("Backend kind: {:?}", backend_kind);

        let display: Display<State> = Display::new()?;
        let dh = display.handle();

        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);

        let xdg_decoration_state = if config.features.enable_xdg_decoration_protocol {
            info!("🌐 Registering zxdg_decoration_manager_v1 global");
            Some(XdgDecorationState::new::<State>(&dh))
        } else {
            None
        };

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
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            None,
        );
        output.create_global::<State>(&dh);

        let state = State {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            xdg_decoration_state,
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
            output_scale_factors: HashMap::new(),
            xwm: None,
            decoration_manager: decoration_manager.clone(),
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            configured_sizes: HashMap::new(),
            pending_configure: HashSet::new(),
            popups: HashMap::new(),
            active_popup_grab: None,
            clipboard_cache: None,
            clipboard_source: None,
            cursor_icon: None,
        };

        let socket_name = format!("wayland-axiom-{}", std::process::id());
        let listener = ListeningSocket::bind(&socket_name)?;
        info!("📡 Wayland socket: {}", socket_name);

        // Initialize DRM backend only when drm kind is selected.
        let drm_backend = match backend_kind {
            BackendKind::Drm => {
                let mut drm = DrmBackend::new();
                let _ = drm.initialize().unwrap_or_else(|e| {
                    warn!(
                        "DRM backend initialization failed: {} — proceeding with DRM stub",
                        e
                    );
                    0
                });
                Some(drm)
            }
            _ => None,
        };

        Ok(Self {
            display,
            socket_name,
            state,
            backend_kind,
            winit_backend: None,
            winit_event_loop: None,
            drm_backend,
            clients: Vec::new(),
            blit_shader: None,
            blit_texture: None,
            listener: Some(listener),
            decoration_consumed_press: false,
            interaction: None,
        })
    }

    /// Initialize the selected backend (winit / drm / noop).
    pub fn initialize(&mut self) -> Result<()> {
        match self.backend_kind {
            BackendKind::Winit => self.initialize_winit(),
            BackendKind::Drm => self.initialize_drm(),
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

    /// Initialize the DRM/KMS backend.
    ///
    /// Opens the DRM device, finds ALL connected displays, sets up GBM-backed
    /// modesetting for each, creates a Smithay `Output` per display with actual
    /// EDID physical dimensions and DPI scale factor, creates a workspace tape
    /// per output, and initialises the libinput + udev hotplug contexts.
    fn initialize_drm(&mut self) -> Result<()> {
        info!("🖥️ Initializing DRM/KMS backend...");

        if let Some(ref mut drm) = self.drm_backend {
            drm.initialize()?;
        } else {
            warn!("DRM backend selected but DrmBackend state is missing — re-probing");
            let mut drm = DrmBackend::new();
            drm.initialize()?;
            self.drm_backend = Some(drm);
        }

        // Replace the hardcoded Output from new() with actual KMS outputs.
        self.state.outputs.clear();

        // Create Smithay Output objects + workspace tapes per KMS output.
        // The first output's dimensions become the primary pointer-clamping
        // viewport; additional outputs extend the available workspace.
        if let Some(ref drm) = self.drm_backend {
            let dh = self.display.handle();
            let outputs = drm.kms_outputs();
            for (i, kms_out) in outputs.iter().enumerate() {
                let output_name = if kms_out.name.is_empty() {
                    format!("Axiom-Output-{}", i)
                } else {
                    kms_out.name.clone()
                };

                let physical_mm_width = kms_out.physical_width_mm as i32;
                let physical_mm_height = kms_out.physical_height_mm as i32;
                let phys_size = if physical_mm_width > 0 && physical_mm_height > 0 {
                    (physical_mm_width, physical_mm_height).into()
                } else {
                    // Fallback for missing EDID: estimate from 96 DPI.
                    (
                        (kms_out.width as i32 * 254 / 960).max(1),
                        (kms_out.height as i32 * 254 / 960).max(1),
                    )
                        .into()
                };

                let output = Output::new(
                    output_name.clone(),
                    PhysicalProperties {
                        size: phys_size,
                        subpixel: Subpixel::Unknown,
                        make: "Axiom".into(),
                        model: format!("Display-{}", i),
                    },
                );
                let mode = OutputMode {
                    size: (kms_out.width as i32, kms_out.height as i32).into(),
                    refresh: kms_out.mode.vrefresh() as i32,
                };
                let scale_i32 = kms_out.scale_factor.round().max(1.0) as i32;
                output.change_current_state(
                    Some(mode),
                    Some(Transform::Normal),
                    Some(Scale::Integer(scale_i32)),
                    None,
                );
                output.create_global::<State>(&dh);

                self.state.outputs.push(output);
                self.state
                    .output_scale_factors
                    .insert(output_name.clone(), kms_out.scale_factor);

                // Create a workspace tape for this output with the
                // correct DPI scale factor.
                {
                    let mut wm = self.state.workspace_manager.write();
                    let tape = wm.ensure_tape(&output_name);
                    tape.set_scale_factor(kms_out.scale_factor);
                }

                info!(
                    "Output '{}': {}x{} @ {:.1}x scale, {}x{}mm physical",
                    output_name,
                    kms_out.width,
                    kms_out.height,
                    kms_out.scale_factor,
                    physical_mm_width,
                    physical_mm_height,
                );
            }

            // Set the primary viewport from the first output.
            if let Some(first) = outputs.first() {
                self.state.window_width = first.width;
                self.state.window_height = first.height;
                self.state
                    .workspace_manager
                    .write()
                    .set_viewport_size(first.width as f64, first.height as f64);
                info!(
                    "Primary display dimensions: {}x{}",
                    first.width, first.height
                );

                // Set the focused output to the first one.
                if !outputs.is_empty() {
                    let first_name = if first.name.is_empty() {
                        "Axiom-Output-0".to_string()
                    } else {
                        first.name.clone()
                    };
                    self.state.workspace_manager.write().focused_output = first_name;
                }
            }
        }

        info!("DRM backend initialized with libinput udev seat discovery");

        // Register a keyboard seat for compatibility even in DRM mode.
        let _keyboard = self
            .state
            .seat
            .add_keyboard(XkbConfig::default(), 200, 200)
            .map_err(|e| anyhow::anyhow!("Failed to add DRM keyboard: {:?}", e))?;

        self.state.seat.add_pointer();

        // Initialize udev DRM hotplug monitor.
        self.init_drm_hotplug_monitor();

        Ok(())
    }

    /// Set up a udev monitor for the "drm" subsystem to detect connector
    /// hotplug events (monitor plugged/unplugged). The monitor FD is
    /// registered with calloop for async notification. When a hotplug
    /// event fires, [`run_one_cycle_drm`] triggers re-enumeration of
    /// outputs without restarting the compositor.
    fn init_drm_hotplug_monitor(&mut self) {
        if let Some(ref mut drm) = self.drm_backend {
            drm.init_udev_monitor();
        }
    }

    /// Synchronize Smithay Output objects and workspace tapes with the
    /// DRM KMS state after a hotplug event.
    ///
    /// Clears all existing outputs and tapes, then re-creates them from
    /// the current KMS state. This is simple and correct even when the
    /// display topology changes dramatically.
    fn call_drm_reenumerate_and_sync(&mut self) {
        let Some(ref drm_backend) = self.drm_backend else {
            return;
        };

        // Re-create compositor outputs from current KMS state.
        self.state.outputs.clear();
        self.state.output_scale_factors.clear();

        let dh = self.display.handle();
        let kms_outputs = drm_backend.kms_outputs();
        for (i, kms_out) in kms_outputs.iter().enumerate() {
            let output_name = if kms_out.name.is_empty() {
                format!("Axiom-Output-{}", i)
            } else {
                kms_out.name.clone()
            };

            let phys_size = if kms_out.physical_width_mm > 0 && kms_out.physical_height_mm > 0 {
                (
                    kms_out.physical_width_mm as i32,
                    kms_out.physical_height_mm as i32,
                )
                    .into()
            } else {
                (
                    (kms_out.width as i32 * 254 / 960).max(1),
                    (kms_out.height as i32 * 254 / 960).max(1),
                )
                    .into()
            };

            let output = Output::new(
                output_name.clone(),
                PhysicalProperties {
                    size: phys_size,
                    subpixel: Subpixel::Unknown,
                    make: "Axiom".into(),
                    model: format!("Display-{}", i),
                },
            );
            let mode = OutputMode {
                size: (kms_out.width as i32, kms_out.height as i32).into(),
                refresh: kms_out.mode.vrefresh() as i32,
            };
            let scale_i32 = kms_out.scale_factor.round().max(1.0) as i32;
            output.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                Some(Scale::Integer(scale_i32)),
                None,
            );
            output.create_global::<State>(&dh);

            self.state.outputs.push(output);
            self.state
                .output_scale_factors
                .insert(output_name.clone(), kms_out.scale_factor);

            let mut wm = self.state.workspace_manager.write();
            let tape = wm.ensure_tape(&output_name);
            tape.set_scale_factor(kms_out.scale_factor);
        }

        // Update primary viewport from first output.
        if let Some(first) = kms_outputs.first() {
            self.state.window_width = first.width;
            self.state.window_height = first.height;
            self.state
                .workspace_manager
                .write()
                .set_viewport_size(first.width as f64, first.height as f64);

            let first_name = if first.name.is_empty() {
                "Axiom-Output-0".to_string()
            } else {
                first.name.clone()
            };
            self.state.workspace_manager.write().focused_output = first_name;
        }

        self.state.needs_redraw = true;
        info!("Output sync complete: {} display(s)", kms_outputs.len());
    }

    /// Run one cycle of the event loop
    pub fn run_one_cycle(&mut self) -> Result<()> {
        match self.backend_kind {
            BackendKind::Winit => self.run_one_cycle_winit()?,
            BackendKind::Drm => self.run_one_cycle_drm()?,
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

        Ok(())
    }

    /// DRM-specific event loop cycle.
    ///
    /// Dispatches the calloop event loop (which polls the DRM FD for
    /// page-flip completion events and the libinput FD for input device
    /// events), then processes any pending events.
    fn run_one_cycle_drm(&mut self) -> Result<()> {
        let drm_available = self
            .drm_backend
            .as_ref()
            .map(|d| d.kms.is_some())
            .unwrap_or(false);

        if !drm_available {
            return Ok(());
        }

        // Dispatch calloop — non-blocking, returns readiness flags.
        // A dispatch error is non-fatal; we log and continue so the
        // compositor can still process Wayland events and render.
        let collector = self
            .drm_backend
            .as_mut()
            .unwrap()
            .dispatch_calloop()
            .unwrap_or_else(|e| {
                warn!("calloop dispatch error: {}", e);
                DrmEventCollector::default()
            });

        // Process DRM page-flip / vblank events
        if collector.drm_ready {
            if let Some(drm) = self.drm_backend.as_mut() {
                match drm.receive_events() {
                    Ok(events) => {
                        let count = events.len();
                        if count > 0 {
                            debug!("DRM: {} event(s) received", count);
                        }
                    }
                    Err(e) => warn!("Error receiving DRM events: {}", e),
                }
            }
        }

        // Process libinput events
        if collector.libinput_ready {
            if let Some(drm) = self.drm_backend.as_mut() {
                let events = drm.dispatch_libinput();
                for ev in events {
                    self.handle_libinput_event(ev);
                }
            }
        }

        // Process udev hotplug events — monitor plugged/unplugged.
        if collector.udev_ready {
            if let Some(drm) = self.drm_backend.as_mut() {
                let hotplug = drm.drain_udev_events();
                if hotplug {
                    info!("🔌 DRM hotplug event — triggering output re-enumeration");
                    match drm.reenumerate_outputs() {
                        Ok((added, removed)) => {
                            if !added.is_empty() || !removed.is_empty() {
                                self.call_drm_reenumerate_and_sync();
                            }
                        }
                        Err(e) => {
                            warn!("DRM output re-enumeration failed: {}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Process a single libinput event and translate it into Smithay seat
    /// events and Axiom InputManager actions.
    fn handle_libinput_event(&mut self, event: input::Event) {
        use input::event::{
            keyboard::KeyboardEventTrait as _, pointer::PointerEventTrait as _, DeviceEvent,
            EventTrait as _, KeyboardEvent, PointerEvent,
        };

        match event {
            input::Event::Device(dev_ev) => match dev_ev {
                DeviceEvent::Added(dev) => {
                    let name = dev.device().name().to_owned();
                    info!("libinput device added: {}", name);
                }
                DeviceEvent::Removed(dev) => {
                    let name = dev.device().name().to_owned();
                    info!("libinput device removed: {}", name);
                }
                _ => {
                    debug!("Unhandled libinput device event");
                }
            },

            input::Event::Keyboard(KeyboardEvent::Key(key_ev)) => {
                if let Some(keyboard) = self.state.seat.get_keyboard() {
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = key_ev.time();
                    let key_code: smithay::input::keyboard::Keycode = key_ev.key().into();
                    let pressed = key_ev.key_state() == input::event::keyboard::KeyState::Pressed;

                    let input_manager = self.state.input_manager.clone();
                    let pending_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                    let pending_clone = pending_actions.clone();

                    let smithay_state = if pressed {
                        smithay::backend::input::KeyState::Pressed
                    } else {
                        smithay::backend::input::KeyState::Released
                    };

                    keyboard.input::<(), _>(
                        &mut self.state,
                        key_code,
                        smithay_state,
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
                                        debug!("⌨️ DRM global shortcut: {}", key_combo);
                                        *pending_clone.borrow_mut() = actions;
                                        return smithay::input::keyboard::FilterResult::Intercept(
                                            (),
                                        );
                                    }
                                }
                            }
                            smithay::input::keyboard::FilterResult::Forward
                        },
                    );

                    let actions: Vec<_> = pending_actions.borrow_mut().drain(..).collect();
                    if !actions.is_empty() {
                        self.process_actions(actions);
                    }
                }
            }

            input::Event::Pointer(pointer_ev) => match pointer_ev {
                PointerEvent::Motion(motion) => {
                    let dx = motion.dx();
                    let dy = motion.dy();
                    self.state.pointer_x =
                        (self.state.pointer_x + dx).clamp(0.0, f64::from(self.state.window_width));
                    self.state.pointer_y =
                        (self.state.pointer_y + dy).clamp(0.0, f64::from(self.state.window_height));

                    // Interactive move/resize consumes the motion event.
                    if self.handle_interaction(self.state.pointer_x, self.state.pointer_y) {
                        return;
                    }

                    let serial = SERIAL_COUNTER.next_serial();
                    let time = motion.time();

                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let floating = self.floating_rects();
                        let under = self.state.workspace_manager.read().element_under(
                            self.state.pointer_x,
                            self.state.pointer_y,
                            &floating,
                        );
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
                                .map(|surface| (surface, smithay::utils::Point::from((sx, sy))))
                        });

                        let motion_event = smithay::input::pointer::MotionEvent {
                            serial,
                            time,
                            location: smithay::utils::Point::from((
                                self.state.pointer_x,
                                self.state.pointer_y,
                            )),
                        };
                        pointer.motion(&mut self.state, focus, &motion_event);
                    }
                }

                PointerEvent::MotionAbsolute(abs) => {
                    let x = abs.absolute_x_transformed(self.state.window_width);
                    let y = abs.absolute_y_transformed(self.state.window_height);
                    self.state.pointer_x = x;
                    self.state.pointer_y = y;

                    // Interactive move/resize consumes the motion event.
                    if self.handle_interaction(x, y) {
                        return;
                    }

                    let serial = SERIAL_COUNTER.next_serial();
                    let time = abs.time();

                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let floating = self.floating_rects();
                        let under = self
                            .state
                            .workspace_manager
                            .read()
                            .element_under(x, y, &floating);
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
                                .map(|surface| (surface, smithay::utils::Point::from((sx, sy))))
                        });

                        let motion_event = smithay::input::pointer::MotionEvent {
                            serial,
                            time,
                            location: smithay::utils::Point::from((x, y)),
                        };
                        pointer.motion(&mut self.state, focus, &motion_event);
                    }
                }

                PointerEvent::Button(btn) => {
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = btn.time();
                    let button = btn.button();
                    let pressed = btn.button_state() == input::event::pointer::ButtonState::Pressed;
                    let smithay_state = if pressed {
                        smithay::backend::input::ButtonState::Pressed
                    } else {
                        smithay::backend::input::ButtonState::Released
                    };

                    // Dismiss active popup grab on any button press outside the popup
                    if pressed {
                        if let Some(popup_id) = self.state.active_popup_grab {
                            let dismiss = if let Some(p) = self.state.popups.get(&popup_id) {
                                let px = self.state.pointer_x as i32;
                                let py = self.state.pointer_y as i32;
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
                    }

                    // Decoration hit-testing: close/minimize/maximize buttons
                    // on server-side decorations. When consumed the event must
                    // NOT be forwarded to Smithay (no unmatched button-release
                    // delivered to Wayland clients).
                    if pressed {
                        if self.handle_decoration_button(
                            self.state.pointer_x,
                            self.state.pointer_y,
                            true,
                        ) {
                            // Release the consumed state so the next
                            // unrelated button press starts clean.
                            self.decoration_consumed_press = false;
                            return;
                        }
                    } else if self.decoration_consumed_press {
                        self.handle_decoration_button(
                            self.state.pointer_x,
                            self.state.pointer_y,
                            false,
                        );
                        self.decoration_consumed_press = false;
                        return;
                    }

                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let button_event = smithay::input::pointer::ButtonEvent {
                            serial,
                            time,
                            button,
                            state: smithay_state,
                        };
                        pointer.button(&mut self.state, &button_event);
                    }
                }

                PointerEvent::ScrollWheel(scroll) => {
                    use input::event::pointer::PointerScrollEvent as _;
                    let time = scroll.time();

                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let mut axis_frame = smithay::input::pointer::AxisFrame::new(time);

                        if scroll.has_axis(input::event::pointer::Axis::Horizontal) {
                            let amount =
                                scroll.scroll_value(input::event::pointer::Axis::Horizontal);
                            if amount.abs() > 0.0 {
                                axis_frame = axis_frame
                                    .value(smithay::backend::input::Axis::Horizontal, amount);
                            }
                        }
                        if scroll.has_axis(input::event::pointer::Axis::Vertical) {
                            let amount = scroll.scroll_value(input::event::pointer::Axis::Vertical);
                            if amount.abs() > 0.0 {
                                axis_frame = axis_frame
                                    .value(smithay::backend::input::Axis::Vertical, amount);
                            }
                        }

                        pointer.axis(&mut self.state, axis_frame);
                        pointer.frame(&mut self.state);
                    }
                }

                PointerEvent::ScrollFinger(scroll) => {
                    use input::event::pointer::PointerScrollEvent as _;
                    let time = scroll.time();

                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let mut axis_frame = smithay::input::pointer::AxisFrame::new(time);

                        if scroll.has_axis(input::event::pointer::Axis::Horizontal) {
                            let amount =
                                scroll.scroll_value(input::event::pointer::Axis::Horizontal);
                            if amount.abs() > 0.0 {
                                axis_frame = axis_frame
                                    .value(smithay::backend::input::Axis::Horizontal, amount);
                            }
                        }
                        if scroll.has_axis(input::event::pointer::Axis::Vertical) {
                            let amount = scroll.scroll_value(input::event::pointer::Axis::Vertical);
                            if amount.abs() > 0.0 {
                                axis_frame = axis_frame
                                    .value(smithay::backend::input::Axis::Vertical, amount);
                            }
                        }

                        pointer.axis(&mut self.state, axis_frame);
                        pointer.frame(&mut self.state);
                    }
                }

                PointerEvent::ScrollContinuous(scroll) => {
                    use input::event::pointer::PointerScrollEvent as _;
                    let time = scroll.time();

                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let mut axis_frame = smithay::input::pointer::AxisFrame::new(time);

                        if scroll.has_axis(input::event::pointer::Axis::Horizontal) {
                            let amount =
                                scroll.scroll_value(input::event::pointer::Axis::Horizontal);
                            if amount.abs() > 0.0 {
                                axis_frame = axis_frame
                                    .value(smithay::backend::input::Axis::Horizontal, amount);
                            }
                        }
                        if scroll.has_axis(input::event::pointer::Axis::Vertical) {
                            let amount = scroll.scroll_value(input::event::pointer::Axis::Vertical);
                            if amount.abs() > 0.0 {
                                axis_frame = axis_frame
                                    .value(smithay::backend::input::Axis::Vertical, amount);
                            }
                        }

                        pointer.axis(&mut self.state, axis_frame);
                        pointer.frame(&mut self.state);
                    }
                }

                _ => {
                    debug!("Unhandled libinput pointer event");
                }
            },

            other => {
                debug!("Unhandled libinput event: {:?}", other);
            }
        }
    }

    /// Common post-event dispatch for all backends.
    fn run_one_cycle_common(&mut self) -> Result<()> {
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

        // Render if needed — dispatch based on backend kind.
        // Winit uses the GL render path; DRM uses GBM surface → set_crtc.
        if self.state.needs_redraw {
            match self.backend_kind {
                BackendKind::Drm => {
                    if let Some(ref mut drm) = self.drm_backend {
                        let rendered = drm.render_frame();
                        if rendered == 0 {
                            debug!("DRM render_frame: no outputs rendered (dumb-buffer only)");
                        }
                    }
                }
                _ => {
                    self.render()?;
                }
            }
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

                // Interactive move/resize consumes the motion event.
                if self.handle_interaction(x, y) {
                    return;
                }

                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                if let Some(pointer) = self.state.seat.get_pointer() {
                    // Find the surface under the pointer and forward motion
                    // Skip dead surfaces (from disconnected clients)
                    let floating = self.floating_rects();
                    let under = self
                        .state
                        .workspace_manager
                        .read()
                        .element_under(x, y, &floating);
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

                let pressed = event.state() == smithay::backend::input::ButtonState::Pressed;

                // Decoration hit-testing: close/minimize/maximize buttons
                // on server-side decorations.
                if pressed {
                    if self.handle_decoration_button(
                        self.state.pointer_x,
                        self.state.pointer_y,
                        true,
                    ) {
                        self.decoration_consumed_press = false;
                        return;
                    }
                } else if self.decoration_consumed_press {
                    self.handle_decoration_button(
                        self.state.pointer_x,
                        self.state.pointer_y,
                        false,
                    );
                    self.decoration_consumed_press = false;
                    return;
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

                    // Workspace navigation via scroll.
                    // Smooth scroll sources (touchpad) feed velocity into momentum physics;
                    // discrete sources (mouse wheel) snap to adjacent columns.
                    let source = event.source();
                    match source {
                        AxisSource::Continuous | AxisSource::Finger => {
                            if let Some(amount) = event.amount(Axis::Horizontal) {
                                let speed = self.state.config.workspace.scroll_speed;
                                let velocity = amount * speed * 8.0;
                                if velocity.abs() > 0.0 {
                                    self.state
                                        .workspace_manager
                                        .write()
                                        .start_momentum_scroll(velocity);
                                    self.state.needs_redraw = true;
                                }
                            }
                        }
                        AxisSource::Wheel | AxisSource::WheelTilt => {
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
                }
            }

            _ => {}
        }
    }

    /// If an interactive window manipulation is active (move or resize),
    /// apply the new pointer position and return `true` so the motion
    /// event is NOT forwarded to Smithay for pointer focus updates.
    fn handle_interaction(&mut self, px: f64, py: f64) -> bool {
        let interaction = match self.interaction {
            Some(ref i) => i.clone(),
            None => return false,
        };
        match interaction {
            WindowInteraction::Move {
                window_id,
                offset_x,
                offset_y,
            } => {
                let new_x = (px - offset_x).round() as i32;
                let new_y = (py - offset_y).round() as i32;
                let mut wm = self.state.window_manager.write();
                if let Some(w) = wm.get_window_mut(window_id) {
                    w.window.set_position(new_x, new_y);
                    self.state.needs_redraw = true;
                }
            }
            WindowInteraction::Resize {
                window_id,
                edge,
                initial_rect: (ix, iy, iw, ih),
                start_x,
                start_y,
            } => {
                let dx = (px - start_x) as i32;
                let dy = (py - start_y) as i32;
                let mut wm = self.state.window_manager.write();
                if let Some(w) = wm.get_window_mut(window_id) {
                    use crate::decoration::ResizeEdge;
                    let (new_x, new_y, new_w, new_h) = match edge {
                        ResizeEdge::Right => (ix, iy, (iw as i32 + dx).max(100) as u32, ih),
                        ResizeEdge::Bottom => (ix, iy, iw, (ih as i32 + dy).max(100) as u32),
                        ResizeEdge::BottomRight => {
                            let w = (iw as i32 + dx).max(100) as u32;
                            let h = (ih as i32 + dy).max(100) as u32;
                            (ix, iy, w, h)
                        }
                        // Left, Top, TopLeft, TopRight, BottomLeft are not yet
                        // covered by decoration hit-testing — placeholder.
                        _ => (ix, iy, iw, ih),
                    };
                    w.window.position = (new_x, new_y);
                    w.window.size = (new_w, new_h);
                    self.state.needs_redraw = true;
                }
            }
        }
        true
    }

    /// Build a list of floating window rects for pointer hit-testing.
    /// Each entry is `(window_id, x, y, width, height)`. Called on every
    /// motion and button event so `element_under` can find floating windows.
    fn floating_rects(&self) -> Vec<(u64, i32, i32, u32, u32)> {
        let wm = self.state.window_manager.read();
        let mut rects = Vec::new();
        for id in self.state.workspace_manager.read().floating_window_ids() {
            if let Some(w) = wm.get_window(id) {
                if !w.properties.minimized {
                    rects.push((
                        id,
                        w.window.position.0,
                        w.window.position.1,
                        w.window.size.0,
                        w.window.size.1,
                    ));
                }
            }
        }
        rects
    }

    /// Decoration hit-testing for pointer button events. Returns `true` if
    /// the button press was consumed by a decoration (close/minimize/etc.),
    /// in which case the caller should **not** forward the event to Smithay's
    /// `PointerHandle::button`. On release the decoration pressed states are
    /// cleared regardless, but the `decoration_consumed_press` flag is also
    /// consulted to decide whether to forward the release to Smithay.
    fn handle_decoration_button(&mut self, pointer_x: f64, pointer_y: f64, pressed: bool) -> bool {
        if pressed {
            // Find the window under the cursor.
            let floating = self.floating_rects();
            let under = self
                .state
                .workspace_manager
                .read()
                .element_under(pointer_x, pointer_y, &floating);
            let Some((window_id, _)) = under else {
                return false;
            };
            // Compute window-relative coordinates for decoration hit-testing.
            let rel = self
                .state
                .window_manager
                .read()
                .get_window(window_id)
                .map(|w| {
                    let rx = (pointer_x - w.window.position.0 as f64) as i32;
                    let ry = (pointer_y - w.window.position.1 as f64) as i32;
                    (rx, ry)
                });
            let Some((rx, ry)) = rel else {
                return false;
            };
            let action = self
                .state
                .decoration_manager
                .write()
                .handle_button_press(window_id, rx, ry);
            match action {
                Some(crate::decoration::DecorationAction::Close) => {
                    if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                        self.state.destroy_window(surface_id);
                        self.state.needs_redraw = true;
                    }
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::Minimize) => {
                    let is_minimized = self.state.window_manager.read().is_minimized(window_id);
                    if is_minimized {
                        self.state
                            .workspace_manager
                            .write()
                            .restore_window(window_id);
                        self.state.window_manager.write().restore_window(window_id);
                        self.state
                            .effects_engine
                            .write()
                            .animate_window_restore(window_id);
                    } else {
                        self.state
                            .workspace_manager
                            .write()
                            .minimize_window(window_id);
                        self.state.window_manager.write().minimize_window(window_id);
                        self.state
                            .effects_engine
                            .write()
                            .animate_window_minimize(window_id);
                    }
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::ToggleMaximize) => {
                    self.state
                        .window_manager
                        .write()
                        .toggle_fullscreen(window_id);
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::StartMove) => {
                    // Enter interactive move mode: set the window as floating,
                    // record the pointer offset and enter grab-like state.
                    self.state
                        .workspace_manager
                        .write()
                        .set_window_floating(window_id, true);
                    let wm = self.state.window_manager.read();
                    if let Some(w) = wm.get_window(window_id) {
                        let offset_x = pointer_x - w.window.position.0 as f64;
                        let offset_y = pointer_y - w.window.position.1 as f64;
                        self.interaction = Some(WindowInteraction::Move {
                            window_id,
                            offset_x,
                            offset_y,
                        });
                    }
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::StartResize(edge)) => {
                    // Enter interactive resize mode. Set the window as
                    // floating so the layout system doesn't overwrite the
                    // custom size each frame.
                    self.state
                        .workspace_manager
                        .write()
                        .set_window_floating(window_id, true);
                    let wm = self.state.window_manager.read();
                    if let Some(w) = wm.get_window(window_id) {
                        let (ix, iy) = w.window.position;
                        let (iw, ih) = w.window.size;
                        self.interaction = Some(WindowInteraction::Resize {
                            window_id,
                            edge,
                            initial_rect: (ix, iy, iw, ih),
                            start_x: pointer_x,
                            start_y: pointer_y,
                        });
                    }
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                None => {}
            }
            // If no decoration button matched, check for edge-resize on tiled
            // / floating windows. A click within `RESIZE_HANDLE` pixels of the
            // window's right or bottom edge starts a resize (bottom-right
            // corner is the most natural resize affordance).
            {
                const RESIZE_HANDLE: i32 = 8;
                let (window_id, _) = match under {
                    Some(t) => t,
                    None => return false,
                };
                // Compute window-relative coordinates for edge hit-testing.
                let rel = self
                    .state
                    .window_manager
                    .read()
                    .get_window(window_id)
                    .map(|w| {
                        let rx = (pointer_x - w.window.position.0 as f64) as i32;
                        let ry = (pointer_y - w.window.position.1 as f64) as i32;
                        (rx, ry, w.window.size.0 as i32, w.window.size.1 as i32)
                    });
                let Some((rx, ry, ww, wh)) = rel else {
                    return false;
                };
                use crate::decoration::ResizeEdge;
                let edge = if rx >= ww - RESIZE_HANDLE && ry >= wh - RESIZE_HANDLE {
                    Some(ResizeEdge::BottomRight)
                } else if rx >= ww - RESIZE_HANDLE {
                    Some(ResizeEdge::Right)
                } else if ry >= wh - RESIZE_HANDLE {
                    Some(ResizeEdge::Bottom)
                } else {
                    None
                };
                if let Some(edge) = edge {
                    // Set as floating so the layout doesn't overwrite size.
                    self.state
                        .workspace_manager
                        .write()
                        .set_window_floating(window_id, true);
                    let (ix, iy) = (pointer_x - rx as f64, pointer_y - ry as f64);
                    let (ix, iy) = (ix as i32, iy as i32);
                    let (iw, ih) = (ww as u32, wh as u32);
                    self.interaction = Some(WindowInteraction::Resize {
                        window_id,
                        edge,
                        initial_rect: (ix, iy, iw, ih),
                        start_x: pointer_x,
                        start_y: pointer_y,
                    });
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
            }
            false
        } else {
            // Release: clear decoration visual state AND stop any interaction.
            let floating = self.floating_rects();
            let under = self
                .state
                .workspace_manager
                .read()
                .element_under(pointer_x, pointer_y, &floating);
            if let Some((window_id, _)) = under {
                let rel = self
                    .state
                    .window_manager
                    .read()
                    .get_window(window_id)
                    .map(|w| {
                        let rx = (pointer_x - w.window.position.0 as f64) as i32;
                        let ry = (pointer_y - w.window.position.1 as f64) as i32;
                        (rx, ry)
                    });
                if let Some((rx, ry)) = rel {
                    self.state
                        .decoration_manager
                        .write()
                        .handle_button_release(window_id, rx, ry);
                }
            }
            // If an interactive move/resize was in progress, finalize it.
            if let Some(interaction) = self.interaction.take() {
                // For resize, send a configure event so the client resizes
                // its buffer to match the new dimensions.
                if let WindowInteraction::Resize { window_id, .. } = interaction {
                    if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                        if let Some(toplevel) = self.state.toplevels.get(&surface_id) {
                            let size = self
                                .state
                                .window_manager
                                .read()
                                .get_window(window_id)
                                .map(|w| w.window.size);
                            if let Some((new_w, new_h)) = size {
                                // Convert physical-pixel window size to
                                // logical pixels for the configure event,
                                // matching the tiling reconfigure path.
                                let scale = self.state.focused_output_scale();
                                let logical_w = ((new_w as f64 / scale).round() as i32).max(1);
                                let logical_h = ((new_h as f64 / scale).round() as i32).max(1);
                                toplevel.with_pending_state(|state| {
                                    state.size = Some((logical_w, logical_h).into());
                                });
                                toplevel.send_configure();
                                self.state
                                    .configured_sizes
                                    .insert(surface_id, (logical_w, logical_h));
                            }
                        }
                    }
                }
                self.decoration_consumed_press = true;
                return true;
            }
            // Consume the release if the press was also consumed, so
            // Wayland clients never see an unmatched button-release.
            self.decoration_consumed_press
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
                CompositorAction::ToggleFloating => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        self.state.window_manager.write().toggle_floating(window_id);
                        self.state
                            .workspace_manager
                            .write()
                            .toggle_window_floating(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::ToggleMinimize => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        let is_minimized = self.state.window_manager.read().is_minimized(window_id);
                        if is_minimized {
                            self.state
                                .workspace_manager
                                .write()
                                .restore_window(window_id);
                            self.state.window_manager.write().restore_window(window_id);
                            self.state
                                .effects_engine
                                .write()
                                .animate_window_restore(window_id);
                            info!("🔲 Input: Restored window {}", window_id);
                        } else {
                            self.state
                                .workspace_manager
                                .write()
                                .minimize_window(window_id);
                            self.state.window_manager.write().minimize_window(window_id);
                            self.state
                                .effects_engine
                                .write()
                                .animate_window_minimize(window_id);
                            info!("🔳 Input: Minimized window {}", window_id);
                        }
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
                CompositorAction::ToggleEffects => {
                    let mut ee = self.state.effects_engine.write();
                    let new_enabled = !ee.is_enabled();
                    ee.apply_live_effects_control(Some(new_enabled), None, None);
                    info!(
                        "✨ Toggle effects: {}",
                        if new_enabled { "enabled" } else { "disabled" }
                    );
                    self.state.needs_redraw = true;
                }
                CompositorAction::LaunchTerminal => {
                    let _ = std::process::Command::new("xterm")
                        .spawn()
                        .map(|_| debug!("🚀 Launched terminal"))
                        .map_err(|e| warn!("Failed to launch terminal: {}", e));
                }
                CompositorAction::LaunchLauncher => {
                    let _ = std::process::Command::new("dmenu_run")
                        .spawn()
                        .map(|_| debug!("🚀 Launched launcher"))
                        .map_err(|e| warn!("Failed to launch launcher: {}", e));
                }
                CompositorAction::Custom(ref cmd) => {
                    // Split the command string on whitespace for the
                    // program name and arguments.
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.is_empty() {
                        return;
                    }
                    let _ = std::process::Command::new(parts[0])
                        .args(&parts[1..])
                        .spawn()
                        .map(|_| debug!("🚀 Launched custom command: {}", cmd))
                        .map_err(|e| warn!("Failed to launch '{}': {}", cmd, e));
                }
            }
        }
    }

    /// Render the current frame via WGPU compositing + single GL blit.
    ///
    /// All window compositing and effects happen on the GPU via WGPU's
    /// `compose_full_frame`. The result is uploaded as a single GL texture
    /// and blitted to the framebuffer with one fullscreen quad draw.
    fn render(&mut self) -> Result<()> {
        let Some(backend) = self.winit_backend.as_mut() else {
            return Ok(());
        };

        // Apply the latest cursor icon to the winit window.
        if let Some(icon) = self.state.cursor_icon {
            backend.window().set_cursor(icon);
        }

        let ww = self.state.window_width;
        let wh = self.state.window_height;

        // Calculate layouts from workspace manager (animations already updated in run_one_cycle)
        let layouts = self
            .state
            .workspace_manager
            .read()
            .calculate_workspace_layouts();

        // Update tiled window positions in window manager.
        // Floating windows keep their manually-set position.
        {
            let mut wm = self.state.window_manager.write();
            for (window_id, layout_rect) in &layouts {
                if let Some(window) = wm.get_window_mut(*window_id) {
                    if !window.properties.floating {
                        window.window.set_position(layout_rect.x, layout_rect.y);
                        window
                            .window
                            .set_size(layout_rect.width, layout_rect.height);
                    }
                }
            }
        }

        // Send xdg_toplevel.configure events for windows whose tiling size changed.
        // This notifies Wayland clients to resize their buffers to match the layout.
        // Done before bind() so we don't hold GL context while doing Wayland protocol work.
        //
        // Physical-pixel layout dims are converted to logical pixels using the
        // focused output's DPI scale factor. HiDPI-aware clients multiply by
        // buffer_scale to allocate their actual pixel buffers.
        let scale = self.state.focused_output_scale();
        for (window_id, rect) in &layouts {
            if let Some(&surface_id) = self.state.window_map.get(window_id) {
                if let Some(toplevel) = self.state.toplevels.get(&surface_id) {
                    let new_w = ((rect.width as f64 / scale).round() as i32).max(1);
                    let new_h = ((rect.height as f64 / scale).round() as i32).max(1);

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

        // ── WGPU Compositing ────────────────────────────────────────────
        // Drain the SHM buffer cache and upload each entry to the renderer
        // as WGPU textures. This replaces the old GL texture upload path.
        if let Some(ref renderer) = self.state.renderer {
            let mut r = renderer.write();

            // Build inverse map: surface_id → window_id for texture uploads.
            let mut surface_to_window: HashMap<u32, u64> = HashMap::new();
            for (&wid, &sid) in &self.state.window_map {
                surface_to_window.insert(sid, wid);
            }

            // Drain buffer_cache and upload to renderer as WGPU textures.
            let pending: Vec<(u32, Vec<u8>, (i32, i32))> = {
                let cache = &mut self.state.buffer_cache;
                let dims = &mut self.state.buffer_cache_dimensions;
                let pending: Vec<_> = cache
                    .drain()
                    .filter_map(|(sid, data)| {
                        let d = dims.remove(&sid).unwrap_or((0, 0));
                        if d.0 > 0 && d.1 > 0 {
                            Some((sid, data, d))
                        } else {
                            None
                        }
                    })
                    .collect();
                pending
            };

            for (surface_id, data, (w, h)) in &pending {
                if let Some(&window_id) = surface_to_window.get(surface_id) {
                    r.update_window_texture(window_id, *w as u32, *h as u32, data);
                }
            }

            // Sync window positions and sizes from layouts into the renderer.
            // The compositor's prepare_frame_data() applies effects (scale,
            // opacity, offset) and upserts rects, but the backend also needs
            // to sync tiled layouts when the backend owns position updates.
            for (window_id, rect) in &layouts {
                let x = rect.x as f32;
                let y = rect.y as f32;
                let w = rect.width as f32;
                let h = rect.height as f32;
                r.upsert_window_rect(*window_id, (x, y), (w, h), 1.0);
            }

            // Add floating windows to the renderer's window list.
            {
                let wm = self.state.window_manager.read();
                for &window_id in self
                    .state
                    .workspace_manager
                    .read()
                    .floating_window_ids()
                    .iter()
                {
                    if let Some(w) = wm.get_window(window_id) {
                        let fx = w.window.position.0 as f32;
                        let fy = w.window.position.1 as f32;
                        let fw = w.window.size.0 as f32;
                        let fh = w.window.size.1 as f32;
                        r.upsert_window_rect(window_id, (fx, fy), (fw, fh), 1.0);
                    }
                }
            }

            // Add popups as temporary render entries (high IDs to avoid conflicts).
            // Popups are drawn as textured quads like windows.
            let popup_ids: Vec<u32> = self.state.popups.keys().copied().collect();
            for popup_id in &popup_ids {
                let popup = &self.state.popups[popup_id];
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
                let popup_w = popup.width.max(1) as f32;
                let popup_h = popup.height.max(1) as f32;
                // Use a high ID offset to avoid conflicts with real window IDs.
                let popup_render_id = 0x8000_0000 + *popup_id as u64;
                r.upsert_window_rect(
                    popup_render_id,
                    (popup_x as f32, popup_y as f32),
                    (popup_w, popup_h),
                    1.0,
                );
                // Upload popup texture if available in the buffer cache.
                // (Popup SHM data goes through the same commit handler as windows.)
            }

            // Compose the full frame via WGPU (windows + effects).
            match r.compose_full_frame(ww, wh) {
                Ok(composed) => {
                    drop(r); // Release renderer lock before GL work

                    // ── GL Blit: upload WGPU result and draw fullscreen quad ──
                    // SAFETY: GL context is current (backend.bind() succeeded above).
                    backend.bind()?; // Re-acquire GL context

                    // Lazily compile the blit shader.
                    let shader = unsafe { ensure_blit_shader(&mut self.blit_shader) };

                    // Upload composed frame to persistent GL texture.
                    let tex =
                        unsafe { update_blit_texture(&mut self.blit_texture, ww, wh, &composed) };

                    // Clear and blit.
                    // SAFETY: GL context is current after backend.bind().
                    unsafe {
                        gl::ClearColor(0.08, 0.08, 0.12, 1.0);
                        gl::Clear(gl::COLOR_BUFFER_BIT);
                        draw_blit_quad(shader, tex);
                    }

                    // Remove temporary popup entries from renderer.
                    if let Some(ref renderer) = self.state.renderer {
                        let mut r = renderer.write();
                        for popup_id in &popup_ids {
                            let popup_render_id = 0x8000_0000 + *popup_id as u64;
                            r.remove_window(popup_render_id);
                        }
                    }
                }
                Err(e) => {
                    warn!("⚠️ WGPU compose failed: {}", e);
                    drop(r);

                    // Fallback: clear to dark background
                    // SAFETY: GL context is current after backend.bind().
                    backend.bind()?;
                    unsafe {
                        gl::ClearColor(0.08, 0.08, 0.12, 1.0);
                        gl::Clear(gl::COLOR_BUFFER_BIT);
                    }
                }
            }
        }

        backend.submit(None)?;

        debug!(
            "🎨 Rendered {} windows at {}x{} (WGPU compositing)",
            layouts.len(),
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

        // Free the persistent blit texture and shader that exist outside
        // per-frame management.
        match self.backend_kind {
            BackendKind::Winit => {
                if let Some(backend) = self.winit_backend.as_mut() {
                    // Try rebinding; failure during shutdown is non-fatal.
                    if backend.bind().is_ok() {
                        if let Some((tex, _, _)) = self.blit_texture.take() {
                            // SAFETY: GL context is current after the rebind above.
                            unsafe {
                                gl::DeleteTextures(1, &tex);
                            }
                        }
                        if let Some(prog) = self.blit_shader.take() {
                            unsafe {
                                gl::DeleteProgram(prog);
                            }
                        }
                    }
                }
            }
            BackendKind::Drm => {
                if let Some(ref mut drm) = self.drm_backend {
                    let _ = drm.shutdown();
                }
            }
            BackendKind::Noop => {
                // Noop shutdown: nothing to clean up.
            }
        }

        Ok(())
    }
}

// ── GL Blit Helpers ────────────────────────────────────────────────────────
// Minimal GL code for the final fullscreen blit. The textured-quad shader is
// compiled once and cached in `AxiomSmithayBackendReal::blit_shader`.

/// Compile the minimal GLES 2.0 textured-quad shader used for the final
/// fullscreen blit. Returns the cached program on subsequent calls.
///
/// # Safety
/// Caller must ensure a GL context is current.
unsafe fn ensure_blit_shader(shader: &mut Option<gl::types::GLuint>) -> Option<gl::types::GLuint> {
    if let Some(prog) = *shader {
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

    unsafe fn compile_shader(ty: gl::types::GLenum, src: &str) -> Option<gl::types::GLuint> {
        let s = gl::CreateShader(ty);
        if s == 0 {
            return None;
        }
        gl::ShaderSource(
            s,
            1,
            &(src.as_ptr() as *const gl::types::GLchar),
            &(src.len() as gl::types::GLint),
        );
        gl::CompileShader(s);
        let mut ok: gl::types::GLint = 0;
        gl::GetShaderiv(s, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            gl::DeleteShader(s);
            return None;
        }
        Some(s)
    }

    let vs = compile_shader(gl::VERTEX_SHADER, vert_src);
    let fs = compile_shader(gl::FRAGMENT_SHADER, frag_src);

    if let (Some(vs), Some(fs)) = (vs, fs) {
        let prog = gl::CreateProgram();
        gl::AttachShader(prog, vs);
        gl::AttachShader(prog, fs);
        gl::LinkProgram(prog);
        let mut linked: gl::types::GLint = 0;
        gl::GetProgramiv(prog, gl::LINK_STATUS, &mut linked);
        if linked != 0 {
            *shader = Some(prog);
        }
        gl::DeleteShader(vs);
        gl::DeleteShader(fs);
        return *shader;
    }

    if let Some(v) = vs {
        gl::DeleteShader(v);
    }
    if let Some(f) = fs {
        gl::DeleteShader(f);
    }
    None
}

/// Upload RGBA data to a persistent GL texture, reusing it when dimensions match.
///
/// # Safety
/// Caller must ensure a GL context is current.
unsafe fn update_blit_texture(
    cache: &mut Option<(gl::types::GLuint, u32, u32)>,
    width: u32,
    height: u32,
    data: &[u8],
) -> gl::types::GLuint {
    unsafe {
        if cache.is_none() {
            let mut t: gl::types::GLuint = 0;
            gl::GenTextures(1, &mut t);
            gl::BindTexture(gl::TEXTURE_2D, t);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            *cache = Some((t, 0, 0));
        }

        let (tex, old_w, old_h) = cache.as_mut().expect("just initialized");

        gl::BindTexture(gl::TEXTURE_2D, *tex);
        if *old_w == width && *old_h == height {
            gl::TexSubImage2D(
                gl::TEXTURE_2D,
                0,
                0,
                0,
                width as i32,
                height as i32,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const std::ffi::c_void,
            );
        } else {
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                width as i32,
                height as i32,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const std::ffi::c_void,
            );
            *old_w = width;
            *old_h = height;
        }
        gl::BindTexture(gl::TEXTURE_2D, 0);
        *tex
    }
}

/// Draw a fullscreen textured quad for the final blit.
///
/// # Safety
/// Caller must ensure a GL context is current and `tex` is a valid GL texture.
unsafe fn draw_blit_quad(shader: Option<gl::types::GLuint>, tex: gl::types::GLuint) {
    let Some(prog) = shader else {
        return;
    };

    #[rustfmt::skip]
    let vertices: [f32; 16] = [
        -1.0,  1.0, 0.0, 1.0,
         1.0,  1.0, 1.0, 1.0,
        -1.0, -1.0, 0.0, 0.0,
         1.0, -1.0, 1.0, 0.0,
    ];

    unsafe {
        gl::UseProgram(prog);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D, tex);

        let pos_loc = gl::GetAttribLocation(prog, c"a_position".as_ptr());
        let tex_loc = gl::GetAttribLocation(prog, c"a_texcoord".as_ptr());

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
