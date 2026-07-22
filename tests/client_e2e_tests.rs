//! Comprehensive end-to-end Wayland client integration tests.
//!
//! All tests run headlessly on the Noop backend (no GPU/winit/display needed).

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use axiom::{
    compositor::AxiomCompositor,
    config::AxiomConfig,
    input::InputManager,
    ipc::{AxiomIPCServer, LazyUIMessage},
    window::WindowManager,
    workspace::ScrollableWorkspaces,
};
use parking_lot::RwLock;

use wayland_client::{
    delegate_noop,
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_registry, wl_seat, wl_shm, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const OFFERED: &str = "hello from client";

fn make_headless_compositor(
    config: AxiomConfig,
) -> Result<(AxiomCompositor, Arc<RwLock<WindowManager>>)> {
    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let ipc_server = AxiomIPCServer::new();

    let mut config = config;
    config.backend.kind = "noop".to_string();

    let compositor = AxiomCompositor::new(
        config,
        false,
        workspace_manager.clone(),
        window_manager.clone(),
        input_manager.clone(),
        ipc_server,
    )?;

    Ok((compositor, window_manager))
}

// ── Test 1: Basic toplevel creation + configure event ────────────────────

struct ToplevelState {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    configured: bool,
    configured_serial: u32,
}

impl ToplevelState {
    fn init_xdg_surface(&mut self, qh: &QueueHandle<Self>) {
        let (wm_base, compositor) = match (self.wm_base.as_ref(), self.compositor.as_ref()) {
            (Some(wm_base), Some(compositor)) => (wm_base, compositor),
            _ => return,
        };
        if self.surface.is_some() {
            return;
        }
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.surface = Some(surface);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for ToplevelState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(ToplevelState: ignore wl_compositor::WlCompositor);
delegate_noop!(ToplevelState: ignore wl_surface::WlSurface);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ToplevelState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for ToplevelState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
            state.configured_serial = serial;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for ToplevelState {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn run_toplevel_client(result_tx: mpsc::Sender<String>) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<ToplevelState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = ToplevelState {
            compositor: None,
            wm_base: None,
            surface: None,
            configured: false,
            configured_serial: 0,
        };

        for _ in 0..64 {
            event_queue.blocking_dispatch(&mut state)?;
            if state.configured {
                break;
            }
        }
        if !state.configured {
            anyhow::bail!("timed out waiting for configure event");
        }
        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
}

#[test]
#[serial_test::serial]
fn test_client_creates_toplevel_and_configures() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, window_manager) = make_headless_compositor(config)?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let (tx, rx) = mpsc::channel();
    let client_handle = thread::spawn(move || run_toplevel_client(tx));

    let mut ticks = 0;
    while ticks < 200 {
        compositor.tick_for_test()?;
        ticks += 1;
        if window_manager.read().window_count() >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();

    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");
    assert!(
        window_manager.read().window_count() >= 1,
        "compositor did not track the client toplevel"
    );

    Ok(())
}

// ── Test 2: Clipboard offer via wayland-client ───────────────────────────

struct ClipboardState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    data_device: Option<wl_data_device::WlDataDevice>,
    data_source: Option<wl_data_source::WlDataSource>,
    surface: Option<wl_surface::WlSurface>,
    offered: bool,
    payload_written: bool,
}

impl ClipboardState {
    fn ensure_data_device(&mut self, qh: &QueueHandle<Self>) {
        if self.data_device.is_some() {
            return;
        }
        if let (Some(mgr), Some(seat)) = (self.data_device_manager.as_ref(), self.seat.as_ref()) {
            self.data_device = Some(mgr.get_data_device(seat, qh, ()));
        }
    }

    fn init_xdg_surface(&mut self, qh: &QueueHandle<Self>) {
        let (wm_base, compositor) = match (self.wm_base.as_ref(), self.compositor.as_ref()) {
            (Some(wm_base), Some(compositor)) => (wm_base, compositor),
            _ => return,
        };
        if self.surface.is_some() {
            return;
        }
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.surface = Some(surface);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "wl_seat" => {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, 7, qh, ()));
                    state.ensure_data_device(qh);
                }
                "wl_data_device_manager" => {
                    state.data_device_manager = Some(
                        registry.bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(
                            name, 3, qh, (),
                        ),
                    );
                    state.ensure_data_device(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(ClipboardState: ignore wl_compositor::WlCompositor);
delegate_noop!(ClipboardState: ignore wl_surface::WlSurface);
delegate_noop!(ClipboardState: ignore wl_shm::WlShm);
delegate_noop!(ClipboardState: ignore wl_seat::WlSeat);
delegate_noop!(ClipboardState: ignore wl_data_device_manager::WlDataDeviceManager);

impl Dispatch<wl_data_offer::WlDataOffer, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        _: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        _: &wl_data_device::WlDataDevice,
        _: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(ClipboardState, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ()),
        5 => (wl_data_source::WlDataSource, ()),
    ]);
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for ClipboardState {
    fn event(
        _state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_source::Event::Send { fd, .. } = event {
            let mut file = std::fs::File::from(fd);
            let _ = file.write_all(OFFERED.as_bytes());
            let _ = file.flush();
            state.payload_written = true;
        }
    }
}

#[derive(Clone)]
struct ClipboardFlags {
    focus_granted: Arc<AtomicBool>,
}

fn run_clipboard_client(flags: ClipboardFlags, result_tx: mpsc::Sender<String>) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<ClipboardState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = ClipboardState {
            compositor: None,
            shm: None,
            wm_base: None,
            seat: None,
            data_device_manager: None,
            data_device: None,
            data_source: None,
            surface: None,
            offered: false,
            payload_written: false,
        };

        for _ in 0..1024 {
            let _ = event_queue.flush();

            if let Some(guard) = event_queue.prepare_read() {
                let fd = guard.connection_fd().as_raw_fd();
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe {
                    libc::poll(&mut pfd as *mut libc::pollfd, 1, 5);
                }
                if pfd.revents & libc::POLLIN != 0 {
                    let _ = guard.read();
                }
            }
            event_queue.dispatch_pending(&mut state)?;

            if flags.focus_granted.load(Ordering::SeqCst) && !state.offered {
                if let (Some(mgr), Some(dd)) = (
                    state.data_device_manager.as_ref(),
                    state.data_device.as_ref(),
                ) {
                    let ds = mgr.create_data_source(&qh, ());
                    ds.offer("text/plain".to_string());
                    state.data_source = Some(ds);
                    dd.set_selection(state.data_source.as_ref(), 0);
                    state.offered = true;
                }
            }

            if state.payload_written && state.offered {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
}

#[test]
#[serial_test::serial]
fn test_client_clipboard_offer_via_wayland() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, window_manager) = make_headless_compositor(config)?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let flags = ClipboardFlags {
        focus_granted: Arc::new(AtomicBool::new(false)),
    };

    let (tx, rx) = mpsc::channel();
    let client_flags = flags.clone();
    let client_handle = thread::spawn(move || run_clipboard_client(client_flags, tx));

    let mut ticks = 0;
    while window_manager.read().window_count() < 1 && ticks < 200 {
        compositor.tick_for_test()?;
        ticks += 1;
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        window_manager.read().window_count() >= 1,
        "compositor did not track the client toplevel"
    );

    compositor.debug_focus_first_client_for_test();
    flags.focus_granted.store(true, Ordering::SeqCst);

    let mut cached: Option<Vec<u8>> = None;
    for _ in 0..200 {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(5));
        cached = compositor.debug_clipboard_cache();
        if cached.as_deref() == Some(OFFERED.as_bytes()) {
            break;
        }
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    assert_eq!(
        cached.as_deref(),
        Some(OFFERED.as_bytes()),
        "compositor clipboard cache did not receive the client's offered selection"
    );

    Ok(())
}

