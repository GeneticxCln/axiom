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
use crate::input::InputManager;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;
use anyhow::Result;
use log::{debug, info, warn};

use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};
use std::collections::{HashMap, HashSet};
use std::os::unix::io::OwnedFd;
use std::sync::{mpsc, Arc};

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

use smithay::{
    backend::{
        input::{
            InputEvent,
        },
        renderer::{gles::{GlesRenderer, GlesTexture}, element::texture::TextureBuffer, utils::on_commit_buffer_handler},
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    delegate_compositor, delegate_data_device, delegate_foreign_toplevel_list, delegate_seat,
    delegate_shm, delegate_xdg_shell,
     input::{
          keyboard::{XkbConfig},
          pointer::{CursorIcon, CursorImageStatus},
          Seat, SeatHandler, SeatState,
      },
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    reexports::wayland_server::{protocol::wl_seat, Display, DisplayHandle, ListeningSocket},
    utils::{Serial, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            with_states, BufferAssignment, CompositorClientState, CompositorHandler,
            CompositorState, SurfaceAttributes,
        },
        fractional_scale::{self, FractionalScaleHandler, FractionalScaleManagerState},
        output::OutputHandler,
        selection::{
            data_device::{
                request_data_device_client_selection, set_data_device_focus,
                ClientDndGrabHandler, DataDeviceHandler,
                DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler, SelectionSource, SelectionTarget,
        },
        shell::xdg::{
            decoration::{XdgDecorationHandler, XdgDecorationState},
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
        shm::{with_buffer_contents, ShmHandler, ShmState},
    },
};

use wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason, ObjectId},
    protocol::{
        wl_buffer, wl_data_source::WlDataSource, wl_output::WlOutput,
        wl_surface::WlSurface,
    },
    Client, Resource,
};

use wayland_protocols::xdg::shell::server::xdg_toplevel;

// Type alias to reduce complexity of the Rc<RefCell<Option<...>>> pattern
// used for passing buffer data out of the SHM commit closure.
type CachedBufferData = std::rc::Rc<std::cell::RefCell<Option<(Vec<u8>, i32, i32)>>>;
type ClipboardUpdate = Vec<u8>;

/// Server-side decorations are rendered via the GLES solid-color pipeline
/// (and text when system fonts are available). Title text rendering falls back gracefully
/// when system fonts are unavailable (titlebars still render with solid colors
/// and buttons).
fn backend_prefers_server_side_decorations() -> bool {
    true
}

/// The compositor now renders visible SSD decoration quads (titlebar
/// backgrounds and buttons) and title text (when system fonts are available).
/// Negotiate server-side decorations with clients that request them.
fn negotiated_xdg_decoration_mode() -> Mode {
    Mode::ServerSide
}

// Submodules split out of this file for maintainability. Each is a child of
// `backend`, so it can read the private fields of `State` and
// `AxiomSmithayBackendReal` (descendant modules see ancestor privates).
mod render;
mod input;
mod clipboard;

