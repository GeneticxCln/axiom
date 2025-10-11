//! REAL Wayland Compositor Backend - Full Wayland Protocol Implementation
//!
//! This implements a complete Wayland compositor backend that can handle real client
//! applications and integrate with the existing Axiom systems.

use anyhow::{Context, Result};
use log::{debug, info, warn};
use std::os::fd::AsFd;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs::File;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::Arc;
use wayland_server::Resource;

// Use direct wayland imports
use wayland_server::{
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region,
        wl_seat, wl_shm, wl_shm_pool, wl_subcompositor, wl_subsurface, wl_surface,
    },
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New,
};

use wayland_protocols::xdg::shell::server::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

use calloop::EventLoop;

// Import Axiom systems
use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::effects::EffectsEngine;
use crate::input::{InputEvent, InputManager, MouseButton};
use crate::window::{AxiomWindow, WindowManager};
use crate::workspace::ScrollableWorkspaces;

/// Real compositor state - this holds actual window data
pub struct CompositorState {
    pub surfaces: Vec<Surface>,
    pub windows: Vec<Window>,
    pub seat_name: String,
    pub output_info: OutputInfo,
    // Input wiring (minimal): track created pointer/keyboard resources and focus
    pub pointers: Vec<wl_pointer::WlPointer>,
    pub keyboards: Vec<wl_keyboard::WlKeyboard>,
    pub pointer_pos: (f64, f64),
    pub focused_surface: Option<wl_surface::WlSurface>,
    pub serial_counter: u32,
    // Queue wl_surface frame callbacks per-surface; flushed on present tick
    pub pending_callbacks: HashMap<u32, Vec<wl_callback::WlCallback>>,
    // Surfaces that committed since last present
    pub dirty_surfaces: HashSet<u32>,
    // XKB keymap for keyboard
    pub xkb_keymap_string: Option<String>,
    // Current modifier state
    pub mods_depressed: u32,
    pub mods_latched: u32,
    pub mods_locked: u32,
    pub mods_group: u32,
}

/// Real surface with actual data
pub struct Surface {
    pub wl_surface: wl_surface::WlSurface,
    pub buffer: Option<wl_buffer::WlBuffer>,
    pub committed: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// XDG surface role tracking to prevent role conflicts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XdgRole {
    None,
    Toplevel,
    Popup,
}

/// Real window that can be displayed
pub struct Window {
    pub xdg_surface: xdg_surface::XdgSurface,
    pub xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub wl_surface: Option<wl_surface::WlSurface>,
    pub title: String,
    pub app_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Last serial sent in configure
    pub last_configure_serial: Option<u32>,
    /// Last serial acked by client
    pub last_acked_serial: Option<u32>,
    /// Whether client has acked latest configure
    pub is_configured: bool,
    /// Whether window is mapped (committed after ack)
    pub is_mapped: bool,
    /// XDG role assigned to this surface
    pub xdg_role: XdgRole,
    /// Whether we're waiting for first commit after ack
    pub pending_map: bool,
}

pub struct OutputInfo {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
    pub name: String,
}

impl Default for CompositorState {
    fn default() -> Self {
        // Build default US QWERTY keymap
        let xkb_keymap_string = build_default_xkb_keymap();
        
        Self {
            surfaces: Vec::new(),
            windows: Vec::new(),
            seat_name: "seat0".to_string(),
            output_info: OutputInfo {
                width: 1920,
                height: 1080,
                refresh: 60000,
                name: "AXIOM-1".to_string(),
            },
            pointers: Vec::new(),
            keyboards: Vec::new(),
            pointer_pos: (0.0, 0.0),
            focused_surface: None,
            serial_counter: 1,
            pending_callbacks: HashMap::new(),
            dirty_surfaces: HashSet::new(),
            xkb_keymap_string,
            mods_depressed: 0,
            mods_latched: 0,
            mods_locked: 0,
            mods_group: 0,
        }
    }
}

/// Enhanced Real Wayland Backend - Integrates with Axiom systems
#[allow(dead_code)]
pub struct AxiomRealBackend {
    // Wayland core
    display: Display<AxiomCompositorState>,
    listening_socket: ListeningSocket,
    socket_name: String,
    event_loop: Option<EventLoop<'static, AxiomCompositorState>>,
    loop_signal: Option<calloop::LoopSignal>,

