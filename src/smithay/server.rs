//! Unified Smithay compositor server
//!
//! This module provides a single, full compositor backend based on Wayland server
//! primitives. It integrates with Axiom managers and the GPU renderer for texture
//! updates and presentation.

use anyhow::{Context, Result};
use log::{debug, info};
use memmap2::{Mmap, MmapOptions};
use parking_lot::RwLock;
use std::collections::HashMap;
use wgpu;
use std::fs::File;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use calloop::EventLoop;
use calloop::timer::{Timer, TimeoutAction};
use xkbcommon::xkb;
use std::ffi::CString;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, IntoRawFd, OwnedFd};
use wayland_protocols::wp::presentation_time::server::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::wp::viewporter::server::{wp_viewport, wp_viewporter};
use wayland_protocols::wp::linux_dmabuf::zv1::server::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::xdg::shell::server::{xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base, xdg_popup};
use wayland_protocols::xdg::decoration::zv1::server::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::wp::primary_selection::zv1::server::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1, zwp_primary_selection_offer_v1,
    zwp_primary_selection_source_v1,
};
use wayland_server::{
    backend::ClientData,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch,
    },
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New, Resource, WEnum,
};
use wayland_protocols_wlr::layer_shell::v1::server::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};

/// Global compositor state for this server
pub struct CompositorState {
    pub seat_name: String,
    pub windows: Vec<WindowEntry>,
    // Layer-shell entries
    pub layer_surfaces: Vec<LayerSurfaceEntry>,
    // X11 (XWayland) role-less surfaces
    pub x11_surfaces: Vec<X11SurfaceEntry>,
    // Handle to workspace manager for exclusive zone application
    pub workspace_manager_handle: Arc<RwLock<crate::workspace::ScrollableWorkspaces>>,
    pub serial_counter: u32,
    pub xdg_bases: Vec<xdg_wm_base::XdgWmBase>,
    pub keyboards: Vec<wl_keyboard::WlKeyboard>,
    pub pointers: Vec<wl_pointer::WlPointer>,
    pub touches: Vec<wl_touch::WlTouch>,
    pub pending_callbacks: Vec<wl_callback::WlCallback>,
    pub last_frame_time: Instant,
    pub last_ping_time: Instant,
    // xkbcommon keymap information for wl_keyboard
    pub xkb: Option<XkbInfo>,
    // positioner states by resource id
    pub positioners: HashMap<u32, PositionerState>,
    // Internal event bus queue (drained in run loop)
    events: Vec<ServerEvent>,
    // Focused Axiom window id (if any)
    pub focused_window_id: Option<u64>,
    // Pointer state
    pub pointer_pos: (f64, f64),
    pub pointer_focus_window: Option<u64>,
    // Cache of last computed layouts for hit-testing
    pub last_layouts: HashMap<u64, crate::window::Rectangle>,
    // Presentation feedbacks by wl_surface id
    pub presentation_feedbacks: HashMap<u32, Vec<wp_presentation_feedback::WpPresentationFeedback>>,
    // Viewporter state per surface id
    viewport_map: HashMap<u32, ViewportState>,
    // Damage tracking per surface id (x, y, width, height)
    pub damage_map: HashMap<u32, Vec<(i32, i32, i32, i32)>>,
    // Advertised dmabuf formats (fourcc, modifier)
    pub dmabuf_formats: Vec<(u32, u64)>,
    // Presentation sequence counter (monotonic frame counter)
    pub present_seq: u64,
    // Clipboard/text selection and DnD infrastructure
    pub data_devices: Vec<wl_data_device::WlDataDevice>,
    pub data_sources: HashMap<u32, DataSourceEntry>,
    pub active_offers: HashMap<u32, DataOfferEntry>,
    pub selection: Option<SelectionState>,
    pub clipboard: Arc<RwLock<crate::clipboard::ClipboardManager>>,
    // Primary selection (middle-click paste) state
    pub primary_devices: Vec<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    pub primary_sources: HashMap<u32, PrimarySourceEntry>,
    pub primary_offers: HashMap<u32, PrimaryOfferEntry>,
    pub primary_selection: Option<PrimarySelectionState>,

    // XDG decorations
    pub toplevel_decorations: HashMap<u32, zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    pub decoration_modes: HashMap<u32, zxdg_toplevel_decoration_v1::Mode>,
    pub decoration_to_toplevel: HashMap<u32, u32>,
    pub force_client_side_decorations: bool,
}

#[derive(Clone)]
pub struct WindowEntry {
    pub xdg_surface: xdg_surface::XdgSurface,
    pub xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub xdg_popup: Option<xdg_popup::XdgPopup>,
    pub wl_surface: Option<wl_surface::WlSurface>,
    pub configured_serial: Option<u32>,
    pub mapped: bool,
    pub title: String,
    pub app_id: String,
    pub axiom_id: Option<u64>,
    // Pending attached wl_buffer id and offset
    pub pending_buffer_id: Option<u32>,
    pub attach_offset: (i32, i32),
    // Popup metadata
    pub parent_surface_id: Option<u32>,
    pub positioner_id: Option<u32>,
    pub window_type: crate::window::WindowType,
}

/// A single full compositor server that owns the Wayland display and integrates
/// with Axiom managers and the renderer.
pub struct CompositorServer {
    pub display: Display<CompositorState>,
    pub listening: ListeningSocket,
    pub socket_name: String,
    // Axiom managers for integration
    pub window_manager: Arc<RwLock<crate::window::WindowManager>>, 
    pub workspace_manager: Arc<RwLock<crate::workspace::ScrollableWorkspaces>>, 
    pub input_manager: Arc<RwLock<crate::input::InputManager>>, 
    pub clipboard: Arc<RwLock<crate::clipboard::ClipboardManager>>, 
    // Input channel from evdev thread
#[allow(dead_code)]
    input_rx: Option<Receiver<HwInputEvent>>,
    // Whether to spawn the headless GPU render loop (disabled when doing on-screen present)
    spawn_headless_renderer: bool,
    // Selected WGPU backends for renderer creation
    selected_backends: wgpu::Backends,
    // Present events from on-screen presenter (winit path)
    present_rx: Option<Receiver<PresentEvent>>,
    // Redraw signal to on-screen presenter
    redraw_tx: Option<std::sync::mpsc::Sender<()>>,
}

// Internal event bus messages produced by Wayland dispatch and handled in the run loop
#[derive(Debug, Clone)]
enum ServerEvent {
    Commit { surface: wl_surface::WlSurface },
    Destroy { surface: wl_surface::WlSurface },
    TitleChanged { surface: wl_surface::WlSurface, title: String },
    AppIdChanged { surface: wl_surface::WlSurface, app_id: String },
    DecorationModeChanged { toplevel_id: u32, mode: zxdg_toplevel_decoration_v1::Mode },
}

