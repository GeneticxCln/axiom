//! End-to-end test of DnD/selection data plumbing through the compositor.
//!
//! A real `wayland-client` connects, creates a surface, and offers data
//! through the data_device selection path. Confirms the compositor
//! receives and caches the offered payload.
//!
//! Also tests ServerDndGrabHandler::send directly.
//!
//! The compositor runs on the Noop backend (headless, no display needed).

use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use axiom::{
    backend::AxiomSmithayBackendReal, config::AxiomConfig, decoration::DecorationManager,
    input::InputManager, window::WindowManager, workspace::ScrollableWorkspaces,
};
use parking_lot::RwLock;
use smithay::wayland::selection::data_device::ServerDndGrabHandler;

use wayland_client::{
    delegate_noop,
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_registry, wl_seat, wl_shm, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const OFFERED: &str = "dnd-data-payload";

// ── Client State ───────────────────────────────────────────────────────

struct ClientState {
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

impl ClientState {
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
        let _toplevel = xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.surface = Some(surface);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for ClientState {
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
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()));
                    state.init_xdg_surface(qh);
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, 7, qh, ()));
                    state.ensure_data_device(qh);
                }
                "wl_data_device_manager" => {
                    state.data_device_manager = Some(
                        registry.bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(
                            name,
                            3,
                            qh,
                            (),
                        ),
                    );
                    state.ensure_data_device(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.init_xdg_surface(qh);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(ClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(ClientState: ignore wl_surface::WlSurface);
delegate_noop!(ClientState: ignore wl_shm::WlShm);
delegate_noop!(ClientState: ignore wl_seat::WlSeat);
delegate_noop!(ClientState: ignore wl_data_device_manager::WlDataDeviceManager);

impl Dispatch<wl_data_offer::WlDataOffer, ()> for ClientState {
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

impl Dispatch<wl_data_device::WlDataDevice, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &wl_data_device::WlDataDevice,
        _: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(ClientState, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ()),
        5 => (wl_data_source::WlDataSource, ()),
    ]);
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for ClientState {
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

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for ClientState {
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

impl Dispatch<wl_data_source::WlDataSource, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_source::Event::Send { fd, .. } = event {
            use std::io::Write;
            let mut file = std::fs::File::from(fd);
            let _ = file.write_all(OFFERED.as_bytes());
            let _ = file.flush();
            state.payload_written = true;
        }
    }
}

#[derive(Clone)]
struct Flags {
    focus_granted: Arc<AtomicBool>,
}

/// Drive a real Wayland client that creates a surface and offers data.
fn run_client(flags: Flags, result_tx: mpsc::Sender<String>) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<ClientState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = ClientState {
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

        for _ in 0..256 {
            let _ = event_queue.flush();
            if let Some(guard) = event_queue.prepare_read() {
                use std::os::unix::io::AsRawFd;
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

            // Once focus granted, offer data via set_selection (exercises the
            // data_device path shared by DnD and clipboard).
            if flags.focus_granted.load(Ordering::SeqCst) && !state.offered {
                if let (Some(mgr), Some(dd)) = (
                    state.data_device_manager.as_ref(),
                    state.data_device.as_ref(),
                ) {
                    let ds = mgr.create_data_source(&qh, ());
                    ds.offer("text/plain".to_string());
                    dd.set_selection(Some(&ds), 0);
                    state.data_source = Some(ds);
                    state.offered = true;
                }
            }

            // Wait until data has been written (pulled by compositor's pipe worker)
            if state.payload_written {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        if !state.payload_written {
            anyhow::bail!("client data payload never written");
        }
        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
}

// ── Helpers ────────────────────────────────────────────────────────────

fn make_headless_backend() -> Result<AxiomSmithayBackendReal> {
    let config = AxiomConfig::default();
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let decoration_manager = Arc::new(RwLock::new(DecorationManager::new(
        &config.window,
        config.features.enable_minimize,
    )));
    AxiomSmithayBackendReal::new_for_test(
        config,
        window_manager,
        workspace_manager,
        input_manager,
        decoration_manager,
    )
}

// ── Tests ──────────────────────────────────────────────────────────────

/// ServerDndGrabHandler serves clipboard cache data when populated.
#[test]
#[serial_test::serial]
fn test_server_dnd_handler_serves_data() -> Result<()> {
    let mut backend = make_headless_backend()?;
    backend.state.clipboard_cache = Some(b"server-dnd-payload".to_vec());

    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    assert_eq!(rc, 0, "pipe2");
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    let seat = backend.state.seat.clone();
    ServerDndGrabHandler::send(&mut backend.state, "text/plain".into(), write_fd, seat);

    let mut buf = Vec::new();
    let mut file = std::fs::File::from(read_fd);
    file.read_to_end(&mut buf)?;
    assert_eq!(buf, b"server-dnd-payload", "serves cached data");
    Ok(())
}

/// ServerDndGrabHandler drops fd when cache is empty (no panic).
#[test]
#[serial_test::serial]
fn test_server_dnd_handler_empty_cache() -> Result<()> {
    let mut backend = make_headless_backend()?;
    backend.state.clipboard_cache = None;

    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    assert_eq!(rc, 0, "pipe2");
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    let seat = backend.state.seat.clone();
    ServerDndGrabHandler::send(&mut backend.state, "text/plain".into(), write_fd, seat);
    // no panic = pass
    Ok(())
}

/// End-to-end: real Wayland client offers data that reaches compositor cache.
#[test]
#[serial_test::serial]
fn test_client_data_offer_reaches_compositor() -> Result<()> {
    let config = AxiomConfig::default();
    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let ipc_server = axiom::ipc::AxiomIPCServer::new();

    let mut cfg = config.clone();
    cfg.backend.kind = "noop".to_string();

    let mut compositor = axiom::compositor::AxiomCompositor::new(
        cfg,
        false,
        workspace_manager.clone(),
        window_manager.clone(),
        input_manager.clone(),
        ipc_server,
    )?;

    let socket_name = compositor.socket_name().to_string();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let flags = Flags {
        focus_granted: Arc::new(AtomicBool::new(false)),
    };
    let (tx, rx) = mpsc::channel();
    let client_flags = flags.clone();
    let client_handle = thread::spawn(move || run_client(client_flags, tx));

    // Tick until client surface is tracked
    let mut ticks = 0;
    while window_manager.read().window_count() < 1 && ticks < 200 {
        compositor.tick_for_test()?;
        ticks += 1;
        thread::sleep(Duration::from_millis(5));
    }
    assert!(window_manager.read().window_count() >= 1);

    // Grant focus so client can offer data
    compositor.debug_focus_first_client_for_test();
    flags.focus_granted.store(true, Ordering::SeqCst);

    // Tick until clipboard cache has the offered data
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
        "compositor clipboard cache should match offered data"
    );
    Ok(())
}