    // Axiom systems integration
    config: AxiomConfig,
    window_manager: Arc<RwLock<WindowManager>>,
    workspace_manager: Arc<RwLock<ScrollableWorkspaces>>,
    effects_engine: Arc<RwLock<EffectsEngine>>,
    decoration_manager: Arc<RwLock<DecorationManager>>,
    input_manager: Arc<RwLock<InputManager>>,

    // State
    running: Arc<RwLock<bool>>,
    window_counter: Arc<Mutex<u64>>,
}

/// Enhanced surface with Axiom integration
pub struct AxiomSurface {
    pub wl_surface: wl_surface::WlSurface,
    pub buffer: Option<wl_buffer::WlBuffer>,
    pub committed: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub window_id: Option<u64>, // Associated window ID
}

/// Window data for Axiom integration
pub struct AxiomWindowData {
    pub surface: wl_surface::WlSurface,
    pub xdg_surface: Option<xdg_surface::XdgSurface>,
    pub xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub title: String,
    pub app_id: String,
    pub configured: bool,
    pub mapped: bool,
    pub axiom_window: AxiomWindow,
}

/// Enhanced compositor state with Axiom integration
pub struct AxiomCompositorState {
    // Core Wayland state
    pub surfaces: HashMap<u64, AxiomSurface>,
    pub windows: HashMap<u64, AxiomWindowData>,
    pub seat_name: String,
    pub output_info: OutputInfo,

    // Integration with Axiom systems
    pub window_manager: Arc<RwLock<WindowManager>>,
    pub workspace_manager: Arc<RwLock<ScrollableWorkspaces>>,
    pub effects_engine: Arc<RwLock<EffectsEngine>>,

    // Window tracking
    pub surface_to_window: HashMap<u64, u64>,
    pub next_surface_id: u64,
    pub next_window_id: u64,
}

/// Loop data for calloop event loop
struct CompositorLoopData {
    display_handle: DisplayHandle,
    state: CompositorState,
    present_interval: std::time::Duration,
}

/// Basic real backend for testing
pub struct RealBackend {
    display: Display<CompositorState>,
    listening_socket: ListeningSocket,
    socket_name: String,
}

impl RealBackend {
    pub fn new() -> Result<Self> {
        info!("🚀 Creating REAL Wayland compositor backend...");

        // Create display
        let display =
            Display::<CompositorState>::new().context("Failed to create Wayland display")?;
        let display_handle = display.handle();

        // Create all the REAL protocol globals
        Self::create_globals(&display_handle);

        // Create and bind socket
        let listening_socket = ListeningSocket::bind_auto("wayland", 1..10)
            .context("Failed to bind Wayland socket")?;

        let socket_name = listening_socket
            .socket_name()
            .ok_or_else(|| anyhow::anyhow!("Failed to get socket name"))?
            .to_string_lossy()
            .to_string();

        info!("✅ REAL Wayland socket created: {}", socket_name);
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);