// ── Test 3: Fullscreen toggle via client set_fullscreen ──────────────────

struct FullscreenState {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    configured: bool,
}

impl FullscreenState {
    fn init_xdg_surface(&mut self, qh: &QueueHandle<Self>) {
        let (wm_base, compositor) = match (self.wm_base.as_ref(), self.compositor.as_ref()) {
            (Some(wm_base), Some(compositor)) => (wm_base, compositor),
            _ => return,
        };
        if self.surface.is_some() {
            return;
        }
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.surface = Some(surface);
        self.toplevel = Some(toplevel);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for FullscreenState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(FullscreenState: ignore wl_compositor::WlCompositor);
delegate_noop!(FullscreenState: ignore wl_surface::WlSurface);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for FullscreenState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for FullscreenState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for FullscreenState {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn run_fullscreen_client(
    fullscreen_sent: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    result_tx: mpsc::Sender<String>,
) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<FullscreenState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = FullscreenState {
            compositor: None,
            wm_base: None,
            surface: None,
            toplevel: None,
            configured: false,
        };

        while !done.load(Ordering::SeqCst) {
            event_queue.blocking_dispatch(&mut state)?;
            if state.configured {
                if let Some(toplevel) = state.toplevel.as_ref() {
                    toplevel.set_fullscreen(None);
                }
                let _ = event_queue.flush();
                fullscreen_sent.store(true, Ordering::SeqCst);
                break;
            }
        }

        while !done.load(Ordering::SeqCst) {
            let _ = event_queue.dispatch_pending(&mut state);
            thread::sleep(Duration::from_millis(2));
        }

        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
}

#[test]
#[serial_test::serial]
fn test_fullscreen_toggle() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, window_manager) = make_headless_compositor(config)?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let fullscreen_sent = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let (tx, rx) = mpsc::channel();
    let c_fs = Arc::clone(&fullscreen_sent);
    let c_done = Arc::clone(&done);
    let client_handle = thread::spawn(move || run_fullscreen_client(c_fs, c_done, tx));