// The clipboard selection-extraction workers now live in `clipboard`, but the
// `SelectionHandler` trait impl (incl. `new_selection`) must stay here because
// it is a trait method of `State`.
use clipboard::{create_clipboard_pipe, spawn_clipboard_read_worker, write_selection_bytes_to_fd};

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
    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        debug!("Client {:?} disconnected: {:?}", client_id, reason);
    }
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
    /// Handle to the Wayland display, used to keep the data device
    /// (clipboard + drag-and-drop offers) focused on the right client.
    pub display_handle: Option<DisplayHandle>,
    pub xdg_decoration_state: Option<XdgDecorationState>,
    pub fractional_scale_manager_state: FractionalScaleManagerState,

    // Seat
    pub seat: Seat<Self>,

    // Axiom subsystems
    pub config: AxiomConfig,
    pub window_manager: Arc<parking_lot::RwLock<WindowManager>>,
    pub workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
    pub input_manager: Arc<parking_lot::RwLock<InputManager>>,

    // Tracking
    pub surfaces: HashMap<u32, SurfaceData>,
    pub window_map: HashMap<u64, u32>,
    pub next_window_id: u64,

    // Outputs
    pub outputs: Vec<Output>,

    /// Per-output DPI scale factors keyed by output name (e.g. "eDP-1" → 2.0).
    /// Empty in winit/noop mode where scale is implicitly 1.0.
    pub output_scale_factors: HashMap<String, f64>,

    /// Server-side decoration manager for titlebar/button rendering.
    /// Shared with [`AxiomCompositor`](crate::compositor::AxiomCompositor).
    pub decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,

    // Keep ToplevelSurface handles alive (they get destroyed when dropped)
    pub toplevels: HashMap<u32, ToplevelSurface>,
    pub toplevel_handles: HashMap<u32, ForeignToplevelHandle>,
    pub foreign_toplevel_list_state: ForeignToplevelListState,

    // Running state
    pub running: bool,
    pub needs_redraw: bool,

    // Current window/viewport size (updated via Resized events after dispatch)
    pub window_width: u32,
    pub window_height: u32,

    // Pointer tracking for input routing
    pub pointer_x: f64,
    pub pointer_y: f64,

    // Imported client buffer textures, keyed by the WlBuffer's ObjectId so a
    // client's pool of buffers (e.g. double-buffering) is uploaded to the GPU
    // exactly once and reused across frames. Evicted on buffer_destroyed.
    // ponytail: keying by buffer identity means transient buffers can linger
    // until destroy; fine for a compositor, add LRU eviction only if it grows.
    pub texture_cache: HashMap<ObjectId, TextureBuffer<GlesTexture>>,

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

    /// Cached clipboard payload served to both X11 and compositor-provided
    /// Wayland selections. Populated from explicit compositor updates and from
    /// the asynchronous Wayland-selection extraction worker.
    pub clipboard_cache: Option<Vec<u8>>,

    /// Sender used by async Wayland-selection extraction workers to publish
    /// freshly-read clipboard bytes back onto the compositor thread.
    clipboard_update_tx: mpsc::Sender<ClipboardUpdate>,
    /// Receiver drained in the main backend loop to refresh `clipboard_cache`
    /// without blocking the compositor thread on pipe reads.
    clipboard_update_rx: mpsc::Receiver<ClipboardUpdate>,

    /// Set when a client offers a new clipboard selection in `new_selection`.
    /// The actual data is fetched on the next cycle (see `maybe_fetch_clipboard`)
    /// because Smithay only registers the selection in `seat_data` *after*
    /// `new_selection` returns — `request_data_device_client_selection` would
    /// find nothing if called directly from `new_selection`.
    clipboard_fetch_pending: bool,

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
    fn keyboard_repeat_settings(config: &AxiomConfig) -> (i32, i32) {
        let delay = config.input.keyboard_repeat_delay.min(i32::MAX as u32) as i32;
        let rate = config.input.keyboard_repeat_rate.min(i32::MAX as u32) as i32;
        (delay, rate)
    }

    fn preferred_text_mime_type(mime_types: &[String]) -> Option<String> {
        [
            "text/plain;charset=utf-8",
            "text/plain;charset=UTF-8",
            "text/plain",
            "TEXT",
            "STRING",
        ]
        .iter()
        .find_map(|wanted| {
            mime_types
                .iter()
                .find(|candidate| candidate.as_str() == *wanted)
                .cloned()
        })
        .or_else(|| mime_types.first().cloned())
    }

    /// Fetch the offered selection payload into the clipboard cache. Called
    /// after a `new_selection` flag (set during `dispatch_clients`, once
    /// Smithay has registered the selection in `seat_data`). Spawns the pipe
    /// reader that streams the client's data into `clipboard_cache`.
    fn maybe_fetch_clipboard(&mut self) {
        if !self.clipboard_fetch_pending {
            return;
        }
        self.clipboard_fetch_pending = false;

        let mime_types = self
            .clipboard_source
            .as_ref()
            .map(|s| s.mime_types())
            .unwrap_or_default();
        let Some(mime) = Self::preferred_text_mime_type(&mime_types) else {
            return;
        };

        let seat = self.seat.clone();
        match create_clipboard_pipe() {
            Ok((read_fd, write_fd)) => match request_data_device_client_selection(
                &seat,
                mime.clone(),
                write_fd,
            ) {
                Ok(()) => {
                    debug!(
                        "📋 Requested Wayland clipboard payload via MIME {}",
                        mime
                    );
                    spawn_clipboard_read_worker(read_fd, self.clipboard_update_tx.clone());
                }
                Err(e) => warn!(
                    "⚠️ Failed requesting Wayland clipboard payload for MIME {}: {:?}",
                    mime, e
                ),
            },
            Err(e) => warn!("⚠️ Failed creating clipboard pipe: {}", e),
        }
    }

    fn display_title(title: Option<String>, app_id: Option<String>) -> String {
        title
            .filter(|s| !s.trim().is_empty())
            .or_else(|| app_id.filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| String::from("Wayland Client"))
    }

    fn read_xdg_toplevel_metadata(surface: &WlSurface) -> (Option<String>, Option<String>) {
        with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| {
                    data.lock()
                        .ok()
                        .map(|role| (role.title.clone(), role.app_id.clone()))
                })
                .unwrap_or((None, None))
        })
    }

    fn update_focus_state(&mut self, focused_window_id: Option<u64>) {
        self.window_manager
            .write()
            .set_focused_window(focused_window_id);
        let mut tracked_ids: Vec<u64> = self.window_map.keys().copied().collect();
        tracked_ids.sort_unstable();
        tracked_ids.dedup();
        let mut decorations = self.decoration_manager.write();
        for id in tracked_ids {
            decorations.set_window_focus(id, Some(id) == focused_window_id);
        }
    }

    fn update_surface_fractional_scale(&self, surface: &WlSurface) {
        let preferred_scale = self
            .window_id_for_surface(surface)
            .map(|window_id| {
                self.workspace_manager
                    .read()
                    .scale_factor_for_window(window_id)
            })
            .unwrap_or_else(|| self.focused_output_scale())
            .clamp(1.0, 4.0);

        with_states(surface, |states| {
            fractional_scale::with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(preferred_scale);
            });
        });
    }

    fn update_window_metadata(
        &mut self,
        surface_id: u32,
        title: Option<String>,
        app_id: Option<String>,
    ) {
        let effective_title = Self::display_title(title.clone(), app_id.clone());
        let window_id = self
            .surfaces
            .get(&surface_id)
            .and_then(|data| data.window_id);

        if let Some(surface_data) = self.surfaces.get_mut(&surface_id) {
            surface_data.title = effective_title.clone();
            surface_data.app_id = app_id.clone();
        }

        if let Some(window_id) = window_id {
            if let Some(window) = self.window_manager.write().get_window_mut(window_id) {
                window.window.title = effective_title.clone();
            }
            self.decoration_manager
                .write()
                .set_window_title(window_id, effective_title.clone());
        }
    }

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

        let visible_title = title.clone();
        let window_id = self
            .window_manager
            .write()
            .add_window(visible_title.clone());
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

        // Register decoration state — SSD rendering is now live via WGPU.
        self.decoration_manager.write().add_window(
            window_id,
            visible_title,
            backend_prefers_server_side_decorations(),
            640,
        );

        window_id
    }

    pub fn destroy_window(&mut self, surface_id: u32) {
        // Remove the ForeignToplevelHandle for external taskbars/docks
        if let Some(handle) = self.toplevel_handles.remove(&surface_id) {
            handle.send_closed();
        }
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
    /// Returns the scale factor of the focused output. The source of truth is
    /// the Output's own scale (tracked in `output_scale_factors`), not the
    /// workspace tape copy. Falls back to 1.0 when no output is registered.
    pub fn focused_output_scale(&self) -> f64 {
        self.output_scale_factors
            .values()
            .next()
            .copied()
            .unwrap_or(1.0)
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
    fn buffer_destroyed(&mut self, buffer: &wl_buffer::WlBuffer) {
        // Free the GPU texture we cached for this buffer (keyed by ObjectId).
        // Without this the GlesTexture (Arc<GlesTextureInternal>) keeps the GL
        // texture alive forever, leaking it when clients cycle through buffers.
        self.texture_cache.remove(&buffer.id());
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        match client.get_data::<ClientState>() {
            Some(state) => &state.compositor_state,
            None => {
                // Smithay initializes ClientState for every connected client, so
                // this branch is defensive only. Return a shared fallback rather
                // than panicking inside a protocol handler (which would kill the
                // whole compositor).
                log::error!(
                    "client_compositor_state: client has no ClientState; using fallback"
                );
                static FALLBACK: std::sync::OnceLock<CompositorClientState> =
                    std::sync::OnceLock::new();
                FALLBACK.get_or_init(CompositorClientState::default)
            }
        }
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

        // Upload SHM buffer to the GLES renderer and cache raw data for GL upload
        let window_id =
            self.window_map
                .iter()
                .find_map(|(&wid, &sid)| if sid == surface_id { Some(wid) } else { None });

        if window_id.is_some() {
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
                        if len > 0 {
                            // SAFETY: Smithay's `with_buffer_contents` callback
                            // guarantees that `ptr` points to `len` bytes of
                            // valid SHM buffer data for the duration of this
                            // closure. The slice is immediately copied (to_vec)
                            // before the closure returns, so no aliasing occurs.
                            let data = unsafe { std::slice::from_raw_parts(ptr, len) };

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
                // ponytail: raw SHM bytes were previously cached here for a
                // GL upload that render() never read — dropped as dead.
                let _ = buf_data;
            }
        } else if self.popups.contains_key(&surface_id) {
            // Popup buffer upload — cached for later GL upload
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
                // ponytail: previously cached raw SHM bytes for a GL upload
                // that render() never consumed — dropped as dead.
                let _ = (buf_data, w, h);
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

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let focused_window_id = focused.and_then(|surface| self.window_id_for_surface(surface));
        self.update_focus_state(focused_window_id);
        // Keep the Wayland data device (clipboard + drag-and-drop offers)
        // focused on the client under the keyboard focus, so a DnD drop target
        // receives the source's data offer.
        if let Some(dh) = &self.display_handle {
            let client = focused.and_then(|s| s.client());
            set_data_device_focus(dh, seat, client);
        }
        if let Some(window_id) = focused_window_id {
            debug!("🎯 Wayland focus changed to window {}", window_id);
        }
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        match image {
            CursorImageStatus::Named(icon) => self.cursor_icon = Some(icon),
            _ => self.cursor_icon = None,
        }
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
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

        let (title, app_id) = Self::read_xdg_toplevel_metadata(&wl_surface);
        let display_title = Self::display_title(title, app_id.clone());

        info!(
            "🪟 New XDG toplevel: surface={} title={:?} app_id={:?}",
            surface_id, display_title, app_id
        );

        // Create ForeignToplevelHandle for external taskbars/docks
        let ftl_handle = self
            .foreign_toplevel_list_state
            .new_toplevel::<State>(display_title.clone(), app_id.clone().unwrap_or_default());
        self.toplevel_handles.insert(surface_id, ftl_handle);
        self.needs_redraw = true;

        self.create_window_from_surface(surface_id, display_title, app_id, wl_surface.clone());
        self.update_surface_fractional_scale(&wl_surface);
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

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let surface_id = wl_surface.id().protocol_id();
        let (title, app_id) = Self::read_xdg_toplevel_metadata(&wl_surface);
        let display_title = Self::display_title(title.clone(), app_id.clone());
        self.update_window_metadata(surface_id, title, app_id);
        debug!(
            "📝 Updated XDG toplevel metadata: surface={} title={:?}",
            surface_id, display_title
        );
        self.needs_redraw = true;
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let surface_id = wl_surface.id().protocol_id();
        let (title, app_id) = Self::read_xdg_toplevel_metadata(&wl_surface);
        let display_title = Self::display_title(title.clone(), app_id.clone());
        self.update_window_metadata(surface_id, title, app_id.clone());
        debug!(
            "🪪 Updated XDG toplevel metadata: surface={} title={:?} app_id={:?}",
            surface_id, display_title, app_id
        );
        self.needs_redraw = true;
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
                    self.clipboard_source = Some(src.clone());
                    // Defer the actual data fetch: Smithay registers the
                    // selection in `seat_data` only *after* `new_selection`
                    // returns, so `request_data_device_client_selection` would
                    // find nothing if invoked here. Flag it and fetch on the
                    // next cycle, once the selection is registered.
                    self.clipboard_fetch_pending = true;
                } else {
                    debug!("📋 Wayland clipboard cleared");
                    self.clipboard_source = None;
                    self.clipboard_cache = None;
                    self.clipboard_fetch_pending = false;
                }
            }
            SelectionTarget::Primary => {
                debug!("📋 Wayland primary selection updated");
            }
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        if !matches!(ty, SelectionTarget::Clipboard) {
            return;
        }
        if let Some(data) = self.clipboard_cache.clone() {
            debug!(
                "📤 Serving compositor clipboard to Wayland client via MIME {} ({} bytes)",
                mime_type,
                data.len()
            );
            write_selection_bytes_to_fd(fd, &data);
        } else {
            debug!(
                "📤 Wayland client requested compositor clipboard via MIME {}, but cache is empty",
                mime_type
            );
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {
    fn started(
        &mut self,
        _source: Option<WlDataSource>,
        _icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        debug!("🖐️ Client-initiated drag-and-drop started");
    }

    fn dropped(&mut self, _target: Option<WlSurface>, _validated: bool, _seat: Seat<Self>) {
        debug!("🖐️ Client-initiated drag-and-drop finished");
    }
}

impl ServerDndGrabHandler for State {
    fn send(&mut self, mime_type: String, fd: OwnedFd, _seat: Seat<Self>) {
        // Axiom does not currently initiate server-side drags, so there is no
        // source payload to stream. Drop the fd so the requesting client does
        // not block forever waiting on data that will never arrive.
        debug!(
            "🖐️ DnD send requested for {} but no server drag source is configured",
            mime_type
        );
        drop(fd);
    }
}

impl OutputHandler for State {
    fn output_bound(&mut self, _output: Output, _wl_output: WlOutput) {
        debug!("🖥️ Client bound a wl_output instance");
    }
}

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        self.update_surface_fractional_scale(&surface);
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        let negotiated = negotiated_xdg_decoration_mode();
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(negotiated);
        });
        toplevel.send_configure();

        if let Some(window_id) = self.window_id_for_surface(toplevel.wl_surface()) {
            let mode = if negotiated == Mode::ServerSide {
                crate::decoration::DecorationMode::ServerSide
            } else {
                crate::decoration::DecorationMode::ClientSide
            };
            self.decoration_manager
                .write()
                .set_decoration_mode(window_id, mode);
        }
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        let negotiated = negotiated_xdg_decoration_mode();
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(negotiated);
        });
        toplevel.send_configure();

        if let Some(window_id) = self.window_id_for_surface(toplevel.wl_surface()) {
            let mode = if negotiated == Mode::ServerSide {
                crate::decoration::DecorationMode::ServerSide
            } else {
                crate::decoration::DecorationMode::ClientSide
            };
            self.decoration_manager
                .write()
                .set_decoration_mode(window_id, mode);
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        let negotiated = negotiated_xdg_decoration_mode();
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(negotiated);
        });
        toplevel.send_configure();

        if let Some(window_id) = self.window_id_for_surface(toplevel.wl_surface()) {
            let mode = if negotiated == Mode::ServerSide {
                crate::decoration::DecorationMode::ServerSide
            } else {
                crate::decoration::DecorationMode::ClientSide
            };
            self.decoration_manager
                .write()
                .set_decoration_mode(window_id, mode);
        }
    }
}