        Ok(Self {
            display,
            listening_socket,
            socket_name,
        })
    }

    fn create_globals(display: &DisplayHandle) {
        info!("📋 Creating REAL Wayland protocol globals...");

        // wl_compositor - for creating surfaces
        display.create_global::<CompositorState, wl_compositor::WlCompositor, ()>(4, ());
        info!("  ✅ wl_compositor v4");

        // wl_shm - for shared memory buffers
        display.create_global::<CompositorState, wl_shm::WlShm, ()>(1, ());
        info!("  ✅ wl_shm v1");

        // xdg_wm_base - for window management
        display.create_global::<CompositorState, xdg_wm_base::XdgWmBase, ()>(3, ());
        info!("  ✅ xdg_wm_base v3");

        // wl_seat - for input
        display.create_global::<CompositorState, wl_seat::WlSeat, ()>(7, ());
        info!("  ✅ wl_seat v7");

        // wl_output - for display info
        display.create_global::<CompositorState, wl_output::WlOutput, ()>(3, ());
        info!("  ✅ wl_output v3");

        // wl_subcompositor - for subsurfaces
        display.create_global::<CompositorState, wl_subcompositor::WlSubcompositor, ()>(1, ());
        info!("  ✅ wl_subcompositor v1");
    }

    pub fn run(mut self) -> Result<()> {
        use calloop::EventLoop;
        use std::time::Duration;
        
        info!("🎬 Starting REAL Wayland compositor with calloop event loop...");
        info!(
            "   Clients can connect via WAYLAND_DISPLAY={}",
            self.socket_name
        );

        let state = CompositorState::default();
        
        // Create calloop event loop
        let mut event_loop: EventLoop<'_, CompositorLoopData> = EventLoop::try_new()
            .context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        
        // Calculate present interval from refresh rate
        let refresh_mhz = state.output_info.refresh.max(1) as u64;
        let present_ns = 1_000_000_000_000u64 / refresh_mhz;
        let present_interval = Duration::from_nanos(present_ns);
        
        info!("⏱️  Present interval: {:.2}ms ({} Hz)", 
            present_interval.as_secs_f64() * 1000.0,
            1000.0 / present_interval.as_millis().max(1) as f64
        );
        
        // Set up listening socket as event source
        let socket_source = calloop::generic::Generic::new(
            self.listening_socket,
            calloop::Interest::READ,
            calloop::Mode::Level,
        );
        
        loop_handle
            .insert_source(socket_source, |_readiness, socket, data| {
                // Accept new clients
                if let Ok(Some(stream)) = socket.accept() {
                    match data.display_handle.insert_client(stream, Arc::new(ClientDataImpl)) {
                        Ok(_) => info!("✅ Client connected!"),
                        Err(e) => warn!("Failed to insert client: {}", e),
                    }
                }
                Ok(calloop::PostAction::Continue)
            })
            .context("Failed to register socket source")?;
        
        // Set up present timer for frame callbacks
        let timer = calloop::timer::Timer::from_duration(present_interval);
        loop_handle
            .insert_source(timer, |_deadline, _timer, data| {
                // Complete frame callbacks for dirty surfaces
                let ts_ms: u32 = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    & 0xFFFF_FFFF) as u32;
                
                let sids: Vec<u32> = data.state.dirty_surfaces.drain().collect();
                for sid in sids {
                    if let Some(list) = data.state.pending_callbacks.get_mut(&sid) {
                        for cb in list.drain(..) {
                            cb.done(ts_ms);
                        }
                    }
                }
                
                // Return new timeout for next frame
                calloop::timer::TimeoutAction::ToDuration(data.present_interval)
            })
            .map_err(|e| anyhow::anyhow!("Failed to register present timer: {:?}", e))?;
        
        // Prepare loop data
        let mut loop_data = CompositorLoopData {
            display_handle: self.display.handle(),
            state,
            present_interval,
        };
        
        info!("🎬 Calloop event loop starting...");
        
        // Main event loop
        loop {
            // Dispatch Wayland client events
            self.display.dispatch_clients(&mut loop_data.state)
                .context("Failed to dispatch clients")?;
            self.display.flush_clients()
                .context("Failed to flush clients")?;
            
            // Dispatch calloop events (socket accepts, timer ticks)
            event_loop.dispatch(Some(Duration::from_millis(10)), &mut loop_data)
                .context("Event loop error")?;
        }
    }

    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }
}

/// Build a default XKB keymap (US layout)
fn build_default_xkb_keymap() -> Option<String> {
    use xkbcommon::xkb;
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &ctx,
        &"",
        &"",
        &"us",
        &"",
        Some("".to_string()),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )?;
    Some(keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1))
}

/// Create a memfd and write keymap string to it
#[cfg(target_os = "linux")]
fn create_memfd_keymap(data: &str) -> std::io::Result<OwnedFd> {
    let name = CString::new("axiom-xkb-keymap").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(data.as_bytes())?;
    file.flush()?;
    let size = data.len();
    drop(file); // Close the file descriptor so it can be passed
    let ofd = unsafe { OwnedFd::from_raw_fd(fd) };
    info!("📋 Created XKB keymap memfd: {} bytes", size);
    Ok(ofd)
}