// Hardware input events captured by evdev thread
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum HwInputEvent {
    Key { key: String, modifiers: Vec<String>, pressed: bool },
    PointerMotion { dx: f64, dy: f64 },
    PointerButton { button: u8, pressed: bool },
    PointerAxis { horizontal: f64, vertical: f64 },
    GestureSwipe { dx: f64, dy: f64, fingers: u32, phase: GesturePhase },
    GesturePinch { scale: f64, rotation: f64, dx: f64, dy: f64, fingers: u32, phase: GesturePhase },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GesturePhase { Begin, Update, End }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxiomLayerKind { Background, Bottom, Top, Overlay }

#[derive(Clone)]
pub struct LayerSurfaceEntry {
    pub wl_surface: wl_surface::WlSurface,
    pub wlr_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub layer: AxiomLayerKind,
    pub namespace: String,
    pub anchors: u32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    pub exclusive_zone: i32,
    pub keyboard_interactivity: u32,
    pub desired_size: (i32, i32),
    pub mapped: bool,
    pub configured_serial: Option<u32>,
    pub axiom_id: Option<u64>,
    pub pending_buffer_id: Option<u32>,
    pub attach_offset: (i32, i32),
    pub last_geometry: crate::window::Rectangle,
}

#[derive(Clone)]
pub struct X11SurfaceEntry {
    pub wl_surface: wl_surface::WlSurface,
    pub mapped: bool,
    pub pending_buffer_id: Option<u32>,
    pub attach_offset: (i32, i32),
    pub axiom_id: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct PresentEvent {
    pub tv_sec_hi: u32,
    pub tv_sec_lo: u32,
    pub tv_nsec: u32,
    pub refresh_ns: u32,
    pub flags: u32, // bitmask compatible with wp_presentation_feedback::Kind
}

impl CompositorServer {
pub fn new(
        window_manager: Arc<RwLock<crate::window::WindowManager>>, 
        workspace_manager: Arc<RwLock<crate::workspace::ScrollableWorkspaces>>, 
        input_manager: Arc<RwLock<crate::input::InputManager>>, 
        clipboard: Arc<RwLock<crate::clipboard::ClipboardManager>>,
        present_rx: Option<Receiver<PresentEvent>>,
        redraw_tx: Option<std::sync::mpsc::Sender<()>>,
        spawn_headless_renderer: bool,
        selected_backends: wgpu::Backends,
    ) -> Result<Self> {
        let display: Display<CompositorState> = Display::new().context("create display")?;
        let dh = display.handle();

        // Create core globals
        dh.create_global::<CompositorState, wl_compositor::WlCompositor, _>(4, ());
        dh.create_global::<CompositorState, wl_shm::WlShm, _>(1, ());
        dh.create_global::<CompositorState, wl_output::WlOutput, _>(3, ());
        dh.create_global::<CompositorState, wl_seat::WlSeat, _>(7, ());
        dh.create_global::<CompositorState, xdg_wm_base::XdgWmBase, _>(3, ());
        dh.create_global::<CompositorState, wp_presentation::WpPresentation, _>(1, ());
        dh.create_global::<CompositorState, wp_viewporter::WpViewporter, _>(1, ());
        dh.create_global::<CompositorState, wl_data_device_manager::WlDataDeviceManager, _>(3, ());
        dh.create_global::<CompositorState, zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _>(4, ());
        dh.create_global::<CompositorState, zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1, _>(1, ());
dh.create_global::<CompositorState, zwlr_layer_shell_v1::ZwlrLayerShellV1, _>(1, ());
dh.create_global::<CompositorState, zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, _>(1, ());
debug!("Globals: wl_compositor v4, wl_shm v1, wl_output v3, wl_seat v7, xdg_wm_base v3, wl_data_device_manager v3, primary_selection v1, wlr_layer_shell v1, xdg-decoration v1");

        // Bind an auto socket for Wayland
        let listening = ListeningSocket::bind_auto("wayland", 1..32).context("bind socket")?;
        let socket_name = listening
.socket_name().map(|s| s.to_string_lossy().to_string())
            .ok_or_else(|| anyhow::anyhow!("missing socket name"))?;

        // Spawn libinput + evdev input threads (best-effort)
        let input_rx = Self::spawn_combined_input_threads();

        Ok(Self {
            display,
            listening,
            socket_name,
            window_manager,
            workspace_manager,
            input_manager,
            clipboard,
            input_rx,
            spawn_headless_renderer,
            selected_backends,
            present_rx,
            redraw_tx,
        })
    }

    pub fn run(self) -> Result<()> {
        std::env::set_var("WAYLAND_DISPLAY", &self.socket_name);
        info!("WAYLAND_DISPLAY={}", self.socket_name);
        // Start XWayland if available so X11 apps can connect
        let mut _xwayland_guard: Option<crate::xwayland::XWaylandManager> = None;
        {
            let wl_name = self.socket_name.clone();
            if let Ok(mut xm) = crate::xwayland::XWaylandManager::new(&crate::config::XWaylandConfig { enabled: true, display: None }) {
                let _ = xm.start_server(&wl_name);
                _xwayland_guard = Some(xm);
            }
        }

        // Start headless GPU render loop in a background thread (optional)
        if self.spawn_headless_renderer {
            let backends = self.selected_backends;
            std::thread::spawn(move || {
                if let Ok(rt) = tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                    rt.block_on(async move {
                        let _ = crate::renderer::AxiomRenderer::start_headless_loop_with_backends(backends).await;
                    });
                }
            });
        }

        // Initialize workspace viewport to match our single wl_output mode
        {
            let mut ws = self.workspace_manager.write();
            ws.set_viewport_size(1920.0, 1080.0);
        }

        // Read decoration policy from environment (set by caller)
        let force_csd: bool = std::env::var("AXIOM_FORCE_CSD")
            .map(|s| matches!(s.as_str(), "1" | "true" | "TRUE" | "True"))
            .unwrap_or(false);

        let mut state = CompositorState {
            seat_name: "seat0".into(),
            windows: Vec::new(),
            layer_surfaces: Vec::new(),
            x11_surfaces: Vec::new(),
            workspace_manager_handle: self.workspace_manager.clone(),
            serial_counter: 1,
            xdg_bases: Vec::new(),
            keyboards: Vec::new(),
            pointers: Vec::new(),
            touches: Vec::new(),
            pending_callbacks: Vec::new(),
            last_frame_time: Instant::now(),
            last_ping_time: Instant::now(),
            events: Vec::new(),
            focused_window_id: None,
            pointer_pos: (960.0, 540.0),
            pointer_focus_window: None,
            last_layouts: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
            viewport_map: HashMap::new(),
            damage_map: HashMap::new(),
            dmabuf_formats: vec![
                (0x34325258u32, 0), // DRM_FORMAT_XRGB8888, MOD_LINEAR ('XR24')
                (0x34325241u32, 0), // DRM_FORMAT_ARGB8888, MOD_LINEAR ('AR24')
                (0x34324258u32, 0), // DRM_FORMAT_XBGR8888, MOD_LINEAR ('XB24')
                (0x34324241u32, 0), // DRM_FORMAT_ABGR8888, MOD_LINEAR ('AB24')
                (0x3231564Eu32, 0), // DRM_FORMAT_NV12, MOD_LINEAR
            ],
            xkb: build_default_xkb_info(),
            positioners: HashMap::new(),
            present_seq: 0,
            data_devices: Vec::new(),
            data_sources: HashMap::new(),
            active_offers: HashMap::new(),
            selection: None,
            clipboard: self.clipboard.clone(),
            primary_devices: Vec::new(),
            primary_sources: HashMap::new(),
            primary_offers: HashMap::new(),
            primary_selection: None,

            toplevel_decorations: HashMap::new(),
            decoration_modes: HashMap::new(),
            decoration_to_toplevel: HashMap::new(),
            force_client_side_decorations: force_csd,
        };

        // Create calloop event loop
        let mut event_loop = EventLoop::try_new().context("create calloop")?;
        let handle = event_loop.handle();

        // Move listening socket and display into dispatch timer closure
        let listening = self.listening;
        let mut display_handle = self.display.handle();

        // Frame timer (~16ms)
        let frame_timer = Timer::from_duration(Duration::from_millis(16));
        let mut present_rx_opt = self.present_rx; // captured by move
        let mut redraw_tx_opt = self.redraw_tx.clone();
        handle.insert_source(frame_timer, move |_deadline: Instant, _meta: &mut (), data: &mut CompositorState| {
            // Drain callbacks and send done
            let now = std::time::SystemTime::now();
            let dur = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let ts_ms: u32 = ((dur.as_millis()) & 0xFFFF_FFFF) as u32;
            let mut had_activity = false;
            for cb in data.pending_callbacks.drain(..) {
                cb.done(ts_ms);
                had_activity = true;
            }
            // Presentation feedback: present all pending feedbacks
            if !data.presentation_feedbacks.is_empty() {
                // Try to get a recent present event from on-screen path
                let mut ev_latest: Option<PresentEvent> = None;
                if let Some(rx) = present_rx_opt.as_mut() {
                    loop {
                        match rx.try_recv() {
                            Ok(ev) => ev_latest = Some(ev),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => { present_rx_opt = None; break; }
                        }
                    }
                }
                // Compose timing values
                let (tv_sec_hi, tv_sec_lo, tv_nsec, refresh_ns, flags_kind) = if let Some(ev) = ev_latest {
                    let kind = wp_presentation_feedback::Kind::from_bits_truncate(ev.flags);
                    (ev.tv_sec_hi, ev.tv_sec_lo, ev.tv_nsec, ev.refresh_ns, kind)
                } else {
                    // Fallback to current time and ~60Hz
                    let tv_sec = dur.as_secs();
                    let tv_nsec = dur.subsec_nanos();
                    let tv_sec_hi: u32 = (tv_sec >> 32) as u32;
                    let tv_sec_lo: u32 = (tv_sec & 0xFFFF_FFFF) as u32;
                    let refresh_ns: u32 = 16_666_666;
                    (tv_sec_hi, tv_sec_lo, tv_nsec, refresh_ns, wp_presentation_feedback::Kind::Vsync)
                };
                let seq = data.present_seq;
                let seq_hi: u32 = (seq >> 32) as u32;
                let seq_lo: u32 = (seq & 0xFFFF_FFFF) as u32;
                for (_sid, list) in std::mem::take(&mut data.presentation_feedbacks) {
                    for fb in list {
                        fb.presented(tv_sec_hi, tv_sec_lo, tv_nsec, refresh_ns, seq_hi, seq_lo, flags_kind);
                        had_activity = true;
                    }
                }
                data.present_seq = data.present_seq.wrapping_add(1);
            }
            if had_activity {
                if let Some(tx) = &redraw_tx_opt { let _ = tx.send(()); }
            }
            data.last_frame_time = Instant::now();
            // Re-arm timer
            TimeoutAction::ToDuration(Duration::from_millis(16))
        }).map_err(|_| anyhow::anyhow!("register frame timer"))?;

        // Ping timer (~5s)
        let ping_timer = Timer::from_duration(Duration::from_secs(5));
        handle.insert_source(ping_timer, move |_deadline: Instant, _meta: &mut (), data: &mut CompositorState| {
            let serial = data.serial_counter;
            for base in &data.xdg_bases {
                base.ping(serial);
            }
            data.serial_counter = data.serial_counter.wrapping_add(1);
            data.last_ping_time = Instant::now();
            TimeoutAction::ToDuration(Duration::from_secs(5))
        }).map_err(|_| anyhow::anyhow!("register ping timer"))?;

        // Main dispatch via a small idle loop on calloop; use a repeated timer to dispatch/flush/handle events
        let dispatch_timer = Timer::from_duration(Duration::from_millis(4));
        let mut display_for_dispatch = self.display;
        let wm = self.window_manager;
        let ws = self.workspace_manager;
        let mut _unused_rx = self.input_rx; // keep ownership to avoid borrow issues; input path is disabled
        let mut redraw_tx_opt2 = self.redraw_tx.clone();
        handle.insert_source(dispatch_timer, move |_deadline: Instant, _meta: &mut (), data: &mut CompositorState| {
            // Accept any pending clients
            loop {
                match listening.accept() {
                    Ok(Some(stream)) => {
                        let _ = display_handle.insert_client(stream, Arc::new(ServerClientData));
                        debug!("Client connected");
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            // Dispatch Wayland clients
            let _ = display_for_dispatch.dispatch_clients(data);
            // Capture whether there is obvious activity (surface damage pending or events queued)
            let had_events = !data.events.is_empty();
            let had_damage = !data.damage_map.is_empty();
            // Drain and handle internal events with access to managers
            let _ = handle_events_inline(data, &wm, &ws);
            // Flush clients
            let _ = display_for_dispatch.flush_clients();
            if had_events || had_damage {
                if let Some(tx) = &redraw_tx_opt2 { let _ = tx.send(()); }
            }
            // Re-arm timer
            TimeoutAction::ToDuration(Duration::from_millis(4))
        }).map_err(|_| anyhow::anyhow!("register dispatch timer"))?;

        // Run event loop
        let _ = event_loop.run(None, &mut state, |_| {});
        Ok(())
    }
}

struct ServerClientData;
impl ClientData for ServerClientData {}

impl CompositorServer {
    #[allow(dead_code)]
    fn handle_hw_input(&mut self, state: &mut CompositorState) -> Result<()> {
        use crate::input::{CompositorAction, InputEvent as AxiomInputEvent};
        // Drain the channel if present
        let mut buf: Vec<HwInputEvent> = Vec::new();
        if let Some(rx) = &self.input_rx {
            loop {
                match rx.try_recv() {
                    Ok(ev) => buf.push(ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.input_rx = None;
                        break;
                    }
                }
            }
        }

        for ev in buf {
            match ev {
                HwInputEvent::GestureSwipe { .. } | HwInputEvent::GesturePinch { .. } => { /* gesture path disabled */ },
                HwInputEvent::Key { key, modifiers, pressed } => {
                    if pressed {
                        let actions = self.input_manager.write().process_input_event(
                            AxiomInputEvent::Keyboard { key: key.clone(), modifiers: modifiers.clone(), pressed }
                        );
                        for action in actions {
                            match action {
                                CompositorAction::ScrollWorkspaceLeft => {
                                    self.workspace_manager.write().scroll_left();
                                    self.apply_layouts(state)?;
                                }
                                CompositorAction::ScrollWorkspaceRight => {
                                    self.workspace_manager.write().scroll_right();
                                    self.apply_layouts(state)?;
                                }
                                CompositorAction::MoveWindowLeft => {
                                    let fid_opt = { let wm = self.window_manager.read(); wm.focused_window_id() };
                                    if let Some(fid) = fid_opt {
                                        let moved = { let mut ws = self.workspace_manager.write(); ws.move_window_left(fid) };
                                        if moved { self.apply_layouts(state)?; }
                                    }
                                }
                                CompositorAction::MoveWindowRight => {
                                    let fid_opt = { let wm = self.window_manager.read(); wm.focused_window_id() };
                                    if let Some(fid) = fid_opt {
                                        let moved = { let mut ws = self.workspace_manager.write(); ws.move_window_right(fid) };
                                        if moved { self.apply_layouts(state)?; }
                                    }
                                }
                                CompositorAction::ToggleFullscreen => {
                                    let fid_opt = { let wm = self.window_manager.read(); wm.focused_window_id() };
                                    if let Some(fid) = fid_opt {
                                        { let mut wm = self.window_manager.write(); let _ = wm.toggle_fullscreen(fid); }
                                        self.apply_layouts(state)?;
                                    }
                                }
                                CompositorAction::CloseWindow => {
                                    let fid_opt = { let wm = self.window_manager.read(); wm.focused_window_id() };
                                    if let Some(fid) = fid_opt {
                                        if let Some(tl) = state
                                            .windows
                                            .iter()
                                            .find(|w| w.axiom_id == Some(fid))
                                            .and_then(|w| w.xdg_toplevel.clone())
                                        {
                                            tl.close();
                                        }
                                    }
                                }
                                CompositorAction::Quit => {
                                    // Graceful shutdown: currently ignored in this server loop
                                }
                                _ => {}
                            }
                        }
                        // Update wl_keyboard modifiers for connected clients
                        self.send_modifiers(state, &modifiers);
                    }
                }
                HwInputEvent::PointerMotion { dx, dy } => {
                    // Update pointer position
                    state.pointer_pos.0 = (state.pointer_pos.0 + dx).clamp(0.0, 1920.0);
                    state.pointer_pos.1 = (state.pointer_pos.1 + dy).clamp(0.0, 1080.0);
                    self.update_pointer_focus_and_motion(state)?;
                }
                HwInputEvent::PointerButton { button, pressed } => {
                    self.handle_pointer_button(state, button, pressed)?;
                }
                HwInputEvent::PointerAxis { horizontal, vertical } => {
                    // Send scroll events to current pointer focus if any
                    if state.pointer_focus_window.is_some() {
                        let time_ms: u32 = (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() & 0xFFFF_FFFF) as u32;
                        if horizontal != 0.0 {
                            for ptr in &state.pointers {
                                ptr.axis(time_ms, wl_pointer::Axis::HorizontalScroll, horizontal);
                            }
                        }
                        if vertical != 0.0 {
                            for ptr in &state.pointers {
                                ptr.axis(time_ms, wl_pointer::Axis::VerticalScroll, vertical);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn handle_events(&mut self, state: &mut CompositorState) -> Result<()> {
        // Take events out to avoid borrow issues while mutating state
        let mut events = Vec::new();
        events.append(&mut state.events);

        for ev in events {
            match ev {
                ServerEvent::Commit { surface } => {
                    // Locate window entry by surface
                    if let Some(idx) = state
                        .windows
                        .iter()
                        .position(|w| w.wl_surface.as_ref() == Some(&surface))
                    {
                        // Read-only check first to avoid holding a mutable borrow across mutations
                        let (should_map, title) = {
                            let w = &state.windows[idx];
                            let t = if w.title.is_empty() { "Untitled".to_string() } else { w.title.clone() };
                            (w.configured_serial.is_some() && !w.mapped, t)
                        };

                        if should_map {
                            // Map window into Axiom managers
                            let new_id = {
                                let mut wm = self.window_manager.write();
                                wm.add_window(title)
                            };
                            {
                                let mut ws = self.workspace_manager.write();
                                ws.add_window(new_id);
                            }
                            {
                                let _ = self.window_manager.write().focus_window(new_id);
                            }
                            let previous_focus = state.focused_window_id.take();
                            state.focused_window_id = Some(new_id);

                            // Update entry
                            {
                                let win_mut = &mut state.windows[idx];
                                win_mut.axiom_id = Some(new_id);
                                win_mut.mapped = true;
                            }

                            // Apply decoration policy to window manager (SSD vs CSD)
                            {
                                let decorated = {
                                    let w = &state.windows[idx];
                                    let default_ssd = !state.force_client_side_decorations;
                                    if let Some(ref tl) = w.xdg_toplevel {
                                        let tlid = tl.id().protocol_id();
                                        if let Some(mode) = state.decoration_modes.get(&tlid) {
                                            matches!(mode, zxdg_toplevel_decoration_v1::Mode::ServerSide)
                                        } else {
                                            default_ssd
                                        }
                                    } else {
                                        default_ssd
                                    }
                                };
                                // Update window manager decoration flag
                                let _ = self.window_manager.write().set_window_decorated(new_id, decorated);
                            }

                            // Input focus routing: leave previous, enter new
                            if let Some(prev_id) = previous_focus {
                                if let Some(prev_surface) = state
                                    .windows
                                    .iter()
                                    .find(|w| w.axiom_id == Some(prev_id))
                                    .and_then(|w| w.wl_surface.clone())
                                {
                                    let serial = state.next_serial();
                                    for kb in &state.keyboards {
                                        kb.leave(serial, &prev_surface);
                                    }
                                    let serial = state.next_serial();
                                    for ptr in &state.pointers {
                                        ptr.leave(serial, &prev_surface);
                                    }
                                }
                            }

                            let serial = state.next_serial();
                            for kb in &state.keyboards {
                                kb.enter(serial, &surface, vec![]);
                            }
                            let serial = state.next_serial();
                            for ptr in &state.pointers {
                                ptr.enter(serial, &surface, 0.0, 0.0);
                            }

                            debug!("axiom: mapped window id={:?}", state.windows[idx].axiom_id);

                            // Apply layout-driven configure to all windows
                            self.apply_layouts(state)?;
                        }
                        // After handling mapping/focus, if a buffer is attached, upload to renderer
                        if let Some(win) = state.windows.iter_mut().find(|w| w.wl_surface.as_ref() == Some(&surface)) {
                            if let (Some(ax_id), Some(buf_id)) = (win.axiom_id, win.pending_buffer_id.take()) {
                                if let Some(rec) = state_buffers(state).get(&buf_id).cloned() {
                                    let sid = surface.id().protocol_id();
                                    let vp = state.viewport_map.get(&sid).cloned();
                                if let Some((data, w, h)) = process_with_viewport(&rec, vp.as_ref()) {
                                    let sid = surface.id().protocol_id();
                                    if vp.is_none() {
                                        // Try damage-aware region uploads
                                        if let Some(damages) = state.damage_map.remove(&sid) {
                                            for (dx, dy, dw, dh) in damages {
                                                if dx >= 0 && dy >= 0 && dw > 0 && dh > 0 {
                                                    let (dxu, dyu, dwu, dhu) = (dx as u32, dy as u32, dw as u32, dh as u32);
                                                    let mut bytes = Vec::with_capacity((dwu * dhu * 4) as usize);
                                                    for row in 0..dhu {
                                                        let src_off = (((dyu + row) * w + dxu) * 4) as usize;
                                                        let end = src_off + (dwu * 4) as usize;
                                                        bytes.extend_from_slice(&data[src_off..end]);
                                                    }
                                                    crate::renderer::queue_texture_update_region(ax_id, w, h, (dxu, dyu, dwu, dhu), bytes);
                                                }
                                            }
                                        } else {
                                            crate::renderer::queue_texture_update(ax_id, data, w, h);
                                        }
                                    } else {
                                        // Viewport path: fall back to full upload
                                        crate::renderer::queue_texture_update(ax_id, data, w, h);
                                    }
                                    rec.buffer.release();
                                }
                                }
                            }
                        }
                    }
                }
                ServerEvent::Destroy { surface } => {
                    // Find window entry
                    if let Some(idx) = state
                        .windows
                        .iter()
                        .position(|w| w.wl_surface.as_ref() == Some(&surface))
                    {
                        let entry = state.windows.remove(idx);
                        if let Some(id) = entry.axiom_id {
                            // Clear focus if needed
                            if state.focused_window_id == Some(id) {
                                state.focused_window_id = None;
                            }
                            // Remove from managers
                            {
                                let mut ws = self.workspace_manager.write();
                                let _ = ws.remove_window(id);
                            }
                            {
                                let mut wm = self.window_manager.write();
                                let _ = wm.remove_window(id);
                            }
                            // Re-focus last mapped window if any
                            if let Some(new_focus_id) = state
                                .windows
                                .iter()
                                .rev()
                                .find_map(|w| w.axiom_id)
                            {
                                let _ = self.window_manager.write().focus_window(new_focus_id);
                                state.focused_window_id = Some(new_focus_id);

                                // Seat focus enter to new surface
                                if let Some(focus_surface) = state
                                    .windows
                                    .iter()
                                    .find(|w| w.axiom_id == Some(new_focus_id))
                                    .and_then(|w| w.wl_surface.clone())
                                {
                                    let serial = state.next_serial();
                                    for kb in &state.keyboards {
                                        kb.enter(serial, &focus_surface, vec![]);
                                    }
                                    let serial = state.next_serial();
                                    for ptr in &state.pointers {
                                        ptr.enter(serial, &focus_surface, 0.0, 0.0);
                                    }
                                }
                            }

                            // Update layout after removal and remove renderer placeholder
                            if let Some(new_focus_id) = state.focused_window_id { let _ = new_focus_id; }
                            crate::renderer::remove_placeholder_quad(id);
                            self.apply_layouts(state)?;
                        }
                    }
                }
                ServerEvent::TitleChanged { surface, title } => {
                    if let Some(win) = state
                        .windows
                        .iter_mut()
                        .find(|w| w.wl_surface.as_ref() == Some(&surface))
                    {
                        win.title = title.clone();
                        if let Some(id) = win.axiom_id {
                            if let Some(w) = self.window_manager.write().get_window_mut(id) {
                                w.window.title = title;
                            }
                        }
                    }
                }
                ServerEvent::AppIdChanged { surface, app_id } => {
                    if let Some(win) = state
                        .windows
                        .iter_mut()
                        .find(|w| w.wl_surface.as_ref() == Some(&surface))
                    {
                        win.app_id = app_id;
                    }
                }
                ServerEvent::DecorationModeChanged { .. } => { /* handled in dispatch timer path */ }
                ServerEvent::DecorationModeChanged { toplevel_id, mode } => {
                    if let Some(win) = state.windows.iter().find(|w| w.xdg_toplevel.as_ref().map(|t| t.id().protocol_id()) == Some(toplevel_id)) {
                        if let Some(id) = win.axiom_id {
                            let decorated = matches!(mode, zxdg_toplevel_decoration_v1::Mode::ServerSide);
                            let _ = self.window_manager.write().set_window_decorated(id, decorated);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn apply_layouts(&mut self, state: &mut CompositorState) -> Result<()> {
        // Compute layouts from workspace manager and push size configures to clients
        let layouts: HashMap<u64, crate::window::Rectangle> = {
            let ws = self.workspace_manager.read();
            ws.calculate_workspace_layouts()
        };

        // Preserve layer layouts; refresh only window layouts
        state.last_layouts.retain(|k, _| *k >= 1_000_000u64);
        state.last_layouts.extend(layouts.iter().map(|(k, v)| (*k, v.clone())));

        for (id, rect) in layouts {
            if let Some(idx) = state.windows.iter().position(|w| w.axiom_id == Some(id)) {
                let serial = state.next_serial();
                // Clone needed role objects without holding a mutable borrow
                let (tl_opt, xdg_surf) = {
                    let w = &state.windows[idx];
                    (w.xdg_toplevel.clone(), w.xdg_surface.clone())
                };
                if let Some(tl) = tl_opt {
let mut states: Vec<u8> = Vec::new();
if state.focused_window_id == Some(id) {
    let activated: u32 = xdg_toplevel::State::Activated as u32;
    states.extend_from_slice(&activated.to_ne_bytes());
}
tl.configure(rect.width as i32, rect.height as i32, states);
                    xdg_surf.configure(serial);
                    // Update serial in a short mutable borrow
                    state.windows[idx].configured_serial = Some(serial);
                }
                // Push placeholder quad to renderer for this window's layout
                crate::renderer::push_placeholder_quad(
                    id,
                    (rect.x as f32, rect.y as f32),
                    (rect.width as f32, rect.height as f32),
                    1.0,
                );
            }
        }
        Ok(())
    }
}

impl CompositorServer {
    fn surface_for_axiom_id(state: &CompositorState, id: u64) -> Option<wl_surface::WlSurface> {
        if let Some(s) = state
            .windows
            .iter()
            .find(|w| w.axiom_id == Some(id))
            .and_then(|w| w.wl_surface.clone())
        {
            return Some(s);
        }
        if let Some(s) = state
            .layer_surfaces
            .iter()
            .find(|e| e.axiom_id == Some(id))
            .map(|e| e.wl_surface.clone())
        {
            return Some(s);
        }
        if let Some(s) = state
            .x11_surfaces
            .iter()
            .find(|e| e.axiom_id == Some(id))
            .map(|e| e.wl_surface.clone())
        {
            return Some(s);
        }
        None
    }

    #[allow(dead_code)]
    fn update_pointer_focus_and_motion(&mut self, state: &mut CompositorState) -> Result<()> {
        // Determine which surface is under the pointer, preferring layer surfaces by priority
        let (px, py) = state.pointer_pos;
        let mut target: Option<(u64, (f64, f64))> = None;
        // Check layer surfaces in priority order
        let mut check_layers = |kinds: &[AxiomLayerKind]| {
            for kind in kinds {
                for e in state.layer_surfaces.iter() {
                    if !e.mapped || e.layer != *kind { continue; }
                    let r = &e.last_geometry;
                    let inside = px >= r.x as f64 && px < (r.x as f64 + r.width as f64) && py >= r.y as f64 && py < (r.y as f64 + r.height as f64);
                    if inside {
                        let local_x = px - r.x as f64;
                        let local_y = py - r.y as f64;
                        if let Some(id) = e.axiom_id { target = Some((id, (local_x, local_y))); return; }
                    }
                }
            }
        };
        check_layers(&[AxiomLayerKind::Overlay, AxiomLayerKind::Top, AxiomLayerKind::Bottom, AxiomLayerKind::Background]);
        // Fallback to tiled windows
        if target.is_none() {
            for (id, rect) in &state.last_layouts {
                if *id >= 1_000_000u64 { continue; } // skip layer ids if present
                let r = rect;
                let inside = px >= r.x as f64 && px < (r.x as f64 + r.width as f64) && py >= r.y as f64 && py < (r.y as f64 + r.height as f64);
                if inside {
                    let local_x = px - r.x as f64;
                    let local_y = py - r.y as f64;
                    target = Some((*id, (local_x, local_y)));
                    break;
                }
            }
        }

        if let Some((id, (lx, ly))) = target {
            if state.pointer_focus_window != Some(id) {
                // Leave previous
                if let Some(prev_id) = state.pointer_focus_window.take() {
                    if let Some(prev_surface) = CompositorServer::surface_for_axiom_id(state, prev_id) {
                        let serial = state.next_serial();
                        for ptr in &state.pointers { ptr.leave(serial, &prev_surface); }
                    }
                }
                // Enter new
                if let Some(surface) = state
                    .windows
                    .iter()
                    .find(|w| w.axiom_id == Some(id))
                    .and_then(|w| w.wl_surface.clone())
                {
                    state.pointer_focus_window = Some(id);
                    let serial = state.next_serial();
                    for ptr in &state.pointers {
                        ptr.enter(serial, &surface, lx, ly);
                    }
                }
            } else {
                // Motion on same surface
                let time_ms: u32 = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() & 0xFFFF_FFFF) as u32;
                for ptr in &state.pointers {
                    ptr.motion(time_ms, lx, ly);
                }
            }
        } else {
            // Outside any window: leave if we had one
            if let Some(prev_id) = state.pointer_focus_window.take() {
                if let Some(prev_surface) = state
                    .windows
                    .iter()
                    .find(|w| w.axiom_id == Some(prev_id))
                    .and_then(|w| w.wl_surface.clone())
                {
                    let serial = state.next_serial();
                    for ptr in &state.pointers {
                        ptr.leave(serial, &prev_surface);
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn handle_pointer_button(&mut self, state: &mut CompositorState, button: u8, pressed: bool) -> Result<()> {
        // Send to focused pointer surface if any
        if let Some(focus_id) = state.pointer_focus_window {
            if let Some(surface) = CompositorServer::surface_for_axiom_id(state, focus_id) {
                // Ensure focus is aligned with click (focus on click)
                if state.focused_window_id != Some(focus_id) {
                    let _ = self.window_manager.write().focus_window(focus_id);
                    state.focused_window_id = Some(focus_id);
                    // Seat focus enter for keyboard
                    let serial = state.next_serial();
                    for kb in &state.keyboards {
                        kb.enter(serial, &surface, vec![]);
                    }
                }
                let time_ms: u32 = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() & 0xFFFF_FFFF) as u32;
                // Linux evdev button codes: BTN_LEFT=272, BTN_RIGHT=273, BTN_MIDDLE=274
                let button_code: u32 = match button {
                    1 => 272,
                    2 => 273,
                    3 => 274,
                    _ => 272,
                };
                let state_flag = if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
                let serial = state.next_serial();
                for ptr in &state.pointers {
                    ptr.button(serial, time_ms, button_code, state_flag);
                }
            }
        }
        Ok(())
    }

    fn spawn_combined_input_threads() -> Option<Receiver<HwInputEvent>> {
        // For now, reuse the evdev/libinput combined thread implementation below
        Self::spawn_evdev_input_thread()
    }

    fn update_pointer_focus_and_motion_inline(state: &mut CompositorState) -> Result<()> {
        let (px, py) = state.pointer_pos;
        let mut target: Option<(u64, (f64, f64))> = None;
        // layers first (overlay->background)
        for kind in [AxiomLayerKind::Overlay, AxiomLayerKind::Top, AxiomLayerKind::Bottom, AxiomLayerKind::Background] {
            for e in state.layer_surfaces.iter() {
                if !e.mapped || e.layer != kind { continue; }
                let r = &e.last_geometry;
                let inside = px >= r.x as f64 && px < (r.x as f64 + r.width as f64) && py >= r.y as f64 && py < (r.y as f64 + r.height as f64);
                if inside { let lx = px - r.x as f64; let ly = py - r.y as f64; if let Some(id) = e.axiom_id { target = Some((id, (lx, ly))); break; } }
            }
            if target.is_some() { break; }
        }
        if target.is_none() {
            for (id, r) in &state.last_layouts { if *id >= 1_000_000u64 { continue; }
                let inside = px >= r.x as f64 && px < (r.x as f64 + r.width as f64) && py >= r.y as f64 && py < (r.y as f64 + r.height as f64);
                if inside { let lx = px - r.x as f64; let ly = py - r.y as f64; target = Some((*id, (lx, ly))); break; }
            }
        }
        if let Some((id, (lx, ly))) = target {
            if state.pointer_focus_window != Some(id) {
                if let Some(prev_id) = state.pointer_focus_window.take() { if let Some(prev_surface) = CompositorServer::surface_for_axiom_id(state, prev_id) { let serial = state.next_serial(); for ptr in &state.pointers { ptr.leave(serial, &prev_surface); } } }
                if let Some(surface) = CompositorServer::surface_for_axiom_id(state, id) { state.pointer_focus_window = Some(id); let serial = state.next_serial(); for ptr in &state.pointers { ptr.enter(serial, &surface, lx, ly); } }
            } else {
                let time_ms: u32 = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() & 0xFFFF_FFFF) as u32; for ptr in &state.pointers { ptr.motion(time_ms, lx, ly); }
            }
        } else if let Some(prev_id) = state.pointer_focus_window.take() { if let Some(prev_surface) = CompositorServer::surface_for_axiom_id(state, prev_id) { let serial = state.next_serial(); for ptr in &state.pointers { ptr.leave(serial, &prev_surface); } } }
        Ok(())
    }

    fn spawn_evdev_input_thread() -> Option<Receiver<HwInputEvent>> {
        use evdev::{Device, EventType, Key, RelativeAxisType};
        use std::fs;
        let (tx, rx) = mpsc::channel::<HwInputEvent>();
        // No libinput thread; evdev-based input fallback below
        // Original evdev scanning for fallback
        // Try scanning /dev/input
        let paths = fs::read_dir("/dev/input").ok()?;
        let mut dev_paths = Vec::new();
        for entry in paths.flatten() {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with("event") {
                    dev_paths.push(p);
                }
            }
        }
        if dev_paths.is_empty() { return Some(rx); }

        std::thread::spawn(move || {
            // Open devices best-effort
            let mut keyboards = Vec::new();
            let mut pointers = Vec::new();
            for p in dev_paths {
                if let Ok(d) = Device::open(&p) {
                    let has_keys = d.supported_events().contains(EventType::KEY);
                    let has_rel = d.supported_events().contains(EventType::RELATIVE);
let has_btn = d.supported_keys().is_some_and(|k| k.contains(Key::BTN_LEFT) || k.contains(Key::BTN_RIGHT) || k.contains(Key::BTN_MIDDLE));
                    if has_keys && !has_rel { keyboards.push(d); }
                    else if has_rel || has_btn { pointers.push(d); }
                }
            }

            use std::collections::HashSet;
            let mut mods: HashSet<&'static str> = HashSet::new();
            loop {
                // Process keyboards
                for dev in &mut keyboards {
                    if let Ok(events) = dev.fetch_events() {
                        for ev in events {
                            if ev.event_type() == EventType::KEY {
                                let code = ev.code();
                                let value = ev.value();
                                let pressed = value != 0;
                                // Track modifiers
                                match Key::new(code) {
                                    Key::KEY_LEFTCTRL | Key::KEY_RIGHTCTRL => { if pressed { mods.insert("Ctrl"); } else { mods.remove("Ctrl"); } }
                                    Key::KEY_LEFTALT | Key::KEY_RIGHTALT => { if pressed { mods.insert("Alt"); } else { mods.remove("Alt"); } }
                                    Key::KEY_LEFTSHIFT | Key::KEY_RIGHTSHIFT => { if pressed { mods.insert("Shift"); } else { mods.remove("Shift"); } }
                                    Key::KEY_LEFTMETA | Key::KEY_RIGHTMETA => { if pressed { mods.insert("Super"); } else { mods.remove("Super"); } }
                                    _ => {}
                                }
                                let key_name: Option<&'static str> = match Key::new(code) {
                                    Key::KEY_LEFT => Some("Left"),
                                    Key::KEY_RIGHT => Some("Right"),
                                    Key::KEY_UP => Some("Up"),
                                    Key::KEY_DOWN => Some("Down"),
                                    Key::KEY_H => Some("H"),
                                    Key::KEY_L => Some("L"),
                                    Key::KEY_J => Some("J"),
                                    Key::KEY_K => Some("K"),
                                    Key::KEY_F11 => Some("F11"),
                                    _ => None,
                                };
                                if let Some(name) = key_name {
                                    let modifiers: Vec<String> = mods.iter().map(|s| s.to_string()).collect();
                                    let _ = tx.send(HwInputEvent::Key { key: name.to_string(), modifiers, pressed });
                                }
                            }
                        }
                    }
                }
                // Process pointers
                for dev in &mut pointers {
                    if let Ok(events) = dev.fetch_events() {
                        let mut dx = 0.0f64;
                        let mut dy = 0.0f64;
                        let mut hscroll = 0.0f64;
                        let mut vscroll = 0.0f64;
                        for ev in events {
                            match ev.event_type() {
                                EventType::RELATIVE => {
                                    if ev.code() == RelativeAxisType::REL_X.0 { dx += ev.value() as f64; }
                                    if ev.code() == RelativeAxisType::REL_Y.0 { dy += ev.value() as f64; }
                                    if ev.code() == RelativeAxisType::REL_HWHEEL.0 { hscroll += ev.value() as f64; }
                                    if ev.code() == RelativeAxisType::REL_WHEEL.0 { vscroll += ev.value() as f64; }
                                }
                                EventType::KEY => {
                                    let k = Key::new(ev.code());
                                    let pressed = ev.value() != 0;
                                    let btn = match k {
                                        Key::BTN_LEFT => Some(1u8),
                                        Key::BTN_RIGHT => Some(2u8),
                                        Key::BTN_MIDDLE => Some(3u8),
                                        _ => None,
                                    };
                                    if let Some(b) = btn { let _ = tx.send(HwInputEvent::PointerButton { button: b, pressed }); }
                                }
                                _ => {}
                            }
                        }
                        if dx != 0.0 || dy != 0.0 { let _ = tx.send(HwInputEvent::PointerMotion { dx, dy }); }
                        if hscroll != 0.0 || vscroll != 0.0 { let _ = tx.send(HwInputEvent::PointerAxis { horizontal: hscroll, vertical: vscroll }); }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });

        Some(rx)
    }
}

// wl_compositor global
impl GlobalDispatch<wl_compositor::WlCompositor, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_compositor::WlCompositor>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}
impl Dispatch<wl_compositor::WlCompositor, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_compositor::WlCompositor,
        _request: wl_compositor::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// wl_shm global
impl GlobalDispatch<wl_shm::WlShm, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_shm::WlShm>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shm = data_init.init(resource, ());
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
    }
}
impl Dispatch<wl_shm::WlShm, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_shm::WlShm,
        request: wl_shm::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, size } = request {
                // Map the file descriptor
                let file: File = fd.into();
                match unsafe { Mmap::map(&file) } {
                    Ok(map) => {
let pool_data = ShmPoolData { map: Arc::new(map), _size: size };
                        data_init.init(id, pool_data);
                    }
                    Err(_e) => {
                        // Failed to map; still init to avoid protocol errors with a tiny anon map
                        let anon = MmapOptions::new().len(1).map_anon().unwrap();
                        let ro = anon.make_read_only().unwrap();
let pool_data = ShmPoolData { map: Arc::new(ro), _size: 0 };
                        data_init.init(id, pool_data);
                    }
                }
                // File drops here; mapping remains valid
            }
    }
}

// wl_seat global
impl GlobalDispatch<wl_seat::WlSeat, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_seat::WlSeat>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let seat = data_init.init(resource, ());
        seat.capabilities(wl_seat::Capability::Keyboard | wl_seat::Capability::Pointer | wl_seat::Capability::Touch);
        seat.name(state.seat_name.clone());
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wl_seat::WlSeat,
        request: wl_seat::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_seat::Request::GetKeyboard { id } => {
                let kb = data_init.init(id, ());
                // Send keymap if available
                if let Some(ref xkb) = state.xkb {
                    if let Ok(fd) = create_memfd_and_write(&xkb.keymap_string) {
                        let size = xkb.keymap_string.len() as u32;
                        let borrowed = fd.as_fd();
                        kb.keymap(wl_keyboard::KeymapFormat::XkbV1, borrowed, size);
                    }
                }
                state.keyboards.push(kb);
            }
            wl_seat::Request::GetPointer { id } => {
                let pt = data_init.init(id, ());
                state.pointers.push(pt);
            }
            wl_seat::Request::GetTouch { id } => {
                let t = data_init.init(id, ());
                state.touches.push(t);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_keyboard::WlKeyboard,
        _request: wl_keyboard::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { }
}

#[derive(Clone)]
struct XkbInfo {
    keymap_string: String,
}

fn build_default_xkb_info() -> Option<XkbInfo> {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let rules = String::new();
    let model = String::new();
    let layout = String::from("us");
    let variant = String::new();
    let options = String::new();
    let keymap = xkb::Keymap::new_from_names(
        &ctx,
        &rules,
        &model,
        &layout,
        &variant,
        Some(options),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )?;
    let km_str = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    Some(XkbInfo { keymap_string: km_str })
}

fn create_memfd_and_write(data: &str) -> std::io::Result<OwnedFd> {
    // Use memfd where available; fall back to anonymous tmp file
    #[cfg(target_os = "linux")]
    {
        let name = CString::new("axiom-xkb-keymap").unwrap();
        let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(data.as_bytes())?;
        let ofd = unsafe { OwnedFd::from_raw_fd(file.into_raw_fd()) };
        Ok(ofd)
    }
    #[cfg(not(target_os = "linux"))]
    {
        use std::fs::OpenOptions;
        use std::io::Seek;
        use std::io::SeekFrom;
        let mut tmp = OpenOptions::new().read(true).write(true).create(true).open("/tmp/axiom-keymap")?;
        tmp.set_len(0)?;
        tmp.seek(SeekFrom::Start(0))?;
        tmp.write_all(data.as_bytes())?;
        let ofd = unsafe { OwnedFd::from_raw_fd(tmp.into_raw_fd()) };
        Ok(ofd)
    }
}

impl CompositorServer {
    fn send_modifiers(&mut self, state: &mut CompositorState, modifiers: &Vec<String>) {
        // Map active modifiers to wl_keyboard::Modifier masks as best-effort
        let mut depressed: u32 = 0;
        if modifiers.iter().any(|m| m == "Shift") { depressed |= 1 << 0; }
        if modifiers.iter().any(|m| m == "Ctrl") { depressed |= 1 << 2; }
        if modifiers.iter().any(|m| m == "Alt") { depressed |= 1 << 3; }
        if modifiers.iter().any(|m| m == "Super") { depressed |= 1 << 6; }
        let latched = 0;
        let locked = 0;
        let group = 0;
        let serial = state.next_serial();
        for kb in &state.keyboards {
            kb.modifiers(serial, depressed, latched, locked, group);
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_pointer::WlPointer,
        _request: wl_pointer::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { }
}

impl Dispatch<wl_touch::WlTouch, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_touch::WlTouch,
        _request: wl_touch::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { }
}

impl Dispatch<wl_callback::WlCallback, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_callback::WlCallback,
        _request: wl_callback::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { }
}

// Clipboard and DnD: wl_data_device_manager v3
#[derive(Clone)]
struct DataSourceEntry {
    pub resource: wl_data_source::WlDataSource,
    pub mime_types: Vec<String>,
}

#[derive(Clone, Default)]
struct DataOfferEntry {
    pub from_source_id: Option<u32>,
    pub server_text: Option<String>,
    pub mime_types: Vec<String>,
}

#[derive(Clone)]
enum SelectionState {
    Client { source_id: u32 },
    Server { text: String, mime_types: Vec<String> },
}

impl CompositorState {
    fn current_selection_mimes(&self) -> Vec<String> {
        match &self.selection {
            Some(SelectionState::Client { source_id }) => {
                self.data_sources.get(source_id).map(|s| s.mime_types.clone()).unwrap_or_default()
            }
            Some(SelectionState::Server { mime_types, .. }) => mime_types.clone(),
            None => Vec::new(),
        }
    }
}

impl GlobalDispatch<wl_data_device_manager::WlDataDeviceManager, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_data_device_manager::WlDataDeviceManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for CompositorState {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &wl_data_device_manager::WlDataDeviceManager,
        request: wl_data_device_manager::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_device_manager::Request::CreateDataSource { id } => {
                let src: wl_data_source::WlDataSource = data_init.init(id, ());
                let entry = DataSourceEntry { resource: src.clone(), mime_types: Vec::new() };
                state.data_sources.insert(src.id().protocol_id(), entry);
            }
            wl_data_device_manager::Request::GetDataDevice { id, seat: _seat } => {
                let dev: wl_data_device::WlDataDevice = data_init.init(id, ());
                state.data_devices.push(dev.clone());
                // Send current selection to this new device if available
                send_selection_to_device(state, client, dhandle, data_init, &dev);
            }
            _ => {}
        }
    }
}

fn send_selection_to_device(
    state: &mut CompositorState,
    client: &Client,
    dhandle: &DisplayHandle,
    data_init: &mut DataInit<'_, CompositorState>,
    device: &wl_data_device::WlDataDevice,
) {
    // Determine selection: prefer explicit selection, else clipboard text
    let selection = if state.selection.is_some() {
        state.selection.clone()
    } else {
        let clip = state.clipboard.read().get_selection();
        clip.map(|text| SelectionState::Server {
            text,
            mime_types: vec!["text/plain;charset=utf-8".to_string(), "text/plain".to_string()],
        })
    };

    let Some(sel) = selection else { return; };

    // Create a server-side wl_data_offer for this client
    // Use the device's version (cap at 3)
    let version = std::cmp::min(3, device.version());
    if let Ok(offer_res) = client.create_resource::<wl_data_offer::WlDataOffer, (), CompositorState>(dhandle, version, ()) {
        // Track offer metadata
        let (from_source_id, server_text, mime_types) = match &sel {
            SelectionState::Client { source_id } => (Some(*source_id), None, state.current_selection_mimes()),
            SelectionState::Server { text, mime_types } => (None, Some(text.clone()), mime_types.clone()),
        };
        state.active_offers.insert(
            offer_res.id().protocol_id(),
            DataOfferEntry { from_source_id, server_text, mime_types: mime_types.clone() },
        );

        // Send data_offer to client first, then offer mime types, then selection
        device.data_offer(&offer_res);
        for mt in mime_types {
            offer_res.offer(mt);
        }
        device.selection(Some(&offer_res));
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_data_source::WlDataSource,
        request: wl_data_source::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_source::Request::Offer { mime_type } => {
                if let Some(entry) = state.data_sources.get_mut(&resource.id().protocol_id()) {
                    entry.mime_types.push(mime_type);
                }
            }
            wl_data_source::Request::Destroy => {
                // If this was the active selection source, clear selection
                let rid = resource.id().protocol_id();
                if matches!(state.selection, Some(SelectionState::Client { source_id }) if source_id == rid) {
                    state.selection = None;
                    for dev in &state.data_devices { dev.selection(None); }
                }
                state.data_sources.remove(&rid);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for CompositorState {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &wl_data_device::WlDataDevice,
        request: wl_data_device::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_device::Request::SetSelection { source, serial: _serial } => {
                // Cancel previous source if any
                if let Some(SelectionState::Client { source_id: prev }) = &state.selection {
                    if let Some(prev_src) = state.data_sources.get(prev).map(|s| s.resource.clone()) {
                        prev_src.cancelled();
                    }
                }
                if let Some(src) = source {
                    state.selection = Some(SelectionState::Client { source_id: src.id().protocol_id() });
                } else {
                    state.selection = None;
                }
                // Broadcast selection to all devices
                let devices = state.data_devices.clone();
                let sender_client = resource.client().unwrap_or_else(|| client.clone());
                for dev in devices.iter() {
                    send_selection_to_device(state, &sender_client, dhandle, data_init, dev);
                }
            }
            wl_data_device::Request::Release => {
                // No-op for now
            }
            wl_data_device::Request::StartDrag { .. } => {
                // Basic stub for DnD; full DnD path can be implemented later
                debug!("StartDrag requested - basic DnD stub active");
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_data_offer::WlDataOffer,
        request: wl_data_offer::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_offer::Request::Receive { mime_type, fd } => {
                let rid = resource.id().protocol_id();
                if let Some(entry) = state.active_offers.get(&rid).cloned() {
                    if let Some(src_id) = entry.from_source_id {
                        if let Some(src) = state.data_sources.get(&src_id).map(|s| s.resource.clone()) {
                            let borrowed = fd.as_fd();
                            src.send(mime_type, borrowed);
                        }
                    } else if let Some(text) = entry.server_text {
                        // Server-side clipboard: write directly
                        let mut file = unsafe { File::from_raw_fd(fd.into_raw_fd()) };
                        let data = if mime_type.starts_with("text/plain") { text.into_bytes() } else { Vec::new() };
                        let _ = file.write_all(&data);
                    }
                }
            }
            wl_data_offer::Request::Destroy => {
                let rid = resource.id().protocol_id();
                state.active_offers.remove(&rid);
            }
            wl_data_offer::Request::Accept { .. } => {
                // No-op; targets often call Accept before Receive
            }
            wl_data_offer::Request::Finish => {
                // No-op for clipboard
            }
            _ => {}
        }
    }
}

// ===== Primary selection (zwp_primary_selection_device_manager_v1) =====
#[derive(Clone)]
struct PrimarySourceEntry {
    pub resource: zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
    pub mime_types: Vec<String>,
}

#[derive(Clone)]
struct PrimaryOfferEntry {
    pub from_source_id: Option<u32>,
    pub server_text: Option<String>,
    pub mime_types: Vec<String>,
}

#[derive(Clone)]
enum PrimarySelectionState {
    Client { source_id: u32 },
    Server { text: String, mime_types: Vec<String> },
}

impl CompositorState {
    fn current_primary_mimes(&self) -> Vec<String> {
        match &self.primary_selection {
            Some(PrimarySelectionState::Client { source_id }) => {
                self.primary_sources.get(source_id).map(|s| s.mime_types.clone()).unwrap_or_default()
            }
            Some(PrimarySelectionState::Server { mime_types, .. }) => mime_types.clone(),
            None => Vec::new(),
        }
    }
}

impl GlobalDispatch<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        client: &Client,
        _resource: &zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
        request: zwp_primary_selection_device_manager_v1::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_device_manager_v1::Request::CreateSource { id } => {
                let src: zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1 = data_init.init(id, ());
                state.primary_sources.insert(src.id().protocol_id(), PrimarySourceEntry { resource: src, mime_types: Vec::new() });
            }
            zwp_primary_selection_device_manager_v1::Request::GetDevice { id, seat: _seat } => {
                let dev: zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1 = data_init.init(id, ());
                state.primary_devices.push(dev.clone());
                // Send current primary selection to this new device, if any
                send_primary_selection_to_device(state, client, dhandle, data_init, &dev);
            }
            _ => {}
        }
    }
}

fn send_primary_selection_to_device(
    state: &mut CompositorState,
    client: &Client,
    dhandle: &DisplayHandle,
    data_init: &mut DataInit<'_, CompositorState>,
    device: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
) {
    // Use explicit primary selection if set; otherwise, optionally fallback to clipboard text
    let selection = if state.primary_selection.is_some() {
        state.primary_selection.clone()
    } else {
        // Optional fallback to server clipboard. Some apps rely on independent primary selection,
        // but this fallback improves UX until a separate manager is added.
        let clip = state.clipboard.read().get_selection();
        clip.map(|text| PrimarySelectionState::Server {
            text,
            mime_types: vec!["text/plain;charset=utf-8".to_string(), "text/plain".to_string()],
        })
    };
    let Some(sel) = selection else { return; };

    let version = std::cmp::min(1, device.version());
    if let Ok(offer) = client.create_resource::<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, (), CompositorState>(dhandle, version, ()) {
        let (from_source_id, server_text, mime_types) = match &sel {
            PrimarySelectionState::Client { source_id } => (Some(*source_id), None, state.current_primary_mimes()),
            PrimarySelectionState::Server { text, mime_types } => (None, Some(text.clone()), mime_types.clone()),
        };
        state.primary_offers.insert(
            offer.id().protocol_id(),
            PrimaryOfferEntry { from_source_id, server_text, mime_types: mime_types.clone() },
        );
        device.data_offer(&offer);
        for mt in mime_types {
            offer.offer(mt);
        }
        device.selection(Some(&offer));
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        request: zwp_primary_selection_source_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_source_v1::Request::Offer { mime_type } => {
                if let Some(entry) = state.primary_sources.get_mut(&resource.id().protocol_id()) {
                    entry.mime_types.push(mime_type);
                }
            }
            zwp_primary_selection_source_v1::Request::Destroy => {
                let rid = resource.id().protocol_id();
                if matches!(state.primary_selection, Some(PrimarySelectionState::Client { source_id }) if source_id == rid) {
                    state.primary_selection = None;
                    for dev in &state.primary_devices { dev.selection(None); }
                }
                state.primary_sources.remove(&rid);
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        request: zwp_primary_selection_device_v1::Request,
        _data: &(),
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_device_v1::Request::SetSelection { source, serial: _ } => {
                // Cancel previous source (protocol has "cancelled" event on data source if we tracked owner)
                if let Some(PrimarySelectionState::Client { source_id: prev }) = &state.primary_selection {
                    if let Some(prev_src) = state.primary_sources.get(prev).map(|s| s.resource.clone()) {
                        prev_src.cancelled();
                    }
                }
                if let Some(src) = source {
                    state.primary_selection = Some(PrimarySelectionState::Client { source_id: src.id().protocol_id() });
                } else {
                    state.primary_selection = None;
                }
                // Broadcast
                let devices = state.primary_devices.clone();
                let sender_client = resource.client().unwrap_or_else(|| client.clone());
                for dev in devices.iter() {
                    send_primary_selection_to_device(state, &sender_client, dhandle, data_init, dev);
                }
            }
            zwp_primary_selection_device_v1::Request::Destroy => {
                // The device itself is being destroyed by the client; remove tracking entry
                let rid = resource.id().protocol_id();
                state.primary_devices.retain(|d| d.id().protocol_id() != rid);
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        request: zwp_primary_selection_offer_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_offer_v1::Request::Receive { mime_type, fd } => {
                let rid = resource.id().protocol_id();
                if let Some(entry) = state.primary_offers.get(&rid).cloned() {
                    if let Some(src_id) = entry.from_source_id {
                        if let Some(src) = state.primary_sources.get(&src_id).map(|s| s.resource.clone()) {
                            let borrowed = fd.as_fd();
                            src.send(mime_type, borrowed);
                        }
                    } else if let Some(text) = entry.server_text {
                        let mut file = unsafe { File::from_raw_fd(fd.into_raw_fd()) };
                        let data = if mime_type.starts_with("text/plain") { text.into_bytes() } else { Vec::new() };
                        let _ = file.write_all(&data);
                    }
                }
            }
            zwp_primary_selection_offer_v1::Request::Destroy => {
                let rid = resource.id().protocol_id();
                state.primary_offers.remove(&rid);
            }
            _ => {}
        }
    }
}

// Shm pool and buffer handling
#[derive(Clone)]
struct ShmPoolData {
    map: Arc<Mmap>,
    _size: i32,
}

#[derive(Clone)]
enum BufferSource {
    Shm { map: Arc<Mmap>, stride: i32, offset: i32, format: WEnum<wl_shm::Format> },
    Dmabuf { planes: Vec<DmabufPlane>, fourcc: u32 },
}

#[derive(Clone)]
struct DmabufPlane {
    map: Arc<Mmap>,
    stride: i32,
    offset: i32,
}

#[derive(Clone)]
struct BufferRecord {
    id: u32,
    buffer: wl_buffer::WlBuffer,
    width: i32,
    height: i32,
    source: BufferSource,
}

impl CompositorState {
    #[allow(dead_code)]
    fn rid_buffer(buf: &wl_buffer::WlBuffer) -> u32 { buf.id().protocol_id() }
}

#[derive(Clone, Default)]
struct ViewportState {
    // source: x, y, width, height in surface buffer coordinates (float)
    source: Option<(f64, f64, f64, f64)>,
    // destination: width, height in surface-local integers (pixels)
    destination: Option<(u32, u32)>,
}

#[derive(Clone)]
struct ViewportData { surface_id: u32 }

#[derive(Clone, Default)]
struct PositionerState {
    size: Option<(i32, i32)>,
    anchor_rect: Option<(i32, i32, i32, i32)>,
    offset: (i32, i32),
}

fn convert_shm_to_rgba(rec: &BufferRecord) -> Option<Vec<u8>> {
    let width = rec.width.max(0) as usize;
    let height = rec.height.max(0) as usize;
    let (stride, offset, format, map) = match &rec.source {
        BufferSource::Shm { map, stride, offset, format } => (*stride as usize, *offset as usize, format.clone(), map.clone()),
        _ => return None,
    };
    if width == 0 || height == 0 { return None; }
    let needed = offset.checked_add(stride.checked_mul(height)? )?;
    if needed > map.len() { return None; }
    let src = &map[offset..offset + stride * height];
    let mut out = vec![0u8; width * height * 4];
    // wl_shm formats are little-endian
    match format {
        WEnum::Value(wl_shm::Format::Xrgb8888) => {
            for y in 0..height {
                let row = &src[y*stride..y*stride + width*4];
                for x in 0..width {
                    let i = x*4;
                    let b = row[i] as u32;
                    let g = row[i+1] as u32;
                    let r = row[i+2] as u32;
                    // X is row[i+3]
                    let o = (y*width + x)*4;
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = 255u8;
                }
            }
        }
        WEnum::Value(wl_shm::Format::Argb8888) => {
            for y in 0..height {
                let row = &src[y*stride..y*stride + width*4];
                for x in 0..width {
                    let i = x*4;
                    let b = row[i] as u32;
                    let g = row[i+1] as u32;
                    let r = row[i+2] as u32;
                    let a = row[i+3] as u32;
                    let o = (y*width + x)*4;
                    // Assume premultiplied; we just pass through (renderer expects RGBA)
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = a as u8;
                }
            }
        }
        _ => {
            // Unsupported format: skip
            return None;
        }
    }
    Some(out)
}

fn convert_dmabuf_to_rgba(rec: &BufferRecord) -> Option<Vec<u8>> {
    const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // 'XR24'
    const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // 'AR24'
    const DRM_FORMAT_XBGR8888: u32 = 0x34324258; // 'XB24'
    const DRM_FORMAT_ABGR8888: u32 = 0x34324241; // 'AB24'
    const DRM_FORMAT_NV12: u32 = 0x3231564E; // 'NV12'
    let width = rec.width.max(0) as usize;
    let height = rec.height.max(0) as usize;
    let (plane, fourcc) = match &rec.source {
        BufferSource::Dmabuf { planes, fourcc } => (planes.get(0)?, *fourcc),
        _ => return None,
    };
    let stride = plane.stride.max(0) as usize;
    let offset = plane.offset.max(0) as usize;
    if width == 0 || height == 0 { return None; }
    let needed = offset.checked_add(stride.checked_mul(height)? )?;
    if needed > plane.map.len() { return None; }
    let src = &plane.map[offset..offset + stride * height];
    let mut out = vec![0u8; width * height * 4];
    match fourcc {
        DRM_FORMAT_XBGR8888 => {
            for y in 0..height {
                let row = &src[y*stride..y*stride + width*4];
                for x in 0..width {
                    let i = x*4;
                    let b = row[i+2] as u32; // since XBGR in memory little-endian maps to R,G,B order as ABGR? here we interpret bytes as B,G,R,A order; swap R and B to get RGBA
                    let g = row[i+1] as u32;
                    let r = row[i] as u32;
                    let o = (y*width + x)*4;
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = 255u8;
                }
            }
        }
        DRM_FORMAT_ABGR8888 => {
            for y in 0..height {
                let row = &src[y*stride..y*stride + width*4];
                for x in 0..width {
                    let i = x*4;
                    let b = row[i+2] as u32;
                    let g = row[i+1] as u32;
                    let r = row[i] as u32;
                    let a = row[i+3] as u32;
                    let o = (y*width + x)*4;
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = a as u8;
                }
            }
        }
        DRM_FORMAT_XRGB8888 => {
            for y in 0..height {
                let row = &src[y*stride..y*stride + width*4];
                for x in 0..width {
                    let i = x*4;
                    let b = row[i] as u32;
                    let g = row[i+1] as u32;
                    let r = row[i+2] as u32;
                    let o = (y*width + x)*4;
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = 255u8;
                }
            }
        }
        DRM_FORMAT_ARGB8888 => {
            for y in 0..height {
                let row = &src[y*stride..y*stride + width*4];
                for x in 0..width {
                    let i = x*4;
                    let b = row[i] as u32;
                    let g = row[i+1] as u32;
                    let r = row[i+2] as u32;
                    let a = row[i+3] as u32;
                    let o = (y*width + x)*4;
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = a as u8;
                }
            }
        }
        DRM_FORMAT_NV12 => {
            // planes: 0=Y full res, 1=UV interleaved half res
            let planes = match &rec.source { BufferSource::Dmabuf { planes, .. } => planes, _ => unreachable!() };
            if planes.len() < 2 { return None; }
            let y_plane = &planes[0];
            let uv_plane = &planes[1];
            let y_stride = y_plane.stride.max(0) as usize;
            let uv_stride = uv_plane.stride.max(0) as usize;
            let y_ptr = &y_plane.map[y_plane.offset.max(0) as usize..];
            let uv_ptr = &uv_plane.map[uv_plane.offset.max(0) as usize..];
            for y in 0..height {
                let y_row = &y_ptr[y * y_stride..];
                let uv_row = &uv_ptr[(y/2) * uv_stride..];
                for x in 0..width {
                    let Y = y_row[x] as i32;
                    let uv_index = (x/2)*2;
                    let U = uv_row[uv_index] as i32;
                    let V = uv_row[uv_index+1] as i32;
                    // Convert NV12 (YUV 4:2:0) to RGB (BT.601 approx)
                    let c = Y - 16;
                    let d = U - 128;
                    let e = V - 128;
                    let mut r = (298*c + 409*e + 128) >> 8;
                    let mut g = (298*c - 100*d - 208*e + 128) >> 8;
                    let mut b = (298*c + 516*d + 128) >> 8;
                    r = r.clamp(0, 255);
                    g = g.clamp(0, 255);
                    b = b.clamp(0, 255);
                    let o = (y*width + x)*4;
                    out[o] = r as u8;
                    out[o+1] = g as u8;
                    out[o+2] = b as u8;
                    out[o+3] = 255u8;
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

fn process_with_viewport(rec: &BufferRecord, vp: Option<&ViewportState>) -> Option<(Vec<u8>, u32, u32)> {
    // Convert the full buffer first
    let rgba = match rec.source {
        BufferSource::Shm { .. } => convert_shm_to_rgba(rec)?,
        BufferSource::Dmabuf { .. } => convert_dmabuf_to_rgba(rec)?,
    };
    let buf_w = rec.width.max(0) as usize;
    let buf_h = rec.height.max(0) as usize;
    if buf_w == 0 || buf_h == 0 { return None; }

    // Default crop is full buffer
    let (mut sx, mut sy, mut sw, mut sh) = (0.0, 0.0, buf_w as f64, buf_h as f64);
    let mut dw = buf_w as u32;
    let mut dh = buf_h as u32;

    if let Some(v) = vp {
        if let Some((x, y, w, h)) = v.source {
            // Negative width/height means unset per protocol; ignore if <= 0
            if w > 0.0 && h > 0.0 { sx = x; sy = y; sw = w; sh = h; }
        }
        if let Some((w, h)) = v.destination {
            dw = w; dh = h;
        }
    }

    // Clamp crop to buffer
    let sx_i = sx.clamp(0.0, buf_w as f64 - 1.0).floor() as usize;
    let sy_i = sy.clamp(0.0, buf_h as f64 - 1.0).floor() as usize;
    let sw_i = sw.clamp(1.0, buf_w as f64 - sx ).floor() as usize;
    let sh_i = sh.clamp(1.0, buf_h as f64 - sy ).floor() as usize;

    let mut cropped = vec![0u8; sw_i * sh_i * 4];
    for y in 0..sh_i {
        let src_off = ((sy_i + y) * buf_w + sx_i) * 4;
        let dst_off = y * sw_i * 4;
        cropped[dst_off..dst_off + sw_i*4].copy_from_slice(&rgba[src_off..src_off + sw_i*4]);
    }

    // If destination differs, naive nearest-neighbor scale
    if (sw_i as u32, sh_i as u32) != (dw, dh) {
        let mut dst = vec![0u8; dw as usize * dh as usize * 4];
        for y in 0..dh as usize {
            let src_y = (y as f64 * sh_i as f64 / dh as f64).floor() as usize;
            for x in 0..dw as usize {
                let src_x = (x as f64 * sw_i as f64 / dw as f64).floor() as usize;
                let src_idx = (src_y * sw_i + src_x) * 4;
                let dst_idx = (y * dw as usize + x) * 4;
                dst[dst_idx..dst_idx+4].copy_from_slice(&cropped[src_idx..src_idx+4]);
            }
        }
        Some((dst, dw, dh))
    } else {
        Some((cropped, sw_i as u32, sh_i as u32))
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ShmPoolData> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        data: &ShmPoolData,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_shm_pool::Request::CreateBuffer { id, offset, width, height, stride, format } => {
                let buf = data_init.init(id, ());
                let rec = BufferRecord {
                    id: buf.id().protocol_id(),
                    buffer: buf.clone(),
                    width,
                    height,
                    source: BufferSource::Shm { map: data.map.clone(), stride, offset, format },
                };
                state_buffers(state).insert(rec.id, rec);
            }
            wl_shm_pool::Request::Resize { size } => {
                // Not supported in this path
                let _ = size;
            }
            wl_shm_pool::Request::Destroy => {
                // Drop handled automatically
                let _ = resource;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_buffer::WlBuffer,
        request: wl_buffer::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let wl_buffer::Request::Destroy = request {
            if let Some(cell) = BUFFERS_MAP.get() {
                let _ = cell.fetch(state).remove(&resource.id().protocol_id());
            }
        }
    }
}

fn state_buffers(state: &mut CompositorState) -> &mut HashMap<u32, BufferRecord> {
    // Side storage keyed by CompositorState pointer value
    BuffersStorageCell::get_or_init(BuffersStorage::default());
    BUFFERS_MAP.get().unwrap().fetch(state)
}

// Poor-man side storage associated with CompositorState pointer address
#[derive(Default)]
struct BuffersStorage {
    map: HashMap<usize, HashMap<u32, BufferRecord>>,
}
static BUFFERS_MAP: OnceLock<BuffersStorageCell> = OnceLock::new();
struct BuffersStorageCell(std::sync::Mutex<BuffersStorage>);
impl BuffersStorageCell {
    fn get_or_init(default: BuffersStorage) {
        let _ = BUFFERS_MAP.get_or_init(|| BuffersStorageCell(std::sync::Mutex::new(default)));
    }
    fn fetch<'a>(&'a self, state: &'a mut CompositorState) -> &'a mut HashMap<u32, BufferRecord> {
        let key = state as *mut _ as usize;
        let mut guard = self.0.lock().unwrap();
guard.map.entry(key).or_default();
        // SAFETY: We keep the storage for the lifetime of the process
        let ptr: *mut HashMap<u32, BufferRecord> = guard.map.get_mut(&key).unwrap();
        drop(guard);
        unsafe { &mut *ptr }
    }
}

// linux-dmabuf global
impl GlobalDispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let dmabuf = data_init.init(resource, ());
        let ver = dmabuf.version();
        for (fmt, modifier) in &state.dmabuf_formats {
            if ver >= 3 {
                let hi: u32 = (modifier >> 32) as u32;
                let lo: u32 = (*modifier & 0xFFFF_FFFF) as u32;
                dmabuf.modifier(*fmt, hi, lo);
            } else {
                dmabuf.format(*fmt);
            }
        }
    }
}

#[derive(Default)]
struct DmabufParamsData {
    planes: std::sync::Mutex<Vec<(OwnedFd, i32, i32, u32, u32, i32)>>, // (fd, offset, stride, mod_hi, mod_lo, plane_idx)
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        request: zwp_linux_dmabuf_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwp_linux_dmabuf_v1::Request::CreateParams { params_id } = request {
            data_init.init(params_id, DmabufParamsData::default());
        }
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, DmabufParamsData> for CompositorState {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        request: zwp_linux_buffer_params_v1::Request,
        data: &DmabufParamsData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_linux_buffer_params_v1::Request::Add { fd, plane_idx, offset, stride, modifier_hi, modifier_lo } => {
                let owned: OwnedFd = fd.into();
                if let Ok(mut v) = data.planes.lock() { v.push((owned, offset as i32, stride as i32, modifier_hi, modifier_lo, plane_idx as i32)); }
            }
            zwp_linux_buffer_params_v1::Request::CreateImmed { width, height, format, flags: _flags, buffer_id } => {
                // Validate planes
                let mut planes_guard = data.planes.lock().unwrap();
                // Collect planes sorted by plane_idx
                let mut planes = planes_guard.drain(..).collect::<Vec<_>>();
                planes.sort_by_key(|p| p.5);
                // Map planes based on format
                const DRM_FORMAT_XRGB8888: u32 = 0x34325258;
                const DRM_FORMAT_ARGB8888: u32 = 0x34325241;
                const DRM_FORMAT_XBGR8888: u32 = 0x34324258;
                const DRM_FORMAT_ABGR8888: u32 = 0x34324241;
                const DRM_FORMAT_NV12: u32 = 0x3231564E;
                let dm_planes: Vec<DmabufPlane> = match format {
                    DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 | DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888 => {
                        if planes.len() != 1 || planes[0].5 != 0 { resource.failed(); return; }
                        let (owned, offset, stride, _mhi, _mlo, _idx) = planes.remove(0);
                        let file = unsafe { File::from_raw_fd(owned.into_raw_fd()) };
                        let needed = offset.saturating_add(stride.saturating_mul(height.max(0)));
                        let mmap = match unsafe { MmapOptions::new().len(needed as usize).map(&file) } {
                            Ok(m) => Arc::new(m),
                            Err(_) => { resource.failed(); return; }
                        };
                        vec![DmabufPlane { map: mmap, stride, offset }]
                    }
                    DRM_FORMAT_NV12 => {
                        if planes.len() != 2 || planes[0].5 != 0 || planes[1].5 != 1 { resource.failed(); return; }
                        // Plane 0: Y, Plane 1: UV interleaved
                        let (owned0, offset0, stride0, _mhi0, _mlo0, _idx0) = planes.remove(0);
                        let (owned1, offset1, stride1, _mhi1, _mlo1, _idx1) = planes.remove(0);
                        let file0 = unsafe { File::from_raw_fd(owned0.into_raw_fd()) };
                        let file1 = unsafe { File::from_raw_fd(owned1.into_raw_fd()) };
                        let needed0 = offset0.saturating_add(stride0.saturating_mul(height.max(0)));
                        let needed1 = offset1.saturating_add(stride1.saturating_mul((height/2).max(0)));
                        let mmap0 = match unsafe { MmapOptions::new().len(needed0 as usize).map(&file0) } {
                            Ok(m) => Arc::new(m), Err(_) => { resource.failed(); return; }
                        };
                        let mmap1 = match unsafe { MmapOptions::new().len(needed1 as usize).map(&file1) } {
                            Ok(m) => Arc::new(m), Err(_) => { resource.failed(); return; }
                        };
                        vec![DmabufPlane { map: mmap0, stride: stride0, offset: offset0 }, DmabufPlane { map: mmap1, stride: stride1, offset: offset1 }]
                    }
                    _ => { resource.failed(); return; }
                };
                let wlbuf: wl_buffer::WlBuffer = data_init.init(buffer_id, ());
                let rec = BufferRecord {
                    id: wlbuf.id().protocol_id(),
                    buffer: wlbuf.clone(),
                    width,
                    height,
                    source: BufferSource::Dmabuf { planes: dm_planes, fourcc: format },
                };
                state_buffers(state).insert(rec.id, rec);
                // params can be destroyed by client later; done
            }
            zwp_linux_buffer_params_v1::Request::Create { width, height, format, flags: _flags } => {
                // Mirror CreateImmed path
                let mut planes_guard = data.planes.lock().unwrap();
                // Collect and sort
                let mut planes = planes_guard.drain(..).collect::<Vec<_>>();
                planes.sort_by_key(|p| p.5);
                const DRM_FORMAT_XRGB8888: u32 = 0x34325258;
                const DRM_FORMAT_ARGB8888: u32 = 0x34325241;
                const DRM_FORMAT_XBGR8888: u32 = 0x34324258;
                const DRM_FORMAT_ABGR8888: u32 = 0x34324241;
                const DRM_FORMAT_NV12: u32 = 0x3231564E;
                let dm_planes: Vec<DmabufPlane> = match format {
                    DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 | DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888 => {
                        if planes.len() != 1 || planes[0].5 != 0 { resource.failed(); return; }
                        let (owned, offset, stride, _mhi, _mlo, _idx) = planes.remove(0);
                        let file = unsafe { File::from_raw_fd(owned.into_raw_fd()) };
                        let needed = offset.saturating_add(stride.saturating_mul(height.max(0)));
                        let mmap = match unsafe { MmapOptions::new().len(needed as usize).map(&file) } {
                            Ok(m) => Arc::new(m),
                            Err(_) => { resource.failed(); return; }
                        };
                        vec![DmabufPlane { map: mmap, stride, offset }]
                    }
                    DRM_FORMAT_NV12 => {
                        if planes.len() != 2 || planes[0].5 != 0 || planes[1].5 != 1 { resource.failed(); return; }
                        let (owned0, offset0, stride0, _mhi0, _mlo0, _idx0) = planes.remove(0);
                        let (owned1, offset1, stride1, _mhi1, _mlo1, _idx1) = planes.remove(0);
                        let file0 = unsafe { File::from_raw_fd(owned0.into_raw_fd()) };
                        let file1 = unsafe { File::from_raw_fd(owned1.into_raw_fd()) };
                        let needed0 = offset0.saturating_add(stride0.saturating_mul(height.max(0)));
                        let needed1 = offset1.saturating_add(stride1.saturating_mul((height/2).max(0)));
                        let mmap0 = match unsafe { MmapOptions::new().len(needed0 as usize).map(&file0) } {
                            Ok(m) => Arc::new(m), Err(_) => { resource.failed(); return; }
                        };
                        let mmap1 = match unsafe { MmapOptions::new().len(needed1 as usize).map(&file1) } {
                            Ok(m) => Arc::new(m), Err(_) => { resource.failed(); return; }
                        };
                        vec![DmabufPlane { map: mmap0, stride: stride0, offset: offset0 }, DmabufPlane { map: mmap1, stride: stride1, offset: offset1 }]
                    }
                    _ => { resource.failed(); return; }
                };
                // Create wl_buffer via event 'created'
                let version = 1u32;
                let newbuf = match client.create_resource::<wl_buffer::WlBuffer, (), CompositorState>(dhandle, version, ()) { Ok(b) => b, Err(_) => { resource.failed(); return; } };
                let rec = BufferRecord { id: newbuf.id().protocol_id(), buffer: newbuf.clone(), width, height, source: BufferSource::Dmabuf { planes: dm_planes, fourcc: format } };
                state_buffers(state).insert(rec.id, rec);
                resource.created(&newbuf);
            }
            zwp_linux_buffer_params_v1::Request::Destroy => { /* nothing */ }
            _ => {}
        }
    }
}

// wp_presentation_time global
impl GlobalDispatch<wp_presentation::WpPresentation, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wp_presentation::WpPresentation>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}
impl Dispatch<wp_presentation::WpPresentation, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wp_presentation::WpPresentation,
        request: wp_presentation::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wp_presentation::Request::Feedback { surface, callback } = request {
                let fb: wp_presentation_feedback::WpPresentationFeedback = data_init.init(callback, ());
                let sid = surface.id().protocol_id();
                state.presentation_feedbacks.entry(sid).or_default().push(fb);
            }
    }
}

// wp_viewporter global
impl GlobalDispatch<wp_viewporter::WpViewporter, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wp_viewporter::WpViewporter>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wp_viewporter::WpViewporter,
        request: wp_viewporter::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wp_viewporter::Request::GetViewport { id, surface } = request {
                let surface_id = surface.id().protocol_id();
                let _vp = data_init.init(id, ViewportData { surface_id });
                // Initialize default viewport state entry
state.viewport_map.entry(surface_id).or_default();
            }
    }
}
impl Dispatch<wp_viewport::WpViewport, ViewportData> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wp_viewport::WpViewport,
        request: wp_viewport::Request,
        data: &ViewportData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
let entry = state.viewport_map.entry(data.surface_id).or_default();
        match request {
            wp_viewport::Request::SetSource { x, y, width, height } => {
                // Note: protocol uses fixed-point; here values are f32/f64 in server crate
                if width > 0.0 && height > 0.0 { entry.source = Some((x, y, width, height)); }
            }
            wp_viewport::Request::SetDestination { width, height } => {
                if width > 0 && height > 0 { entry.destination = Some((width as u32, height as u32)); }
            }
            wp_viewport::Request::Destroy => {
                state.viewport_map.remove(&data.surface_id);
            }
            _ => {}
        }
    }
}

// ============ WLR Layer Shell ============
impl GlobalDispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        request: zwlr_layer_shell_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_layer_shell_v1::Request::GetLayerSurface { id, surface, output: _output, layer, namespace } => {
                let wlr: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1 = data_init.init(id, ());
let kind = match layer {
                    WEnum::Value(zwlr_layer_shell_v1::Layer::Background) => AxiomLayerKind::Background,
                    WEnum::Value(zwlr_layer_shell_v1::Layer::Bottom) => AxiomLayerKind::Bottom,
                    WEnum::Value(zwlr_layer_shell_v1::Layer::Top) => AxiomLayerKind::Top,
                    WEnum::Value(zwlr_layer_shell_v1::Layer::Overlay) => AxiomLayerKind::Overlay,
                    _ => AxiomLayerKind::Top,
                };
                let entry = LayerSurfaceEntry {
                    wl_surface: surface,
                    wlr_surface: wlr,
                    layer: kind,
                    namespace,
                    anchors: 0,
                    margin_top: 0,
                    margin_right: 0,
                    margin_bottom: 0,
                    margin_left: 0,
                    exclusive_zone: 0,
                    keyboard_interactivity: 0,
                    desired_size: (0, 0),
                    mapped: false,
                    configured_serial: None,
                    axiom_id: None,
                    pending_buffer_id: None,
                    attach_offset: (0, 0),
                    last_geometry: crate::window::Rectangle { x: 0, y: 0, width: 0, height: 0 },
                };
                state.layer_surfaces.push(entry);
                // Send initial configure with a default size; client will ack then commit
                let vw = 1920; let vh = 30;
                let serial = state.next_serial();
                let last = state.layer_surfaces.last().unwrap();
                last.wlr_surface.configure(serial, vw, vh);
                // Note: mapped set on first Commit after AckConfigure
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        request: zwlr_layer_surface_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Find index first to avoid double borrow
        let idx_opt = state.layer_surfaces.iter().position(|e| e.wlr_surface == *resource);
        if let Some(idx) = idx_opt {
            // Work on a short-lived mutable borrow for updates
            {
                let entry = &mut state.layer_surfaces[idx];
                match request {
                    zwlr_layer_surface_v1::Request::SetSize { width, height } => { entry.desired_size = (width as i32, height as i32); }
                    zwlr_layer_surface_v1::Request::SetAnchor { anchor } => { entry.anchors = u32::from(anchor); }
                    zwlr_layer_surface_v1::Request::SetExclusiveZone { zone } => { entry.exclusive_zone = zone; }
                    zwlr_layer_surface_v1::Request::SetMargin { top, right, bottom, left } => { entry.margin_top = top; entry.margin_right = right; entry.margin_bottom = bottom; entry.margin_left = left; }
                    zwlr_layer_surface_v1::Request::SetKeyboardInteractivity { keyboard_interactivity } => { entry.keyboard_interactivity = u32::from(keyboard_interactivity); }
                    zwlr_layer_surface_v1::Request::SetLayer { layer } => {
                        entry.layer = match layer { WEnum::Value(zwlr_layer_shell_v1::Layer::Background) => AxiomLayerKind::Background, WEnum::Value(zwlr_layer_shell_v1::Layer::Bottom) => AxiomLayerKind::Bottom, WEnum::Value(zwlr_layer_shell_v1::Layer::Top) => AxiomLayerKind::Top, WEnum::Value(zwlr_layer_shell_v1::Layer::Overlay) => AxiomLayerKind::Overlay, _ => AxiomLayerKind::Top };
                    }
                    zwlr_layer_surface_v1::Request::AckConfigure { serial } => {
                        entry.configured_serial = Some(serial);
                    }
                    zwlr_layer_surface_v1::Request::Destroy => {
                        // Drop the entry after this scope
                    }
                    _ => {}
                }
            }
            // Handle destroy separately to avoid holding borrow
            if let zwlr_layer_surface_v1::Request::Destroy = request {
                let removed = state.layer_surfaces.remove(idx);
                if let Some(id) = removed.axiom_id { crate::renderer::remove_placeholder_quad(id); }
                return;
            }
            // Compute geometry on an immutable snapshot, then send configure
            let (x, y, w, h, ws_id, layer, excl, anchors) = {
                let entry = &state.layer_surfaces[idx];
                let viewport = (1920i32, 1080i32);
                let (x, y, w, h) = compute_layer_geometry(viewport, entry);
                (x, y, w, h, entry.wlr_surface.clone(), entry.layer, entry.exclusive_zone, entry.anchors)
            };
            // Update reserved insets on workspace based on exclusive zones
            {
                let mut top = 0f64; let mut right = 0f64; let mut bottom = 0f64; let mut left = 0f64;
                if excl > 0 {
                    // Map exclusive zone based on anchors
                    let a_left = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Left)) != 0;
                    let a_right = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Right)) != 0;
                    let a_top = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Top)) != 0;
                    let a_bottom = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Bottom)) != 0;
                    if a_top && !a_bottom { top = excl as f64; }
                    if a_bottom && !a_top { bottom = excl as f64; }
                    if a_left && !a_right { left = excl as f64; }
                    if a_right && !a_left { right = excl as f64; }
                }
                // Apply to workspace reserved insets via manager handle
                {
                    let mut ws_guard = state.workspace_manager_handle.write();
                    ws_guard.update_reserved_insets_max(top, right, bottom, left);
                }
            }
            // Push a placeholder quad at proper z based on layer mapping
            let z = match state.layer_surfaces[idx].layer { AxiomLayerKind::Background => 0.0, AxiomLayerKind::Bottom => 0.05, AxiomLayerKind::Top => 0.98, AxiomLayerKind::Overlay => 0.995 };
            // Allocate an id if needed
            if state.layer_surfaces[idx].axiom_id.is_none() {
                // Use a high id space for layers to avoid colliding with windows
                let nid = 1_000_000u64 + idx as u64;
                state.layer_surfaces[idx].axiom_id = Some(nid);
            }
            let lid = state.layer_surfaces[idx].axiom_id.unwrap();
            crate::renderer::push_placeholder_quad(lid, (x as f32, y as f32), (w as f32, h as f32), z);
            let serial = state.next_serial();
            ws_id.configure(serial, w as u32, h as u32);
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wp_presentation_feedback::WpPresentationFeedback,
        _request: wp_presentation_feedback::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { }
}

// Inline event handling utilities for calloop timer
fn handle_events_inline(
    state: &mut CompositorState,
    wm: &Arc<RwLock<crate::window::WindowManager>>,
    ws: &Arc<RwLock<crate::workspace::ScrollableWorkspaces>>,
) -> Result<()> {
    // Take events out to avoid borrow issues while mutating state
    let mut events = Vec::new();
    events.append(&mut state.events);

    for ev in events {
        match ev {
            ServerEvent::Commit { surface } => {
                // Locate window entry by surface
                if let Some(idx) = state
                    .windows
                    .iter()
                    .position(|w| w.wl_surface.as_ref() == Some(&surface))
                {
                    // Read-only check first
                    let (should_map, title) = {
                        let w = &state.windows[idx];
                        let t = if w.title.is_empty() { "Untitled".to_string() } else { w.title.clone() };
                        (w.configured_serial.is_some() && !w.mapped, t)
                    };

                    if should_map {
                        // Determine if this is a popup or a toplevel
                        let is_popup = { let w = &state.windows[idx]; w.xdg_popup.is_some() };
                        let new_id = { let mut wml = wm.write(); wml.add_window(title) };
                        // For toplevel windows, add to workspace; for popups, skip tiling/workspace placement
                        if !is_popup { let mut wsl = ws.write(); wsl.add_window(new_id); }
                        // Focus only toplevels on map; popups should not steal keyboard focus automatically
                        if !is_popup { let _ = wm.write().focus_window(new_id); }
                        let previous_focus = state.focused_window_id.take();
                        if !is_popup { state.focused_window_id = Some(new_id); }
                        {
                            let win_mut = &mut state.windows[idx];
                            win_mut.axiom_id = Some(new_id);
                            win_mut.mapped = true;
                        }
                        // If popup, assign properties and relationship
                        if is_popup {
                            if let Some(win_id) = state.windows[idx].axiom_id {
                                if let Some(parent_sid) = state.windows[idx].parent_surface_id {
                                    // Find parent entry to get its axiom id
                                    if let Some(parent_id) = state
                                        .windows
                                        .iter()
                                        .find(|w| w.wl_surface.as_ref().map(|s| s.id().protocol_id()) == Some(parent_sid))
                                        .and_then(|w| w.axiom_id)
                                    {
                                        let mut wml = wm.write();
                                        let _ = wml.set_window_layer(win_id, crate::window::WindowLayer::AboveNormal);
                                        let _ = wml.add_popup(win_id, parent_id);
                                        // Do not change keyboard focus
                                    }
                                }
                            }
                        }
                        // Input focus routing (toplevel only)
                        if !is_popup {
                            if let Some(prev_id) = previous_focus {
                                if let Some(prev_surface) = state
                                    .windows
                                    .iter()
                                    .find(|w| w.axiom_id == Some(prev_id))
                                    .and_then(|w| w.wl_surface.clone())
                                {
                                    let serial = state.next_serial();
                                    for kb in &state.keyboards { kb.leave(serial, &prev_surface); }
                                    let serial = state.next_serial();
                                    for ptr in &state.pointers { ptr.leave(serial, &prev_surface); }
                                }
                            }
                            let serial = state.next_serial();
                            for kb in &state.keyboards { kb.enter(serial, &surface, vec![]); }
                            let serial = state.next_serial();
                            for ptr in &state.pointers { ptr.enter(serial, &surface, 0.0, 0.0); }
                        }
                        apply_layouts_inline(state, wm, ws)?;
                    }
                    // After mapping/focus, if a buffer is attached, upload to renderer
                    if let Some(win) = state.windows.iter_mut().find(|w| w.wl_surface.as_ref() == Some(&surface)) {
                        if let (Some(ax_id), Some(buf_id)) = (win.axiom_id, win.pending_buffer_id.take()) {
                            if let Some(rec) = state_buffers(state).get(&buf_id).cloned() {
                                let sid = surface.id().protocol_id();
                                let vp = state.viewport_map.get(&sid).cloned();
                                if let Some((data, w, h)) = process_with_viewport(&rec, vp.as_ref()) {
                                    let sid = surface.id().protocol_id();
                                    if vp.is_none() {
                                        if let Some(damages) = state.damage_map.remove(&sid) {
                                            for (dx, dy, dw, dh) in damages {
                                                if dx >= 0 && dy >= 0 && dw > 0 && dh > 0 {
                                                    let (dxu, dyu, dwu, dhu) = (dx as u32, dy as u32, dw as u32, dh as u32);
                                                    let mut bytes = Vec::with_capacity((dwu * dhu * 4) as usize);
                                                    for row in 0..dhu {
                                                        let src_off = (((dyu + row) * w + dxu) * 4) as usize;
                                                        let end = src_off + (dwu * 4) as usize;
                                                        bytes.extend_from_slice(&data[src_off..end]);
                                                    }
                                                    crate::renderer::queue_texture_update_region(ax_id, w, h, (dxu, dyu, dwu, dhu), bytes);
                                                }
                                            }
                                        } else {
                                            crate::renderer::queue_texture_update(ax_id, data, w, h);
                                        }
                                    } else {
                                        crate::renderer::queue_texture_update(ax_id, data, w, h);
                                    }
                                    rec.buffer.release();
                                }
                            }
                        }
                    }
                } else if let Some(lidx) = state.layer_surfaces.iter().position(|e| e.wl_surface == surface) {
                    // Handle layer-surface commit: map if needed, upload buffer
                    let (mapped, configured, axid_opt, sid, vp) = {
                        let e = &state.layer_surfaces[lidx];
                        (e.mapped, e.configured_serial, e.axiom_id, e.wl_surface.id().protocol_id(), state.viewport_map.get(&e.wl_surface.id().protocol_id()).cloned())
                    };
                    if configured.is_some() && !mapped {
                        // Map layer: assign id and push placeholder
                        let (x, y, w, h, layer) = {
                            let mut e = &mut state.layer_surfaces[lidx];
                            if e.axiom_id.is_none() { e.axiom_id = Some(1_000_000u64 + lidx as u64); }
                            let geom = compute_layer_geometry((1920,1080), e);
                            e.last_geometry = crate::window::Rectangle { x: geom.0, y: geom.1, width: geom.2, height: geom.3 };
                            e.mapped = true;
                            (geom.0, geom.1, geom.2, geom.3, e.layer)
                        };
                        let z = match layer { AxiomLayerKind::Background => 0.0, AxiomLayerKind::Bottom => 0.05, AxiomLayerKind::Top => 0.98, AxiomLayerKind::Overlay => 0.995 };
                        let axid = state.layer_surfaces[lidx].axiom_id.unwrap();
                        crate::renderer::push_placeholder_quad(axid, (x as f32, y as f32), (w as f32, h as f32), z);
                        // Add to hit-test layouts
                        state.last_layouts.insert(axid, state.layer_surfaces[lidx].last_geometry.clone());
                    }
                    // Upload buffer if pending
                    let pbid_opt = { state.layer_surfaces[lidx].pending_buffer_id.take() };
                    if let (Some(axid), Some(buf_id)) = (axid_opt.or(state.layer_surfaces[lidx].axiom_id), pbid_opt) {
                        if let Some(rec) = state_buffers(state).get(&buf_id).cloned() {
                            if let Some((data, w, h)) = process_with_viewport(&rec, vp.as_ref()) {
                                let sid2 = sid;
                                if let Some(damages) = state.damage_map.remove(&sid2) {
                                    for (dx, dy, dw, dh) in damages {
                                        if dx >= 0 && dy >= 0 && dw > 0 && dh > 0 {
                                            let (dxu, dyu, dwu, dhu) = (dx as u32, dy as u32, dw as u32, dh as u32);
                                            let mut bytes = Vec::with_capacity((dwu * dhu * 4) as usize);
                                            for row in 0..dhu {
                                                let src_off = (((dyu + row) * w + dxu) * 4) as usize;
                                                let end = src_off + (dwu * 4) as usize;
                                                bytes.extend_from_slice(&data[src_off..end]);
                                            }
                                            crate::renderer::queue_texture_update_region(axid, w, h, (dxu, dyu, dwu, dhu), bytes);
                                        }
                                    }
                                } else {
                                    crate::renderer::queue_texture_update(axid, data, w, h);
                                }
                                rec.buffer.release();
                            }
                        }
                    }
                } else if let Some(xidx) = state.x11_surfaces.iter().position(|e| e.wl_surface == surface) {
                    // Handle X11 surface commit: map on first commit, then upload buffer
                    if !state.x11_surfaces[xidx].mapped {
                        let new_id = { let mut wml = wm.write(); wml.add_window("X11 Window".to_string()) };
                        { let mut wsl = ws.write(); wsl.add_window(new_id); }
                        state.focused_window_id = Some(new_id);
                        {
                            let e = &mut state.x11_surfaces[xidx];
                            e.mapped = true; e.axiom_id = Some(new_id);
                        }
                        let serial = state.next_serial();
                        for kb in &state.keyboards { kb.enter(serial, &surface, vec![]); }
                        let serial = state.next_serial();
                        for ptr in &state.pointers { ptr.enter(serial, &surface, 0.0, 0.0); }
                        apply_layouts_inline(state, wm, ws)?;
                    }
                    if let Some(buf_id) = { state.x11_surfaces[xidx].pending_buffer_id.take() } {
                        if let Some(rec) = state_buffers(state).get(&buf_id).cloned() {
                            let sid = surface.id().protocol_id();
                            let vp = state.viewport_map.get(&sid).cloned();
                            if let Some((data, w, h)) = process_with_viewport(&rec, vp.as_ref()) {
                                if let Some(axid) = state.x11_surfaces[xidx].axiom_id {
                                    if let Some(damages) = state.damage_map.remove(&sid) {
                                        for (dx, dy, dw, dh) in damages {
                                            if dx >= 0 && dy >= 0 && dw > 0 && dh > 0 {
                                                let (dxu, dyu, dwu, dhu) = (dx as u32, dy as u32, dw as u32, dh as u32);
                                                let mut bytes = Vec::with_capacity((dwu * dhu * 4) as usize);
                                                for row in 0..dhu {
                                                    let src_off = (((dyu + row) * w + dxu) * 4) as usize;
                                                    let end = src_off + (dwu * 4) as usize;
                                                    bytes.extend_from_slice(&data[src_off..end]);
                                                }
                                                crate::renderer::queue_texture_update_region(axid, w, h, (dxu, dyu, dwu, dhu), bytes);
                                            }
                                        }
                                    } else {
                                        crate::renderer::queue_texture_update(axid, data, w, h);
                                    }
                                }
                                rec.buffer.release();
                            }
                        }
                    }
                }
            }
            ServerEvent::Destroy { surface } => {
                if let Some(idx) = state
                    .windows
                    .iter()
                    .position(|w| w.wl_surface.as_ref() == Some(&surface))
                {
                    let entry = state.windows.remove(idx);
                    if let Some(id) = entry.axiom_id {
                        if state.focused_window_id == Some(id) { state.focused_window_id = None; }
                        { let mut wsl = ws.write(); let _ = wsl.remove_window(id); }
                        { let mut wml = wm.write(); let _ = wml.remove_window(id); }
                        if let Some(new_focus_id) = state
                            .windows
                            .iter()
                            .rev()
                            .find_map(|w| w.axiom_id)
                        {
                            let _ = wm.write().focus_window(new_focus_id);
                            state.focused_window_id = Some(new_focus_id);
                            if let Some(focus_surface) = state
                                .windows
                                .iter()
                                .find(|w| w.axiom_id == Some(new_focus_id))
                                .and_then(|w| w.wl_surface.clone())
                            {
                                let serial = state.next_serial();
                                for kb in &state.keyboards { kb.enter(serial, &focus_surface, vec![]); }
                                let serial = state.next_serial();
                                for ptr in &state.pointers { ptr.enter(serial, &focus_surface, 0.0, 0.0); }
                            }
                        }
                        crate::renderer::remove_placeholder_quad(id);
                        apply_layouts_inline(state, wm, ws)?;
                    }
                    // Discard pending presentation feedbacks for this surface
                    let sid = surface.id().protocol_id();
                    if let Some(list) = state.presentation_feedbacks.remove(&sid) { for fb in list { fb.discarded(); } }
                } else if let Some(xidx) = state.x11_surfaces.iter().position(|e| e.wl_surface == surface) {
                    let entry = state.x11_surfaces.remove(xidx);
                    if let Some(id) = entry.axiom_id {
                        if state.focused_window_id == Some(id) { state.focused_window_id = None; }
                        { let mut wsl = ws.write(); let _ = wsl.remove_window(id); }
                        { let mut wml = wm.write(); let _ = wml.remove_window(id); }
                        crate::renderer::remove_placeholder_quad(id);
                        apply_layouts_inline(state, wm, ws)?;
                    }
                    let sid = surface.id().protocol_id();
                    if let Some(list) = state.presentation_feedbacks.remove(&sid) { for fb in list { fb.discarded(); } }
                }
            }
            ServerEvent::TitleChanged { surface, title } => {
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.wl_surface.as_ref() == Some(&surface))
                {
                    win.title = title.clone();
                    if let Some(id) = win.axiom_id {
                        if let Some(w) = wm.write().get_window_mut(id) { w.window.title = title; }
                    }
                }
            }
            ServerEvent::AppIdChanged { surface, app_id } => {
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.wl_surface.as_ref() == Some(&surface))
                {
                    win.app_id = app_id;
                }
            }
            ServerEvent::DecorationModeChanged { .. } => { /* no-op here */ }
        }
    }
    Ok(())
}

