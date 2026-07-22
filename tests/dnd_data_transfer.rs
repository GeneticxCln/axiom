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

// ── Helper: headless compositor (noop backend) ──────────────────────────

fn make_headless_compositor(
    config: AxiomConfig,
) -> Result<axiom::compositor::AxiomCompositor> {
    use axiom::compositor::AxiomCompositor;
    use axiom::ipc::AxiomIPCServer;

    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let ipc_server = AxiomIPCServer::new();

    let mut cfg = config.clone();
    cfg.backend.kind = "noop".to_string();

    AxiomCompositor::new(
        cfg,
        false,
        workspace_manager.clone(),
        window_manager.clone(),
        input_manager.clone(),
        ipc_server,
    )
}

// ── start_server_dnd unit tests ─────────────────────────────────────────

/// start_server_dnd populates the clipboard cache with the given data.
#[test]
#[serial_test::serial]
fn test_start_server_dnd_populates_cache() -> Result<()> {
    let mut backend = make_headless_backend()?;
    let payload = b"hello from server dnd".to_vec();
    backend.start_server_dnd(payload.clone(), "text/plain".into());

    assert_eq!(
        backend.state.clipboard_cache,
        Some(payload),
        "clipboard_cache should contain the DnD payload"
    );
    Ok(())
}

/// start_server_dnd with text/plain — data is cached and serveable.
#[test]
#[serial_test::serial]
fn test_start_server_dnd_text_plain() -> Result<()> {
    let mut backend = make_headless_backend()?;
    let payload = b"plain text payload".to_vec();
    backend.start_server_dnd(payload.clone(), "text/plain".into());

    assert_eq!(backend.state.clipboard_cache, Some(payload.clone()));

    // Verify the data can be served via ServerDndGrabHandler::send
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
    assert_eq!(buf, payload, "ServerDndGrabHandler::send serves cached data");
    Ok(())
}

/// start_server_dnd with text/html MIME type.
#[test]
#[serial_test::serial]
fn test_start_server_dnd_text_html() -> Result<()> {
    let mut backend = make_headless_backend()?;
    let payload = b"<html><body>Hello</body></html>".to_vec();
    backend.start_server_dnd(payload.clone(), "text/html".into());

    assert_eq!(backend.state.clipboard_cache, Some(payload.clone()));

    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    assert_eq!(rc, 0, "pipe2");
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    let seat = backend.state.seat.clone();
    ServerDndGrabHandler::send(&mut backend.state, "text/html".into(), write_fd, seat);

    let mut buf = Vec::new();
    let mut file = std::fs::File::from(read_fd);
    file.read_to_end(&mut buf)?;
    assert_eq!(buf, payload, "HTML payload served correctly");
    Ok(())
}

/// start_server_dnd with application/octet-stream (binary MIME type).
#[test]
#[serial_test::serial]
fn test_start_server_dnd_binary_mime() -> Result<()> {
    let mut backend = make_headless_backend()?;
    let payload = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0x7F];
    backend.start_server_dnd(payload.clone(), "application/octet-stream".into());

    assert_eq!(backend.state.clipboard_cache, Some(payload.clone()));

    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    assert_eq!(rc, 0, "pipe2");
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    let seat = backend.state.seat.clone();
    ServerDndGrabHandler::send(
        &mut backend.state,
        "application/octet-stream".into(),
        write_fd,
        seat,
    );

    let mut buf = Vec::new();
    let mut file = std::fs::File::from(read_fd);
    file.read_to_end(&mut buf)?;
    assert_eq!(buf, payload, "binary payload served correctly");
    Ok(())
}

/// start_server_dnd with empty data payload.
#[test]
#[serial_test::serial]
fn test_start_server_dnd_empty_payload() -> Result<()> {
    let mut backend = make_headless_backend()?;
    let payload: Vec<u8> = vec![];
    backend.start_server_dnd(payload.clone(), "text/plain".into());

    assert_eq!(
        backend.state.clipboard_cache,
        Some(payload),
        "clipboard_cache should contain empty vec"
    );

    // Serving empty data should produce an empty read
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
    assert!(buf.is_empty(), "empty payload produces empty read");
    Ok(())
}

/// start_server_dnd with a large payload (> 64KB).
///
/// Reads from the pipe in a helper thread since the kernel pipe buffer
/// (~64KB) is smaller than the payload — a synchronous write_all would
/// block waiting for a reader.
#[test]
#[serial_test::serial]
fn test_start_server_dnd_large_payload() -> Result<()> {
    let mut backend = make_headless_backend()?;
    // 128KB of repeating pattern
    let payload = vec![0xABu8; 128 * 1024];
    backend.start_server_dnd(payload.clone(), "application/octet-stream".into());

    assert_eq!(
        backend.state.clipboard_cache.as_ref().map(|v| v.len()),
        Some(payload.len()),
        "large payload should be fully cached"
    );
    assert_eq!(
        backend.state.clipboard_cache,
        Some(payload.clone()),
        "large payload should match exactly"
    );

    // Verify the large payload can be served — read from pipe in a thread
    // so the write_all inside ServerDndGrabHandler::send does not deadlock
    // on a full pipe buffer.
    let mut fds = [0i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    assert_eq!(rc, 0, "pipe2");
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    let reader = thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        let mut file = std::fs::File::from(read_fd);
        let _ = file.read_to_end(&mut buf);
        buf
    });

    let seat = backend.state.seat.clone();
    ServerDndGrabHandler::send(
        &mut backend.state,
        "application/octet-stream".into(),
        write_fd,
        seat,
    );

    let buf = reader.join().expect("pipe reader thread");
    assert_eq!(buf.len(), payload.len(), "large payload full length served");
    assert_eq!(buf, payload, "large payload content matches");
    Ok(())
}