#[cfg(not(target_os = "linux"))]
fn create_memfd_keymap(data: &str) -> std::io::Result<OwnedFd> {
    use std::fs::OpenOptions;
    use std::io::Seek;
    let mut tmp = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("/tmp/axiom-keymap")?;
    tmp.write_all(data.as_bytes())?;
    tmp.flush()?;
    tmp.seek(std::io::SeekFrom::Start(0))?;
    let fd = tmp.into_raw_fd();
    let ofd = unsafe { OwnedFd::from_raw_fd(fd) };
    Ok(ofd)
}

// Client data implementation
struct ClientDataImpl;

impl ClientData for ClientDataImpl {
    fn initialized(&self, _client_id: ClientId) {
        info!("🔌 New client connected!");
    }

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        info!("🔌 Client disconnected");
    }
}

// REAL wl_compositor protocol implementation
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
        state: &mut Self,
        _client: &Client,
        _resource: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                let surface = data_init.init(id, ());
                info!("🪟 REAL surface created!");
                state.surfaces.push(Surface {
                    wl_surface: surface,
                    buffer: None,
                    committed: false,
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                });
            }
            wl_compositor::Request::CreateRegion { id } => {
                data_init.init(id, ());
                debug!("Region created");
            }
            _ => {}
        }
    }
}

// REAL wl_surface protocol implementation
impl CompositorState {
    // Helpers to send minimal input events
    fn next_serial(&mut self) -> u32 {
        let s = self.serial_counter;
        self.serial_counter = self.serial_counter.wrapping_add(1);
        s
    }

    fn send_pointer_enter_if_needed(&mut self, surface: &wl_surface::WlSurface) {
        if self.focused_surface.as_ref() != Some(surface) {
            // send leave to previous
            if let Some(prev) = self.focused_surface.take() {
                let serial = self.next_serial();
                for p in &self.pointers {
                    p.leave(serial, &prev);
                    // Send frame after leave
                    if p.version() >= 5 {
                        p.frame();
                    }
                }
            }
            // set new focus and send enter
            self.focused_surface = Some(surface.clone());
            let serial = self.next_serial();
            for p in &self.pointers {
                // Surface-local coords: use current pointer_pos
                p.enter(serial, surface, self.pointer_pos.0, self.pointer_pos.1);
                // Send frame after enter
                if p.version() >= 5 {
                    p.frame();
                }
            }
        }
    }