fn apply_layouts_inline(
    state: &mut CompositorState,
    wm: &Arc<RwLock<crate::window::WindowManager>>,
    ws: &Arc<RwLock<crate::workspace::ScrollableWorkspaces>>,
) -> Result<()> {
    let layouts: HashMap<u64, crate::window::Rectangle> = { let wsr = ws.read(); wsr.calculate_workspace_layouts() };
    // Preserve layer layouts; refresh only window layouts
    state.last_layouts.retain(|k, _| *k >= 1_000_000u64);
    state.last_layouts.extend(layouts.iter().map(|(k, v)| (*k, v.clone())));
    // Compute z-order map based on window manager stacking/layers
    let z_map: std::collections::HashMap<u64, f32> = {
        let order = wm.read().get_windows_by_render_order();
        let n = order.len().max(1) as f32;
        order.into_iter().enumerate().map(|(i, wid)| {
            let z = 0.1 + (i as f32) / n * 0.9; // 0.1 .. 1.0 range
            (wid, z)
        }).collect()
    };

    for (id, rect) in layouts {
        if let Some(idx) = state.windows.iter().position(|w| w.axiom_id == Some(id)) {
            let serial = state.next_serial();
            let (tl_opt, xdg_surf) = { let w = &state.windows[idx]; (w.xdg_toplevel.clone(), w.xdg_surface.clone()) };
            if let Some(tl) = tl_opt {
                let mut states: Vec<u8> = Vec::new();
                if state.focused_window_id == Some(id) {
                    let activated: u32 = xdg_toplevel::State::Activated as u32;
                    states.extend_from_slice(&activated.to_ne_bytes());
                }
                tl.configure(rect.width as i32, rect.height as i32, states);
                xdg_surf.configure(serial);
                state.windows[idx].configured_serial = Some(serial);
            }
            let z = z_map.get(&id).copied().unwrap_or(1.0);
            crate::renderer::push_placeholder_quad(id, (rect.x as f32, rect.y as f32), (rect.width as f32, rect.height as f32), z);
        }
    }

    // Handle popups: place them relative to their parent
    for w in state.windows.iter() {
        if w.xdg_popup.is_some() {
            if let (Some(id), Some(pid)) = (w.axiom_id, w.parent_surface_id) {
                // Find parent window id
                if let Some(parent_axiom_id) = state
                    .windows
                    .iter()
                    .find(|p| p.wl_surface.as_ref().map(|s| s.id().protocol_id()) == Some(pid))
                    .and_then(|p| p.axiom_id)
                {
                    if let Some(parent_rect) = state.last_layouts.get(&parent_axiom_id) {
                        // Use positioner state for size/offset
                        let (mut x, mut y, mut wdt, mut hgt) = (0i32, 0i32, 300u32, 200u32);
                        if let Some(pos_id) = w.positioner_id {
                            if let Some(pos) = state.positioners.get(&pos_id) {
                                if let Some((ax, ay, _aw, _ah)) = pos.anchor_rect { x = ax; y = ay; }
                                x += w.attach_offset.0;
                                y += w.attach_offset.1;
                                if let Some((sw, sh)) = pos.size { wdt = sw as u32; hgt = sh as u32; }
                            }
                        }
                        let gx = parent_rect.x + x;
                        let gy = parent_rect.y + y;
                        let z = z_map.get(&id).copied().unwrap_or(0.95);
                        crate::renderer::push_placeholder_quad(id, (gx as f32, gy as f32), (wdt as f32, hgt as f32), z);
                    }
                }
            }
        }
    }
    Ok(())
}