/// Full round-trip: start_server_dnd → ServerDndGrabHandler::send serves data.
#[test]
#[serial_test::serial]
fn test_start_server_dnd_round_trip() -> Result<()> {
    let mut backend = make_headless_backend()?;
    let payload = b"round-trip-dnd-payload".to_vec();
    backend.start_server_dnd(payload.clone(), "text/plain".into());

    // Verify the cache is populated
    assert_eq!(backend.state.clipboard_cache, Some(payload.clone()));

    // Read the data back through the DnD handler
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
    assert_eq!(buf, payload, "round-trip: start_server_dnd → send");
    Ok(())
}

// ── IPC integration test ─────────────────────────────────────────────────

/// Full IPC integration: StartDnd message flows through the compositor and
/// populates the clipboard cache.
#[test]
#[serial_test::serial]
fn test_ipc_start_dnd_compositor_dispatch() -> Result<()> {
    use axiom::ipc::LazyUIMessage;

    let config = AxiomConfig::default();
    let mut compositor = make_headless_compositor(config)?;

    // Get the IPC command sender
    let sender = compositor.ipc_command_sender();

    // Send a StartDnd command via the IPC channel
    let cmd = LazyUIMessage::StartDnd {
        text: "ipc-dnd-text".into(),
        mime_type: "text/plain".into(),
    };
    sender.send(cmd).unwrap();

    // Run a tick — this should process the IPC message and call
    // start_server_dnd on the backend, which populates the clipboard cache.
    let result = compositor.tick_for_test();
    assert!(result.is_ok(), "tick should succeed");

    // Verify the clipboard cache was populated with the DnD text
    let cached = compositor.debug_clipboard_cache();
    assert!(cached.is_some(), "clipboard cache should be populated after StartDnd IPC");
    assert_eq!(
        cached.as_deref().unwrap(),
        b"ipc-dnd-text",
        "clipboard data should match the DnD text"
    );

    Ok(())
}

/// IPC integration: StartDnd with a different MIME type (text/html).
#[test]
#[serial_test::serial]
fn test_ipc_start_dnd_html_mime() -> Result<()> {
    use axiom::ipc::LazyUIMessage;

    let config = AxiomConfig::default();
    let mut compositor = make_headless_compositor(config)?;

    let sender = compositor.ipc_command_sender();
    let cmd = LazyUIMessage::StartDnd {
        text: "<p>html content</p>".into(),
        mime_type: "text/html".into(),
    };
    sender.send(cmd).unwrap();

    let result = compositor.tick_for_test();
    assert!(result.is_ok(), "tick should succeed");

    let cached = compositor.debug_clipboard_cache();
    assert!(cached.is_some(), "clipboard cache should be populated");
    assert_eq!(
        cached.as_deref().unwrap(),
        b"<p>html content</p>",
        "clipboard data should match HTML DnD text"
    );

    Ok(())
}

/// IPC integration: StartDnd with empty text.
#[test]
#[serial_test::serial]
fn test_ipc_start_dnd_empty_text() -> Result<()> {
    use axiom::ipc::LazyUIMessage;

    let config = AxiomConfig::default();
    let mut compositor = make_headless_compositor(config)?;

    let sender = compositor.ipc_command_sender();
    let cmd = LazyUIMessage::StartDnd {
        text: String::new(),
        mime_type: "text/plain".into(),
    };
    sender.send(cmd).unwrap();

    let result = compositor.tick_for_test();
    assert!(result.is_ok(), "tick should succeed");

    let cached = compositor.debug_clipboard_cache();
    assert!(cached.is_some(), "clipboard cache should be Some (empty vec)");
    assert!(
        cached.unwrap().is_empty(),
        "clipboard data should be empty for empty DnD text"
    );

    Ok(())
}

/// IPC integration: StartDnd with large text (> 64KB).
#[test]
#[serial_test::serial]
fn test_ipc_start_dnd_large_text() -> Result<()> {
    use axiom::ipc::LazyUIMessage;

    let config = AxiomConfig::default();
    let mut compositor = make_headless_compositor(config)?;

    let sender = compositor.ipc_command_sender();
    // 128KB of repeating 'A'
    let large_text = "A".repeat(128 * 1024);
    let cmd = LazyUIMessage::StartDnd {
        text: large_text.clone(),
        mime_type: "text/plain".into(),
    };
    sender.send(cmd).unwrap();

    let result = compositor.tick_for_test();
    assert!(result.is_ok(), "tick should succeed");

    let cached = compositor.debug_clipboard_cache();
    assert!(cached.is_some(), "clipboard cache should be populated");
    assert_eq!(
        cached.as_deref().unwrap().len(),
        large_text.len(),
        "large text should be fully cached"
    );
    assert_eq!(
        cached.as_deref().unwrap(),
        large_text.as_bytes(),
        "large text content should match"
    );

    Ok(())
}