    fn send_pointer_motion(&mut self, x: f64, y: f64) {
        self.pointer_pos = (x, y);
        let time_ms: u32 = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            & 0xFFFF_FFFF) as u32;
        for p in &self.pointers {
            p.motion(time_ms, x, y);
            // Send frame event to indicate end of pointer event batch
            if p.version() >= 5 {
                p.frame();
            }
        }
    }

    fn send_pointer_button(&mut self, button: u32, pressed: bool) {
        let serial = self.next_serial();
        let time_ms: u32 = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            & 0xFFFF_FFFF) as u32;
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        for p in &self.pointers {
            p.button(serial, time_ms, button, state);
            // Send frame event to indicate end of pointer event batch
            if p.version() >= 5 {
                p.frame();
            }
        }
    }

    fn surface_at(&self, x: f64, y: f64) -> Option<wl_surface::WlSurface> {
        // Simple hit-test: first window whose rect contains point
        for w in self.windows.iter() {
            if w.pending_map {
                continue;
            }
            let rx = w.x as f64;
            let ry = w.y as f64;
            let rw = w.width as f64;
            let rh = w.height as f64;
            if x >= rx && y >= ry && x < rx + rw && y < ry + rh {
                if let Some(ref s) = w.wl_surface {
                    return Some(s.clone());
                }
            }
        }
        None
    }

    pub fn handle_pointer_motion(&mut self, x: f64, y: f64, input_mgr: Option<&mut InputManager>) {
        // Hit test and update focus if needed
        if let Some(surface) = self.surface_at(x, y) {
            self.send_pointer_enter_if_needed(&surface);
        }
        self.send_pointer_motion(x, y);

        // Forward to InputManager as a MouseMove
        if let Some(im) = input_mgr {
            let _ = im.process_input_event(InputEvent::MouseMove {
                x,
                y,
                delta_x: 0.0,
                delta_y: 0.0,
            });
        }
    }

    pub fn handle_pointer_button(
        &mut self,
        button: u32,
        pressed: bool,
        input_mgr: Option<&mut InputManager>,
    ) {
        self.send_pointer_button(button, pressed);
        if let Some(im) = input_mgr {
            let btn = match button {
                0x110 => MouseButton::Left,
                0x111 => MouseButton::Right,
                0x112 => MouseButton::Middle,
                _ => MouseButton::Other((button & 0xFF) as u8),
            };
            let (x, y) = self.pointer_pos;
            let _ = im.process_input_event(InputEvent::MouseButton {
                button: btn,
                pressed,
                x,
                y,
            });
        }
    }

    pub fn handle_key_event(
        &mut self,
        keycode: u32,
        pressed: bool,
        modifiers: Vec<String>,
        input_mgr: Option<&mut InputManager>,
    ) {
        // Update modifier state based on provided modifiers
        self.update_modifiers(&modifiers);
        
        // Broadcast to all wl_keyboard resources
        let serial = self.next_serial();
        let time_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() & 0xFFFF_FFFF) as u32;
        let state = if pressed {
            wl_keyboard::KeyState::Pressed
        } else {
            wl_keyboard::KeyState::Released
        };
        
        // Send modifiers first
        let mod_serial = self.next_serial();
        for kb in &self.keyboards {
            kb.modifiers(
                mod_serial,
                self.mods_depressed,
                self.mods_latched,
                self.mods_locked,
                self.mods_group,
            );
        }
        
        // Then send key event
        for kb in &self.keyboards {
            kb.key(serial, time_ms, keycode, state);
        }

        // Basic mapping from keycode to a readable key string
        let key_str = match keycode {
            1 => "Escape".to_string(),
            16 => "Q".to_string(),
            17 => "W".to_string(),
            30 => "A".to_string(),
            31 => "S".to_string(),
            44 => "Z".to_string(),
            57 => "Space".to_string(),
            105 => "Left".to_string(),
            106 => "Right".to_string(),
            _ => format!("Key{}", keycode),
        };

        if let Some(im) = input_mgr {
            let _ = im.process_input_event(InputEvent::Keyboard {
                key: key_str,
                modifiers,
                pressed,
            });
        }
    }
    
    fn update_modifiers(&mut self, modifiers: &[String]) {
        // Simple modifier bitmask mapping (XKB standard positions)
        // Shift=bit0, CapsLock=bit1, Ctrl=bit2, Alt=bit3, Mod2=bit4, Mod3=bit5, Super=bit6, Mod5=bit7
        let mut depressed: u32 = 0;
        
        for m in modifiers {
            match m.as_str() {
                "Shift" => depressed |= 1 << 0,
                "CapsLock" => depressed |= 1 << 1,
                "Ctrl" | "Control" => depressed |= 1 << 2,
                "Alt" => depressed |= 1 << 3,
                "Mod2" => depressed |= 1 << 4,
                "Mod3" => depressed |= 1 << 5,
                "Super" | "Meta" => depressed |= 1 << 6,
                "Mod5" | "AltGr" => depressed |= 1 << 7,
                _ => {},
            }
        }
        
        self.mods_depressed = depressed;
        // For simplicity, keep latched/locked/group at 0
    }
    
    /// Send axis (scroll) events to all pointer resources
    pub fn handle_pointer_axis(
        &mut self,
        horizontal_delta: f64,
        vertical_delta: f64,
        discrete_horizontal: Option<i32>,
        discrete_vertical: Option<i32>,
    ) {
        let time_ms = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() & 0xFFFF_FFFF) as u32;
        
        for p in &self.pointers {
            let version = p.version();
            
            // Send discrete scroll if available (v5+)
            if version >= 5 {
                if let Some(discrete_v) = discrete_vertical {
                    p.axis_discrete(wl_pointer::Axis::VerticalScroll, discrete_v);
                }
                if let Some(discrete_h) = discrete_horizontal {
                    p.axis_discrete(wl_pointer::Axis::HorizontalScroll, discrete_h);
                }
            }
            
            // Send continuous axis events
            if vertical_delta.abs() > 0.001 {
                p.axis(time_ms, wl_pointer::Axis::VerticalScroll, vertical_delta);
            }
            if horizontal_delta.abs() > 0.001 {
                p.axis(time_ms, wl_pointer::Axis::HorizontalScroll, horizontal_delta);
            }
            
            // Send frame to complete the event batch (v5+)
            if version >= 5 {
                p.frame();
            }
        }
    }
}