// wl_output global
impl GlobalDispatch<wl_output::WlOutput, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_output::WlOutput>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output = data_init.init(resource, ());
        output.geometry(
            0,
            0,
            300,
            200,
            wl_output::Subpixel::Unknown,
            "Axiom".to_string(),
            "Unified".to_string(),
            wl_output::Transform::Normal,
        );
        output.mode(wl_output::Mode::Current | wl_output::Mode::Preferred, 1920, 1080, 60000);
        output.scale(1);
        output.done();
    }
}
impl Dispatch<wl_output::WlOutput, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_output::WlOutput,
        _request: wl_output::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// zxdg_decoration_manager_v1 global
impl GlobalDispatch<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}
impl Dispatch<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        request: zxdg_decoration_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, toplevel } => {
                let deco: zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1 = data_init.init(id, ());
                let tl_id = toplevel.id().protocol_id();
                let did = deco.id().protocol_id();
                state.toplevel_decorations.insert(tl_id, deco.clone());
                state.decoration_to_toplevel.insert(did, tl_id);
                let mode = if state.force_client_side_decorations {
                    zxdg_toplevel_decoration_v1::Mode::ClientSide
                } else {
                    zxdg_toplevel_decoration_v1::Mode::ServerSide
                };
                state.decoration_modes.insert(tl_id, mode);
                deco.configure(mode);
                debug!("xdg-decoration: created for toplevel={}, initial mode={:?}", tl_id, mode);
            }
            zxdg_decoration_manager_v1::Request::Destroy => { /* no-op */ }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        request: zxdg_toplevel_decoration_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let deco_id = resource.id().protocol_id();
        match request {
            zxdg_toplevel_decoration_v1::Request::SetMode { mode } => {
                if let Some(&tl_id) = state.decoration_to_toplevel.get(&deco_id) {
                    let requested = match mode { WEnum::Value(m) => m, _ => zxdg_toplevel_decoration_v1::Mode::ServerSide };
                    let chosen = if state.force_client_side_decorations { zxdg_toplevel_decoration_v1::Mode::ClientSide } else { requested };
                    if let Some(deco) = state.toplevel_decorations.get(&tl_id) { deco.configure(chosen); }
                    state.decoration_modes.insert(tl_id, chosen);
                    state.events.push(ServerEvent::DecorationModeChanged { toplevel_id: tl_id, mode: chosen });
                    debug!("xdg-decoration: set_mode for toplevel={} requested={:?} chosen={:?}", tl_id, mode, chosen);
                }
            }
            zxdg_toplevel_decoration_v1::Request::UnsetMode => {
                if let Some(&tl_id) = state.decoration_to_toplevel.get(&deco_id) {
                    let chosen = if state.force_client_side_decorations { zxdg_toplevel_decoration_v1::Mode::ClientSide } else { zxdg_toplevel_decoration_v1::Mode::ServerSide };
                    if let Some(deco) = state.toplevel_decorations.get(&tl_id) { deco.configure(chosen); }
                    state.decoration_modes.insert(tl_id, chosen);
                    state.events.push(ServerEvent::DecorationModeChanged { toplevel_id: tl_id, mode: chosen });
                    debug!("xdg-decoration: unset_mode for toplevel={} fallback={:?}", tl_id, chosen);
                }
            }
            zxdg_toplevel_decoration_v1::Request::Destroy => {
                if let Some(tl_id) = state.decoration_to_toplevel.remove(&deco_id) {
                    state.toplevel_decorations.remove(&tl_id);
                    state.decoration_modes.remove(&tl_id);
                    debug!("xdg-decoration: destroyed for toplevel={}", tl_id);
                }
            }
            _ => {}
        }
    }
}