    let mut ticks = 0;
    while window_manager.read().window_count() < 1 && ticks < 200 {
        compositor.tick_for_test()?;
        ticks += 1;
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        window_manager.read().window_count() >= 1,
        "compositor did not track the client toplevel"
    );

    while !fullscreen_sent.load(Ordering::SeqCst) {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(5));
    }

    for _ in 0..10 {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(5));
    }

    let window_id = window_manager
        .read()
        .focused_window_id()
        .expect("no focused window after client mapped");
    let is_fullscreen = window_manager
        .read()
        .get_window(window_id)
        .map(|w| w.properties.fullscreen)
        .unwrap_or(false);
    assert!(
        is_fullscreen,
        "compositor did not mark window as fullscreen after client set_fullscreen"
    );

    done.store(true, Ordering::SeqCst);
    for _ in 0..10 {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(1));
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    Ok(())
}

// ── Test 4: Minimize / restore via IPC ───────────────────────────────────

struct PassthroughState {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    configured: bool,
}

impl PassthroughState {
    fn init_xdg_surface(&mut self, qh: &QueueHandle<Self>) {
        let (wm_base, compositor) = match (self.wm_base.as_ref(), self.compositor.as_ref()) {
            (Some(wm_base), Some(compositor)) => (wm_base, compositor),
            _ => return,
        };
        if self.surface.is_some() {
            return;
        }
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.surface = Some(surface);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for PassthroughState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()),
                    );
                    state.init_xdg_surface(qh);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(PassthroughState: ignore wl_compositor::WlCompositor);
delegate_noop!(PassthroughState: ignore wl_surface::WlSurface);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for PassthroughState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for PassthroughState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for PassthroughState {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn run_passthrough_client(done: Arc<AtomicBool>, result_tx: mpsc::Sender<String>) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<PassthroughState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = PassthroughState {
            compositor: None,
            wm_base: None,
            surface: None,
            configured: false,
        };

        while !done.load(Ordering::SeqCst) {
            event_queue.blocking_dispatch(&mut state)?;
            if state.configured {
                break;
            }
        }

        while !done.load(Ordering::SeqCst) {
            let _ = event_queue.dispatch_pending(&mut state);
            thread::sleep(Duration::from_millis(2));
        }

        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
}

#[test]
#[serial_test::serial]
fn test_client_minimize_restore() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, window_manager) = make_headless_compositor(config)?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let done = Arc::new(AtomicBool::new(false));

    let (tx, rx) = mpsc::channel();
    let c_done = Arc::clone(&done);
    let client_handle = thread::spawn(move || run_passthrough_client(c_done, tx));

    let mut ticks = 0;
    while window_manager.read().window_count() < 1 && ticks < 200 {
        compositor.tick_for_test()?;
        ticks += 1;
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        window_manager.read().window_count() >= 1,
        "compositor did not track the client toplevel"
    );

    let window_id = window_manager
        .read()
        .focused_window_id()
        .expect("no focused window");

    let sender = compositor.ipc_command_sender();
    sender
        .send(LazyUIMessage::WorkspaceCommand {
            action: "minimize_window".into(),
            parameters: serde_json::json!({"window_id": window_id}),
        })
        .unwrap();

    for _ in 0..20 {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(5));
    }

    assert!(
        window_manager.read().is_minimized(window_id),
        "window should be minimized after IPC minimize command"
    );

    sender
        .send(LazyUIMessage::WorkspaceCommand {
            action: "restore_window".into(),
            parameters: serde_json::json!({"window_id": window_id}),
        })
        .unwrap();

    for _ in 0..20 {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(5));
    }

    assert!(
        !window_manager.read().is_minimized(window_id),
        "window should be restored after IPC restore command"
    );

    done.store(true, Ordering::SeqCst);
    for _ in 0..10 {
        compositor.tick_for_test()?;
        thread::sleep(Duration::from_millis(1));
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    Ok(())
}