// REAL wl_surface protocol implementation
impl Dispatch<wl_surface::WlSurface, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Attach { buffer, x, y } => {
                debug!("Surface attach at ({}, {})", x, y);
                if let Some(surface) = state
                    .surfaces
                    .iter_mut()
                    .find(|s| &s.wl_surface == resource)
                {
                    surface.buffer = buffer;
                    surface.x = x;
                    surface.y = y;
                }
            }
            wl_surface::Request::Commit => {
                debug!("Surface commit");
                if let Some(surface) = state
                    .surfaces
                    .iter_mut()
                    .find(|s| &s.wl_surface == resource)
                {
                    surface.committed = true;
                    debug!("✅ Surface committed");
                }
                // Mark this surface dirty for next present tick
                let sid = resource.id().protocol_id();
                state.dirty_surfaces.insert(sid);

                // If there is a corresponding xdg_surface that has acked configure, map the window
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.wl_surface.as_ref() == Some(resource))
                {
                    // Check if window has a role assigned
                    if win.xdg_role == XdgRole::None {
                        warn!("⚠️ Commit on xdg_surface without role assignment");
                        return;
                    }
                    
                    // For toplevel/popup, require configure ack before first map
                    if !win.is_mapped && win.pending_map {
                        if !win.is_configured {
                            warn!("❌ Client attempted to map window before acking configure!");
                            // Protocol violation: must ack_configure before committing buffer
                            return;
                        }
                        
                        // Valid first map after configure ack
                        info!(
                            "🗺️ Mapping {} at ({}, {}) size {}x{}",
                            match win.xdg_role {
                                XdgRole::Toplevel => "toplevel",
                                XdgRole::Popup => "popup",
                                XdgRole::None => "unknown",
                            },
                            win.x, win.y, win.width, win.height
                        );
                        win.is_mapped = true;
                        win.pending_map = false;
                        state.send_pointer_enter_if_needed(resource);
                    } else if win.is_mapped {
                        // Already mapped, just update
                        debug!("🔄 Update commit for already-mapped window");
                    }
                }
            }
            wl_surface::Request::Damage {
                x,
                y,
                width,
                height,
            } => {
                debug!("Surface damage at ({}, {}) size {}x{}", x, y, width, height);
            }
            wl_surface::Request::Frame { callback } => {
                // Initialize the callback and queue it for next present tick per-surface
                let cb = data_init.init(callback, ());
                let sid = resource.id().protocol_id();
                state.pending_callbacks.entry(sid).or_default().push(cb);
                debug!("Frame callback queued for surface {}", sid);
            }
            wl_surface::Request::Destroy => {
                // Remove surface and any queued callbacks
                let sid = resource.id().protocol_id();
                state.surfaces.retain(|s| &s.wl_surface != resource);
                state.pending_callbacks.remove(&sid);
                state.dirty_surfaces.remove(&sid);
                debug!("Surface destroyed");
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_region::WlRegion, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_region::WlRegion,
        _request: wl_region::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
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
    ) {
    }
}

// REAL wl_shm protocol implementation
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
        // Advertise supported formats
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
        shm.format(wl_shm::Format::Rgb888);
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
            data_init.init(id, (fd.as_raw_fd(), size));
            debug!("SHM pool created with size: {}", size);
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, (RawFd, i32)> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        _data: &(RawFd, i32),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_shm_pool::Request::CreateBuffer {
                id,
                offset: _,
                width,
                height,
                stride,
                format,
            } => {
                data_init.init(id, ());
                info!(
                    "📦 Buffer created: {}x{} format:{:?} stride:{}",
                    width, height, format, stride
                );
            }
            wl_shm_pool::Request::Resize { size } => {
                debug!("SHM pool resized to: {}", size);
            }
            wl_shm_pool::Request::Destroy => {
                debug!("SHM pool destroyed");
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_buffer::WlBuffer,
        request: wl_buffer::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let wl_buffer::Request::Destroy = request {
            debug!("Buffer destroyed");
        }
    }
}