// Helpers
impl CompositorState {
    fn next_serial(&mut self) -> u32 {
        let s = self.serial_counter;
        self.serial_counter = self.serial_counter.wrapping_add(1);
        s
    }
}

// xdg_wm_base global
impl GlobalDispatch<xdg_wm_base::XdgWmBase, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<xdg_wm_base::XdgWmBase>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let base = data_init.init(resource, ());
        state.xdg_bases.push(base);
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &xdg_wm_base::XdgWmBase,
        request: xdg_wm_base::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
xdg_wm_base::Request::GetXdgSurface { id, surface } => {
                let xdg = data_init.init(id, ());
                debug!("xdg_surface created for wl_surface");
                state.windows.push(WindowEntry {
                    xdg_surface: xdg.clone(),
                    xdg_toplevel: None,
                    xdg_popup: None,
                    wl_surface: Some(surface),
                    configured_serial: None,
                    mapped: false,
                    title: String::new(),
                    app_id: String::new(),
                    axiom_id: None,
                    pending_buffer_id: None,
                    attach_offset: (0, 0),
                    parent_surface_id: None,
                    positioner_id: None,
                    window_type: crate::window::WindowType::Normal,
                });
            }
            xdg_wm_base::Request::CreatePositioner { id } => { 
                let pos_res = data_init.init(id, ());
                let pid = pos_res.id().protocol_id();
                state.positioners.insert(pid, PositionerState::default());
            }
            xdg_wm_base::Request::Pong { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<xdg_positioner::XdgPositioner, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &xdg_positioner::XdgPositioner,
        request: xdg_positioner::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { 
        let pid = resource.id().protocol_id();
        let entry = state.positioners.entry(pid).or_default();
        match request {
            xdg_positioner::Request::SetSize { width, height } => { if width > 0 && height > 0 { entry.size = Some((width, height)); } }
            xdg_positioner::Request::SetAnchorRect { x, y, width, height } => { entry.anchor_rect = Some((x, y, width, height)); }
            xdg_positioner::Request::SetOffset { x, y } => { entry.offset = (x, y); }
            xdg_positioner::Request::Destroy => { state.positioners.remove(&pid); }
            _ => {}
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &xdg_popup::XdgPopup,
        request: xdg_popup::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_popup::Request::Grab { .. } => {
                // For now ignore implicit grab handling
            }
            xdg_popup::Request::Destroy => {
                // Cleanup any associated positioner
                let _ = state.positioners.remove(&resource.id().protocol_id());
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &xdg_surface::XdgSurface,
        request: xdg_surface::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_surface::Request::GetToplevel { id } => {
                let toplevel = data_init.init(id, ());
                // Precompute serial to avoid borrow conflict
                let serial = state.next_serial();
                if let Some(win) = state.windows.iter_mut().rev().find(|w| w.xdg_surface == *resource && w.xdg_toplevel.is_none()) {
                    win.xdg_toplevel = Some(toplevel.clone());
                    // send initial configure
                    toplevel.configure(800, 600, vec![]);
                    win.configured_serial = Some(serial);
                    resource.configure(serial);
                    win.mapped = false;
                    debug!("xdg_toplevel created; initial configure serial={}", serial);
                }
            }
xdg_surface::Request::GetPopup { id, parent: _parent, positioner } => {
                let popup = data_init.init(id, ());
                let pid = positioner.id().protocol_id();
// parent is unused in this stub; metadata is stored later via wl_surface relationships
                let (mut x, mut y, mut w, mut h) = (0, 0, 300, 200);
                if let Some(pos) = state.positioners.get(&pid) {
                    if let Some((ax, ay, aw, ah)) = pos.anchor_rect { let _ = aw; let _ = ah; x = ax + pos.offset.0; y = ay + pos.offset.1; }
                    if let Some((sw, sh)) = pos.size { w = sw; h = sh; }
                }
                                // Position is relative to the parent surface local coords
                popup.configure(x, y, w, h);
                let serial = state.next_serial();
                resource.configure(serial);
                // Mark this xdg_surface entry as a popup and store metadata
                if let Some(win) = state.windows.iter_mut().rev().find(|w| w.xdg_surface == *resource && w.xdg_popup.is_none()) {
                    win.xdg_popup = Some(popup.clone());
// parent surface id will be associated during Commit via wl_surface tree
                    win.positioner_id = Some(pid);
                    win.window_type = crate::window::WindowType::Popup;
                }
                debug!("xdg_popup configured at ({}, {}) size {}x{}", x, y, w, h);
            }
            xdg_surface::Request::AckConfigure { serial } => {
                if let Some(win) = state.windows.iter_mut().find(|w| w.xdg_surface == *resource) {
                    win.configured_serial = Some(serial);
                    debug!("xdg_surface ack_configure serial={}", serial);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &xdg_toplevel::XdgToplevel,
        request: xdg_toplevel::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_toplevel::Request::SetTitle { title } => {
                if let Some(win) = state.windows.iter_mut().find(|w| w.xdg_toplevel.as_ref() == Some(resource)) {
                    win.title = title.clone();
                    if let Some(ref surface) = win.wl_surface {
                        state.events.push(ServerEvent::TitleChanged { surface: surface.clone(), title: win.title.clone() });
                    }
                    debug!("toplevel title={}", win.title);
                }
            }
            xdg_toplevel::Request::SetAppId { app_id } => {
                if let Some(win) = state.windows.iter_mut().find(|w| w.xdg_toplevel.as_ref() == Some(resource)) {
                    win.app_id = app_id.clone();
                    if let Some(ref surface) = win.wl_surface {
                        state.events.push(ServerEvent::AppIdChanged { surface: surface.clone(), app_id: win.app_id.clone() });
                    }
                    debug!("toplevel app_id={}", win.app_id);
                }
            }
            xdg_toplevel::Request::Destroy => {
                // Treat as surface destroy/unmap request
                if let Some(win) = state.windows.iter().find(|w| w.xdg_toplevel.as_ref() == Some(resource)) {
                    if let Some(ref surface) = win.wl_surface {
                        state.events.push(ServerEvent::Destroy { surface: surface.clone() });
                    }
                }
            }
            _ => {}
        }
    }
}

fn compute_layer_geometry(viewport: (i32, i32), entry: &LayerSurfaceEntry) -> (i32, i32, u32, u32) {
    let (vw, vh) = viewport;
    let mut w = if entry.desired_size.0 > 0 { entry.desired_size.0 } else { 0 };
    let mut h = if entry.desired_size.1 > 0 { entry.desired_size.1 } else { 0 };
    let anchors = entry.anchors;
let anchor_left = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Left)) != 0;
    let anchor_right = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Right)) != 0;
    let anchor_top = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Top)) != 0;
    let anchor_bottom = (anchors & u32::from(zwlr_layer_surface_v1::Anchor::Bottom)) != 0;
    if w == 0 && anchor_left && anchor_right { w = vw; }
    if h == 0 && anchor_top && anchor_bottom { h = vh; }
    if w == 0 { w = vw; }
    if h == 0 { h = 30; }
    let mut x = if anchor_left { 0 } else if anchor_right { vw - w } else { (vw - w) / 2 };
    let mut y = if anchor_top { 0 } else if anchor_bottom { vh - h } else { (vh - h) / 2 };
    x += entry.margin_left - entry.margin_right;
    y += entry.margin_top - entry.margin_bottom;
    (x.max(0), y.max(0), w as u32, h as u32)
}