// Delegate macros
delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_xdg_shell!(State);
delegate_data_device!(State);
delegate_foreign_toplevel_list!(State);
smithay::delegate_fractional_scale!(State);
smithay::delegate_xdg_decoration!(State);
smithay::delegate_output!(State);

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

        let mut seat_state = SeatState::new();
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
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
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

        let display: Display<State> = Display::new()?;
        let dh = display.handle();

        let compositor_state = CompositorState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<State>(&dh);

        let xdg_decoration_state = if config.features.enable_xdg_decoration_protocol {
            info!("🌐 Registering zxdg_decoration_manager_v1 global");
            Some(XdgDecorationState::new::<State>(&dh))
        } else {
            None
        };

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "axiom");
        let (clipboard_update_tx, clipboard_update_rx) = mpsc::channel();

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
            display_handle: Some(display.handle()),
            xdg_decoration_state,
            fractional_scale_manager_state,
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
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
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
        };

        let socket_name = format!("wayland-axiom-{}", std::process::id());
        let listener = ListeningSocket::bind(&socket_name)?;
        info!("📡 Wayland socket: {}", socket_name);

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
                .add_keyboard(XkbConfig::default(), repeat_delay, repeat_rate)?;

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
            let host_scale = self
                .winit_backend
                .as_ref()
                .map(|b| b.window().scale_factor().clamp(1.0, 4.0))
                .unwrap_or(1.0);
            {
                let mut wm = self.state.workspace_manager.write();
                let tape = wm.ensure_tape("default");
                tape.set_scale_factor(host_scale);
                tape.set_viewport_size(w as f64, h as f64);
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


    /// Common post-event dispatch for all backends.
    fn run_one_cycle_common(&mut self) -> Result<()> {
        // Accept new Wayland clients on the bound listening socket. Without
        // this, connect() succeeds at the kernel level but the server never
        // reads the connection, so no client can ever bind to Axiom.
        if let Some(listener) = &self.listener {
            loop {
                match listener.accept() {
                    Ok(Some(stream)) => {
                        if let Err(e) = self.display.handle().insert_client(
                            stream,
                            Arc::new(ClientState {
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
            let _ = self
                .state
                .seat
                .add_keyboard(XkbConfig::default(), 0, 0);
        }
        let surface = self
            .state
            .toplevels
            .values()
            .next()
            .map(|t| t.wl_surface().clone());
        if let Some(surface) = surface {
            if let Some(keyboard) = self.state.seat.get_keyboard() {
                keyboard.set_focus(&mut self.state, Some(surface.clone()), Serial::from(0));
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

fn smithay_output_scale(scale: f64) -> Scale {
    if (scale.fract()).abs() < f64::EPSILON {
        Scale::Integer(scale.round().max(1.0) as i32)
    } else {
        Scale::Fractional(scale)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        backend_prefers_server_side_decorations, negotiated_xdg_decoration_mode,
        smithay_output_scale, State,
    };
    use crate::config::AxiomConfig;
    use smithay::output::Scale;
    use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;

    #[test]
    fn test_keyboard_repeat_settings_follow_config_values() {
        let mut cfg = AxiomConfig::default();
        cfg.input.keyboard_repeat_delay = 600;
        cfg.input.keyboard_repeat_rate = 25;
        assert_eq!(State::keyboard_repeat_settings(&cfg), (600, 25));
    }

    #[test]
    fn test_display_title_prefers_explicit_title() {
        let title = State::display_title(Some("My App".into()), Some("org.example.App".into()));
        assert_eq!(title, "My App");
    }

    #[test]
    fn test_display_title_falls_back_to_app_id() {
        let title = State::display_title(Some("   ".into()), Some("org.example.App".into()));
        assert_eq!(title, "org.example.App");
    }

    #[test]
    fn test_display_title_falls_back_to_default() {
        let title = State::display_title(None, None);
        assert_eq!(title, "Wayland Client");
    }

    #[test]
    fn test_backend_prefers_server_side_decorations() {
        // SSD rendering is live via the GLES pipeline (SolidColorRenderElement
        // for backdrop/titlebar/buttons, TextureRenderElement for client content).
        // Title text rendering falls back gracefully when system fonts unavailable.
        assert!(backend_prefers_server_side_decorations());
        assert_eq!(negotiated_xdg_decoration_mode(), Mode::ServerSide);
    }

    #[test]
    fn test_smithay_output_scale_supports_fractional_values() {
        match smithay_output_scale(1.5) {
            Scale::Fractional(scale) => assert!((scale - 1.5).abs() < f64::EPSILON),
            other => panic!("expected fractional scale, got {:?}", other),
        }
    }

    #[test]
    fn test_preferred_text_mime_type_prefers_utf8_plain_text() {
        let mime = State::preferred_text_mime_type(&[
            "application/json".to_string(),
            "text/plain;charset=utf-8".to_string(),
        ]);
        assert_eq!(mime.as_deref(), Some("text/plain;charset=utf-8"));
    }
}
