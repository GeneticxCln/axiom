//! Headless (Noop) end-to-end test of the Wayland→compositor clipboard path.
//!
//! A real `wayland-client` offers a `text/plain` selection. The compositor's
//! `SelectionHandler::new_selection` (src/backend/mod.rs) reads the offered
//! bytes into `State::clipboard_cache` via the pipe worker in
//! src/backend/clipboard.rs. This test asserts the cache ends up equal to the
//! offered payload.
//!
//! Smithay denies `wl_data_device.set_selection` unless the offering client
//! holds keyboard focus on the seat (see smithay's data_device handler). In a
//! real session input grants that focus; headlessly we grant it through the
//! test-only `debug_focus_first_client_for_test` accessor. The data transfer
//! itself is genuinely exercised over the wire — nothing is faked.

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
    compositor::AxiomCompositor, config::AxiomConfig, input::InputManager, ipc::AxiomIPCServer,
    window::WindowManager, workspace::ScrollableWorkspaces,
};
use parking_lot::RwLock;

use wayland_client::{
    delegate_noop,
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// Offered clipboard payload — must round-trip to `State::clipboard_cache`.
const OFFERED: &str = "hello from client";

async fn make_headless_compositor(
    config: AxiomConfig,
) -> Result<(AxiomCompositor, std::sync::Arc<RwLock<WindowManager>>)> {
    let workspace_manager =
        std::sync::Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = std::sync::Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = std::sync::Arc::new(RwLock::new(InputManager::new(
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
    )
    .await?;

    Ok((compositor, window_manager))
}

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
        match (self.data_device_manager.as_ref(), self.seat.as_ref()) {
            (Some(mgr), Some(seat)) => {
                self.data_device = Some(mgr.get_data_device(seat, qh, ()));
            }
            _ => {}
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
        if let wl_registry::Event::Global { name, interface, .. } = event {
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
                            name, 3, qh, (),
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
delegate_noop!(ClientState: ignore wl_shm_pool::WlShmPool);
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

/// When the compositor requests our offer, write the payload to the fd it
/// handed us. This is the wire transfer the test actually exercises.
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
            let mut file = std::fs::File::from(fd);
            let _ = file.write_all(OFFERED.as_bytes());
            let _ = file.flush();
            // Dropping `file` closes the fd, signalling end-of-data to the
            // compositor's clipboard pipe reader.
            state.payload_written = true;
        }
    }
}

#[derive(Clone)]
struct Flags {
    focus_granted: Arc<AtomicBool>,
}

/// Drive the real Wayland client to completion. Uses a poll-based (non-blocking)
/// dispatch loop so it can observe `focus_granted` (set by the test once the
/// compositor has granted keyboard focus) without blocking forever waiting for
/// a Wayland event.
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

        for _ in 0..1024 {
            // Flush any buffered client→server requests (e.g. the selection
            // offer) so the compositor can act on them.
            let _ = event_queue.flush();

            // Read any pending Wayland events without blocking indefinitely:
            // poll the socket with a short timeout, then drain the queue.
            if let Some(guard) = event_queue.prepare_read() {
                let fd = guard.connection_fd().as_raw_fd();
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: `pfd` points to a single valid, initialized element.
                unsafe {
                    libc::poll(&mut pfd as *mut libc::pollfd, 1, 5);
                }
                if pfd.revents & libc::POLLIN != 0 {
                    let _ = guard.read();
                }
            }
            event_queue.dispatch_pending(&mut state)?;

            // Once the compositor has granted us keyboard focus (a real input
            // event would do this), offer the selection.
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

            // Keep pumping until our payload has been written to the compositor
            // (which it only learns about after a round-trip).
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

#[tokio::test]
#[serial_test::serial]
async fn test_client_clipboard_offer_reaches_compositor_cache() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, window_manager) = make_headless_compositor(config).await?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let flags = Flags {
        focus_granted: Arc::new(AtomicBool::new(false)),
    };

    let (tx, rx) = mpsc::channel();
    let client_flags = flags.clone();
    let client_handle = thread::spawn(move || run_client(client_flags, tx));

    // Tick until the client's toplevel is tracked by the compositor.
    let mut ticks = 0;
    while window_manager.read().window_count() < 1 && ticks < 200 {
        compositor.tick_for_test().await?;
        ticks += 1;
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        window_manager.read().window_count() >= 1,
        "compositor did not track the client toplevel"
    );

    // Grant the focused client keyboard/data-device focus so set_selection is
    // accepted (Smithay requires focus; a real session gets it via input).
    compositor.debug_focus_first_client_for_test();
    flags.focus_granted.store(true, Ordering::SeqCst);

    // Tick until the offer is read into the clipboard cache.
    let mut cached: Option<Vec<u8>> = None;
    for _ in 0..200 {
        compositor.tick_for_test().await?;
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