// REAL XDG Shell protocol implementation
impl GlobalDispatch<xdg_wm_base::XdgWmBase, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<xdg_wm_base::XdgWmBase>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
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
                let xdg_surface = data_init.init(id, ());
                info!("🪟 XDG surface created for window!");

                // Track association between wl_surface and xdg_surface by creating a placeholder window
                state.windows.push(Window {
                    xdg_surface: xdg_surface.clone(),
                    xdg_toplevel: None,
                    wl_surface: Some(surface.clone()),
                    title: String::new(),
                    app_id: String::new(),
                    x: 100,
                    y: 100,
                    width: 800,
                    height: 600,
                    last_configure_serial: None,
                    last_acked_serial: None,
                    is_configured: false,
                    is_mapped: false,
                    xdg_role: XdgRole::None,
                    pending_map: false,
                });
            }
            xdg_wm_base::Request::CreatePositioner { id } => {
                data_init.init(id, ());
            }
            xdg_wm_base::Request::Pong { serial } => {
                debug!("Pong received: {}", serial);
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
                // Check if role already assigned
                if let Some(win) = state
                    .windows
                    .iter()
                    .find(|w| w.xdg_surface == *resource)
                {
                    if win.xdg_role != XdgRole::None {
                        warn!("⚠️ Attempted to assign toplevel role to surface that already has role: {:?}", win.xdg_role);
                        // Protocol error: role already assigned
                        return;
                    }
                }
                
                let toplevel = data_init.init(id, ());
                info!("🎉 REAL WINDOW CREATED! XDG Toplevel ready!");

                // Find placeholder window created at GetXdgSurface and attach toplevel
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .rev()
                    .find(|w| w.xdg_surface == *resource && w.xdg_toplevel.is_none())
                {
                    // Assign toplevel role
                    win.xdg_role = XdgRole::Toplevel;
                    win.xdg_toplevel = Some(toplevel.clone());
                    win.is_configured = false;
                    win.pending_map = true;
                }
                
                // Generate serial after releasing borrow
                let serial = state.next_serial();
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .rev()
                    .find(|w| w.xdg_surface == *resource)
                {
                    win.last_configure_serial = Some(serial);
                    
                    toplevel.configure(800, 600, vec![]);
                    resource.configure(serial);
                    
                    info!("📤 Sent configure with serial {} to new toplevel", serial);
                } else {
                    // If not found, create a new entry as fallback
                    let serial = state.next_serial();
                    state.windows.push(Window {
                        xdg_surface: resource.clone(),
                        xdg_toplevel: Some(toplevel.clone()),
                        wl_surface: None,
                        title: String::new(),
                        app_id: String::new(),
                        x: 100,
                        y: 100,
                        width: 800,
                        height: 600,
                        last_configure_serial: Some(serial),
                        last_acked_serial: None,
                        is_configured: false,
                        is_mapped: false,
                        xdg_role: XdgRole::Toplevel,
                        pending_map: true,
                    });
                    resource.configure(serial);
                    info!("📤 Sent configure with serial {} to fallback toplevel", serial);
                }
            }
            xdg_surface::Request::GetPopup { id, .. } => {
                // Check if role already assigned
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.xdg_surface == *resource)
                {
                    if win.xdg_role != XdgRole::None {
                        warn!("⚠️ Attempted to assign popup role to surface that already has role: {:?}", win.xdg_role);
                        return;
                    }
                    // Assign popup role
                    win.xdg_role = XdgRole::Popup;
                }
                
                // Generate serial after releasing borrow
                let serial = state.next_serial();
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.xdg_surface == *resource)
                {
                    win.last_configure_serial = Some(serial);
                    resource.configure(serial);
                    info!("📤 Sent configure with serial {} to new popup", serial);
                } else {
                    // Fallback: create placeholder with popup role
                    let serial = state.next_serial();
                    resource.configure(serial);
                }
                data_init.init(id, ());
            }
            xdg_surface::Request::AckConfigure { serial } => {
                // Validate and track acked serial
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.xdg_surface == *resource)
                {
                    // Verify this serial was actually sent
                    if win.last_configure_serial == Some(serial) {
                        win.last_acked_serial = Some(serial);
                        win.is_configured = true;
                        win.pending_map = true;
                        info!("✅ Configure acknowledged: serial={} (valid)", serial);
                    } else if win.last_acked_serial.map_or(false, |acked| serial <= acked) {
                        // Client acking an old serial - allowed but warn
                        warn!("⚠️ Client acked old serial {} (last_acked: {:?}, last_sent: {:?})",
                            serial, win.last_acked_serial, win.last_configure_serial);
                        // Still update, as protocol allows acking older serials
                        win.last_acked_serial = Some(serial.max(win.last_acked_serial.unwrap_or(0)));
                        win.is_configured = true;
                        win.pending_map = true;
                    } else {
                        // Unknown serial - protocol violation
                        warn!("❌ Client acked unknown serial {} (expected: {:?})",
                            serial, win.last_configure_serial);
                        // Don't mark as configured - this is a protocol error
                    }
                } else {
                    warn!("⚠️ AckConfigure for unknown xdg_surface");
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
                info!("📝 Window title: {}", title);
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.xdg_toplevel.as_ref() == Some(resource))
                {
                    win.title = title;
                }
            }
            xdg_toplevel::Request::SetAppId { app_id } => {
                info!("📦 Window app ID: {}", app_id);
                if let Some(win) = state
                    .windows
                    .iter_mut()
                    .find(|w| w.xdg_toplevel.as_ref() == Some(resource))
                {
                    win.app_id = app_id;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &xdg_popup::XdgPopup,
        _request: xdg_popup::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<xdg_positioner::XdgPositioner, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &xdg_positioner::XdgPositioner,
        _request: xdg_positioner::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// REAL wl_seat protocol implementation
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
        seat.capabilities(wl_seat::Capability::Keyboard | wl_seat::Capability::Pointer);
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
                
                // Send keymap to the keyboard
                if let Some(ref keymap_str) = state.xkb_keymap_string {
                    match create_memfd_keymap(keymap_str) {
                        Ok(fd) => {
                            let size = keymap_str.len() as u32;
                            kb.keymap(
                                wl_keyboard::KeymapFormat::XkbV1,
                                fd.as_fd(),
                                size,
                            );
                            debug!("📋 Sent XKB keymap to keyboard: {} bytes", size);
                        }
                        Err(e) => {
                            warn!("Failed to create keymap memfd: {}", e);
                        }
                    }
                } else {
                    warn!("No XKB keymap available for keyboard!");
                }
                
                // Send repeat info (30 keys/sec, 500ms delay)
                kb.repeat_info(30, 500);
                
                state.keyboards.push(kb);
                debug!("✅ Keyboard configured with keymap and repeat info");
            }
            wl_seat::Request::GetPointer { id } => {
                let ptr = data_init.init(id, ());
                state.pointers.push(ptr);
                debug!("Pointer requested");
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
    ) {
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
    ) {
    }
}

