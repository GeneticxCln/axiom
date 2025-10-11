//! Minimal Wayland server using wayland-server 0.31 and calloop
//! This is a thin, compiling server that accepts clients and advertises
//! wl_compositor, wl_shm, wl_output, and xdg_wm_base. No rendering.

use anyhow::{Context, Result};
use log::{info, debug};

use wayland_protocols::xdg::shell::server::{xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols::wp::presentation_time::server::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::wp::viewporter::server::{wp_viewporter, wp_viewport};
use wayland_server::{
    backend::ClientData,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New,
    Resource, WEnum,
};
use memmap2::{Mmap, MmapOptions};
use std::fs::File;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// Global compositor state for this minimal server
pub struct MinimalState {
    pub seat_name: String,
    pub windows: Vec<WindowEntry>,
    pub serial_counter: u32,
    pub xdg_bases: Vec<xdg_wm_base::XdgWmBase>,
    pub keyboards: Vec<wl_keyboard::WlKeyboard>,
    pub pointers: Vec<wl_pointer::WlPointer>,
    pub pending_callbacks: Vec<wl_callback::WlCallback>,
    pub last_frame_time: Instant,
    pub last_ping_time: Instant,
    // Internal event bus queue (drained in run loop)
    pub events: Vec<ServerEvent>,
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
    pub viewport_map: HashMap<u32, ViewportState>,
}

#[derive(Clone)]
pub struct WindowEntry {
    pub xdg_surface: xdg_surface::XdgSurface,
    pub xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub wl_surface: Option<wl_surface::WlSurface>,
    pub configured_serial: Option<u32>,
    pub mapped: bool,
    pub title: String,
    pub app_id: String,
    pub axiom_id: Option<u64>,
    // Pending attached wl_buffer id and offset
    pub pending_buffer_id: Option<u32>,
    pub attach_offset: (i32, i32),
}

/// A minimal Wayland server that runs a Display and accepts clients
use parking_lot::RwLock;

pub struct MinimalServer {
    pub display: Display<MinimalState>,
    pub listening: ListeningSocket,
    pub socket_name: String,
    // Axiom managers for integration
    pub window_manager: Arc<RwLock<crate::window::WindowManager>>, 
    pub workspace_manager: Arc<RwLock<crate::workspace::ScrollableWorkspaces>>, 
    pub input_manager: Arc<RwLock<crate::input::InputManager>>, 
    // Input channel from evdev thread
    input_rx: Option<Receiver<HwInputEvent>>,
}

// Internal event bus messages produced by Wayland dispatch and handled in the run loop
#[derive(Debug, Clone)]
enum ServerEvent {
    Commit { surface: wl_surface::WlSurface },
    Destroy { surface: wl_surface::WlSurface },
    TitleChanged { surface: wl_surface::WlSurface, title: String },
    AppIdChanged { surface: wl_surface::WlSurface, app_id: String },
}

// Hardware input events captured by evdev thread
#[derive(Debug, Clone)]
enum HwInputEvent {
    Key { key: String, modifiers: Vec<String>, pressed: bool },
    PointerMotion { dx: f64, dy: f64 },
    PointerButton { button: u8, pressed: bool },
}

impl MinimalServer {
    pub fn new(
        window_manager: Arc<RwLock<crate::window::WindowManager>>, 
        workspace_manager: Arc<RwLock<crate::workspace::ScrollableWorkspaces>>, 
        input_manager: Arc<RwLock<crate::input::InputManager>>, 
    ) -> Result<Self> {
        let display: Display<MinimalState> = Display::new().context("create display")?;
        let dh = display.handle();

        // Create core globals
        dh.create_global::<MinimalState, wl_compositor::WlCompositor, _>(4, ());
        dh.create_global::<MinimalState, wl_shm::WlShm, _>(1, ());
        dh.create_global::<MinimalState, wl_output::WlOutput, _>(3, ());
        dh.create_global::<MinimalState, wl_seat::WlSeat, _>(7, ());
        dh.create_global::<MinimalState, xdg_wm_base::XdgWmBase, _>(3, ());
        dh.create_global::<MinimalState, wp_presentation::WpPresentation, _>(1, ());
        dh.create_global::<MinimalState, wp_viewporter::WpViewporter, _>(1, ());
        debug!("Globals: wl_compositor v4, wl_shm v1, wl_output v3, wl_seat v7, xdg_wm_base v3");

        // Bind an auto socket for Wayland
        let listening = ListeningSocket::bind_auto("wayland", 1..32).context("bind socket")?;
        let socket_name = listening
            .socket_name()
            .and_then(|s| Some(s.to_string_lossy().to_string()))
            .ok_or_else(|| anyhow::anyhow!("missing socket name"))?;

        // Spawn evdev input thread (best-effort; may fail without permissions)
        let input_rx = Self::spawn_evdev_input_thread();

        Ok(Self {
            display,
            listening,
            socket_name,
            window_manager,
            workspace_manager,
            input_manager,
            input_rx,
        })
    }

    pub fn run(mut self) -> Result<()> {
        std::env::set_var("WAYLAND_DISPLAY", &self.socket_name);
        info!("WAYLAND_DISPLAY={}", self.socket_name);

        // Start headless GPU render loop in a background thread
        std::thread::spawn(|| {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                let _ = rt.block_on(async {
                    let _ = crate::renderer::AxiomRenderer::start_headless_loop().await;
                });
            }
        });

        // Initialize workspace viewport to match our single wl_output mode
        {
            let mut ws = self.workspace_manager.write();
            ws.set_viewport_size(1920.0, 1080.0);
        }

        let mut state = MinimalState {
            seat_name: "seat0".into(),
            windows: Vec::new(),
            serial_counter: 1,
            xdg_bases: Vec::new(),
            keyboards: Vec::new(),
            pointers: Vec::new(),
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
        };

        loop {
            if let Ok(Some(stream)) = self.listening.accept() {
                let _ = self
                    .display
                    .handle()
                    .insert_client(stream, Arc::new(ServerClientData));
                debug!("Client connected");
            }
            // Drain input from evdev and handle
            self.handle_hw_input(&mut state)?;

            self.display.dispatch_clients(&mut state)?;

            // Drain and handle internal events produced during dispatch
            self.handle_events(&mut state)?;

            // Simple frame tick (~16ms)
            if state.last_frame_time.elapsed() >= std::time::Duration::from_millis(16) {
                let ts_ms: u32 = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() & 0xFFFF_FFFF) as u32;
                for cb in state.pending_callbacks.drain(..) {
                    cb.done(ts_ms);
                }
                state.last_frame_time = Instant::now();
            }

            // Periodic ping (~5s)
            if state.last_ping_time.elapsed() >= std::time::Duration::from_secs(5) {
                let serial = state.serial_counter; // use next_serial but without borrow; update below
                for base in &state.xdg_bases {
                    base.ping(serial);
                }
                state.serial_counter = state.serial_counter.wrapping_add(1);
                state.last_ping_time = Instant::now();
            }

            self.display.flush_clients()?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

struct ServerClientData;
impl ClientData for ServerClientData {}

impl MinimalServer {
    fn handle_hw_input(&mut self, state: &mut MinimalState) -> Result<()> {
        use crate::input::{CompositorAction, InputEvent as AxiomInputEvent};
        // Drain the channel if present
        // Drain events to a buffer to avoid borrowing self immutably while mutating
        let mut buf: Vec<HwInputEvent> = Vec::new();
        if let Some(rx) = &self.input_rx {
            loop {
                match rx.try_recv() {
                    Ok(ev) => buf.push(ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => { self.input_rx = None; break; }
                }
            }
        }

        for ev in buf {
            match ev {
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
                                CompositorAction::Quit => {
                                    // Graceful shutdown: currently ignored in minimal server loop
                                }
                                _ => {}
                            }
                        }
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
            }
        }
        Ok(())
    }

    fn handle_events(&mut self, state: &mut MinimalState) -> Result<()> {
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
                                        crate::renderer::queue_texture_update(ax_id, data, w, h);
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
            }
        }

        Ok(())
    }

    fn apply_layouts(&mut self, state: &mut MinimalState) -> Result<()> {
        // Compute layouts from workspace manager and push size configures to clients
        let layouts: HashMap<u64, crate::window::Rectangle> = {
            let ws = self.workspace_manager.read();
            ws.calculate_workspace_layouts()
        };

        // Cache layouts for pointer hit-testing
        state.last_layouts.clear();
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
                    tl.configure(rect.width as i32, rect.height as i32, vec![]);
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

impl MinimalServer {
    fn update_pointer_focus_and_motion(&mut self, state: &mut MinimalState) -> Result<()> {
        // Determine which window is under the pointer
        let (px, py) = state.pointer_pos;
        let mut target: Option<(u64, (f64, f64))> = None;
        for (id, rect) in &state.last_layouts {
            let r = rect;
            let inside = px >= r.x as f64
                && px < (r.x as f64 + r.width as f64)
                && py >= r.y as f64
                && py < (r.y as f64 + r.height as f64);
            if inside {
                let local_x = px - r.x as f64;
                let local_y = py - r.y as f64;
                target = Some((*id, (local_x, local_y)));
                break;
            }
        }

        if let Some((id, (lx, ly))) = target {
            if state.pointer_focus_window != Some(id) {
                // Leave previous
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

    fn handle_pointer_button(&mut self, state: &mut MinimalState, button: u8, pressed: bool) -> Result<()> {
        // Send to focused pointer surface if any
        if let Some(focus_id) = state.pointer_focus_window {
            if let Some(surface) = state
                .windows
                .iter()
                .find(|w| w.axiom_id == Some(focus_id))
                .and_then(|w| w.wl_surface.clone())
            {
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

    fn spawn_evdev_input_thread() -> Option<Receiver<HwInputEvent>> {
        use evdev::{Device, EventType, Key, RelativeAxisType};
        use std::fs;
        let (tx, rx) = mpsc::channel::<HwInputEvent>();
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
                    let has_btn = d.supported_keys().map_or(false, |k| k.contains(Key::BTN_LEFT) || k.contains(Key::BTN_RIGHT) || k.contains(Key::BTN_MIDDLE));
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
                        for ev in events {
                            match ev.event_type() {
                                EventType::RELATIVE => {
                                    if ev.code() == RelativeAxisType::REL_X.0 { dx += ev.value() as f64; }
                                    if ev.code() == RelativeAxisType::REL_Y.0 { dy += ev.value() as f64; }
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
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });

        Some(rx)
    }
}


// wl_compositor global
impl GlobalDispatch<wl_compositor::WlCompositor, ()> for MinimalState {
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
impl Dispatch<wl_compositor::WlCompositor, ()> for MinimalState {
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
impl GlobalDispatch<wl_shm::WlShm, ()> for MinimalState {
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
impl Dispatch<wl_shm::WlShm, ()> for MinimalState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wl_shm::WlShm,
        request: wl_shm::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_shm::Request::CreatePool { id, fd, size } => {
                // Map the file descriptor
                let file: File = fd.into();
                match unsafe { Mmap::map(&file) } {
                    Ok(map) => {
                        let pool_data = ShmPoolData { map: Arc::new(map), size };
                        data_init.init(id, pool_data);
                    }
                    Err(_e) => {
                        // Failed to map; still init to avoid protocol errors with a tiny anon map
                        let anon = unsafe { MmapOptions::new().len(1).map_anon().unwrap() };
                        let ro = anon.make_read_only().unwrap();
                        let pool_data = ShmPoolData { map: Arc::new(ro), size: 0 };
                        data_init.init(id, pool_data);
                    }
                }
                // File drops here; mapping remains valid
            }
            _ => {}
        }
    }
}

// wl_seat global
impl GlobalDispatch<wl_seat::WlSeat, ()> for MinimalState {
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
impl Dispatch<wl_seat::WlSeat, ()> for MinimalState {
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
                state.keyboards.push(kb);
            }
            wl_seat::Request::GetPointer { id } => {
                let pt = data_init.init(id, ());
                state.pointers.push(pt);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for MinimalState {
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

impl Dispatch<wl_pointer::WlPointer, ()> for MinimalState {
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

impl Dispatch<wl_callback::WlCallback, ()> for MinimalState {
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

// Shm pool and buffer handling
#[derive(Clone)]
struct ShmPoolData {
    map: Arc<Mmap>,
    size: i32,
}

#[derive(Clone)]
struct BufferRecord {
    id: u32,
    buffer: wl_buffer::WlBuffer,
    width: i32,
    height: i32,
    stride: i32,
    offset: i32,
    format: WEnum<wl_shm::Format>,
    // Hold ref to pool map
    map: Arc<Mmap>,
}

impl MinimalState {
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

fn convert_shm_to_rgba(rec: &BufferRecord) -> Option<Vec<u8>> {
    let width = rec.width.max(0) as usize;
    let height = rec.height.max(0) as usize;
    let stride = rec.stride.max(0) as usize;
    let offset = rec.offset.max(0) as usize;
    if width == 0 || height == 0 { return None; }
    let needed = offset.checked_add(stride.checked_mul(height)? )?;
    if needed > rec.map.len() { return None; }
    let src = &rec.map[offset..offset + stride * height];
    let mut out = vec![0u8; width * height * 4];
    // wl_shm formats are little-endian
    match rec.format {
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

fn process_with_viewport(rec: &BufferRecord, vp: Option<&ViewportState>) -> Option<(Vec<u8>, u32, u32)> {
    // Convert the full buffer first
    let rgba = convert_shm_to_rgba(rec)?;
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

impl Dispatch<wl_shm_pool::WlShmPool, ShmPoolData> for MinimalState {
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
                    width, height, stride, offset, format,
                    map: data.map.clone(),
                };
                state_buffers(state).insert(rec.id, rec);
            }
            wl_shm_pool::Request::Resize { size } => {
                // Not supported in this minimal path
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

impl Dispatch<wl_buffer::WlBuffer, ()> for MinimalState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &wl_buffer::WlBuffer,
        request: wl_buffer::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_buffer::Request::Destroy => {
                if let Some(cell) = BUFFERS_MAP.get() {
                    let _ = cell.fetch(state).remove(&resource.id().protocol_id());
                }
            }
            _ => {}
        }
    }
}

fn state_buffers(state: &mut MinimalState) -> &mut HashMap<u32, BufferRecord> {
    // Side storage keyed by MinimalState pointer value
    BuffersStorageCell::get_or_init(BuffersStorage::default());
    BUFFERS_MAP.get().unwrap().fetch(state)
}

// Poor-man side storage associated with MinimalState pointer address
struct BuffersStorage {
    map: HashMap<usize, HashMap<u32, BufferRecord>>,
}
impl Default for BuffersStorage { fn default() -> Self { Self { map: HashMap::new() } } }
static BUFFERS_MAP: OnceLock<BuffersStorageCell> = OnceLock::new();
struct BuffersStorageCell(std::sync::Mutex<BuffersStorage>);
impl BuffersStorageCell {
    fn get_or_init(default: BuffersStorage) {
        let _ = BUFFERS_MAP.get_or_init(|| BuffersStorageCell(std::sync::Mutex::new(default)));
    }
    fn fetch<'a>(&'a self, state: &'a mut MinimalState) -> &'a mut HashMap<u32, BufferRecord> {
        let key = state as *mut _ as usize;
        let mut guard = self.0.lock().unwrap();
        guard.map.entry(key).or_insert_with(HashMap::new);
        // SAFETY: We keep the storage for the lifetime of the process
        let ptr: *mut HashMap<u32, BufferRecord> = guard.map.get_mut(&key).unwrap();
        drop(guard);
        unsafe { &mut *ptr }
    }
}

// wp_presentation_time global
impl GlobalDispatch<wp_presentation::WpPresentation, ()> for MinimalState {
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
impl Dispatch<wp_presentation::WpPresentation, ()> for MinimalState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wp_presentation::WpPresentation,
        request: wp_presentation::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_presentation::Request::Feedback { surface, callback } => {
                let fb: wp_presentation_feedback::WpPresentationFeedback = data_init.init(callback, ());
                let sid = surface.id().protocol_id();
                state.presentation_feedbacks.entry(sid).or_default().push(fb);
            }
            _ => {}
        }
    }
}

// wp_viewporter global
impl GlobalDispatch<wp_viewporter::WpViewporter, ()> for MinimalState {
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
impl Dispatch<wp_viewporter::WpViewporter, ()> for MinimalState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wp_viewporter::WpViewporter,
        request: wp_viewporter::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_viewporter::Request::GetViewport { id, surface } => {
                let surface_id = surface.id().protocol_id();
                let _vp = data_init.init(id, ViewportData { surface_id });
                // Initialize default viewport state entry
                state.viewport_map.entry(surface_id).or_insert_with(ViewportState::default);
            }
            _ => {}
        }
    }
}
impl Dispatch<wp_viewport::WpViewport, ViewportData> for MinimalState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wp_viewport::WpViewport,
        request: wp_viewport::Request,
        data: &ViewportData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let entry = state.viewport_map.entry(data.surface_id).or_insert_with(ViewportState::default);
        match request {
            wp_viewport::Request::SetSource { x: _x, y: _y, width: _width, height: _height } => {
                // Minimal stub: ignore cropping for now
                entry.source = None;
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

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for MinimalState {
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

// wl_output global
impl GlobalDispatch<wl_output::WlOutput, ()> for MinimalState {
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
            "Minimal".to_string(),
            wl_output::Transform::Normal,
        );
        output.mode(wl_output::Mode::Current | wl_output::Mode::Preferred, 1920, 1080, 60000);
        output.scale(1);
        output.done();
    }
}
impl Dispatch<wl_output::WlOutput, ()> for MinimalState {
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

// Helpers
impl MinimalState {
    fn next_serial(&mut self) -> u32 {
        let s = self.serial_counter;
        self.serial_counter = self.serial_counter.wrapping_add(1);
        s
    }
}

// xdg_wm_base global
impl GlobalDispatch<xdg_wm_base::XdgWmBase, ()> for MinimalState {
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
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for MinimalState {
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
                    wl_surface: Some(surface),
                    configured_serial: None,
                    mapped: false,
                    title: String::new(),
                    app_id: String::new(),
                    axiom_id: None,
                    pending_buffer_id: None,
                    attach_offset: (0, 0),
                });
            }
            xdg_wm_base::Request::CreatePositioner { id } => { let _ = data_init.init(id, ()); }
            xdg_wm_base::Request::Pong { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<xdg_positioner::XdgPositioner, ()> for MinimalState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &xdg_positioner::XdgPositioner,
        _request: xdg_positioner::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) { }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for MinimalState {
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

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for MinimalState {
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
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for MinimalState {
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
                }
            }
            wl_surface::Request::Commit => {
                // Defer manager mutations and input focus to run loop via event bus
                state.events.push(ServerEvent::Commit { surface: resource.clone() });
                // For presentation-time feedback consumers, we will discard for now
                let sid = resource.id().protocol_id();
                if let Some(list) = state.presentation_feedbacks.remove(&sid) {
                    for fb in list {
                        fb.discarded();
                    }
                }
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