impl Dispatch<wl_surface::WlSurface, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Attach { buffer, x, y } => {
                if let Some(win) = state.windows.iter_mut().find(|w| w.wl_surface.as_ref() == Some(resource)) {
                    win.pending_buffer_id = buffer.as_ref().map(|b| b.id().protocol_id());
                    win.attach_offset = (x, y);
                } else if let Some(entry) = state.layer_surfaces.iter_mut().find(|e| &e.wl_surface == resource) {
                    entry.pending_buffer_id = buffer.as_ref().map(|b| b.id().protocol_id());
                    entry.attach_offset = (x, y);
                } else if let Some(x11e) = state.x11_surfaces.iter_mut().find(|e| &e.wl_surface == resource) {
                    x11e.pending_buffer_id = buffer.as_ref().map(|b| b.id().protocol_id());
                    x11e.attach_offset = (x, y);
                } else {
                    // Track as a potential XWayland surface
                    state.x11_surfaces.push(X11SurfaceEntry { wl_surface: resource.clone(), mapped: false, pending_buffer_id: buffer.as_ref().map(|b| b.id().protocol_id()), attach_offset: (x, y), axiom_id: None });
                }
            }
            wl_surface::Request::Damage { x, y, width, height } => {
                let sid = resource.id().protocol_id();
                state.damage_map.entry(sid).or_default().push((x, y, width, height));
            }
            wl_surface::Request::DamageBuffer { x, y, width, height } => {
                let sid = resource.id().protocol_id();
                state.damage_map.entry(sid).or_default().push((x, y, width, height));
            }
            wl_surface::Request::Commit => {
                // Defer manager mutations and input focus to run loop via event bus
                state.events.push(ServerEvent::Commit { surface: resource.clone() });
                // Keep presentation feedbacks pending; they will be presented next frame
            }
            wl_surface::Request::Destroy => {
                state.events.push(ServerEvent::Destroy { surface: resource.clone() });
            }
            wl_surface::Request::Frame { callback } => {
                // Initialize the callback resource and queue it
                let cb: wl_callback::WlCallback = _data_init.init(callback, ());
                state.pending_callbacks.push(cb);
            }
            _ => {}
        }
    }
}