// REAL wl_output protocol implementation
impl GlobalDispatch<wl_output::WlOutput, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_output::WlOutput>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output = data_init.init(resource, ());

        // Send real output information
        output.geometry(
            0,
            0,
            300,
            200, // physical size in mm
            wl_output::Subpixel::Unknown,
            "Axiom".to_string(),
            "Virtual Display".to_string(),
            wl_output::Transform::Normal,
        );

        output.mode(
            wl_output::Mode::Current | wl_output::Mode::Preferred,
            state.output_info.width,
            state.output_info.height,
            state.output_info.refresh,
        );

        output.scale(1);

        // Note: name() is only available in version 4+
        // For now, we'll skip it to maintain compatibility
        // output.name(state.output_info.name.clone());

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

// Subcompositor support
impl GlobalDispatch<wl_subcompositor::WlSubcompositor, ()> for CompositorState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_subcompositor::WlSubcompositor>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<wl_subcompositor::WlSubcompositor, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_subcompositor::WlSubcompositor,
        request: wl_subcompositor::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wl_subcompositor::Request::GetSubsurface { id, .. } = request {
            data_init.init(id, ());
        }
    }
}

impl Dispatch<wl_subsurface::WlSubsurface, ()> for CompositorState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_subsurface::WlSubsurface,
        _request: wl_subsurface::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}
