//! Real-client smoke test for the Axiom compositor (headless / Noop backend).
//!
//! Proves end-to-end that a Wayland *client* can:
//!   1. connect to the compositor's listening socket (bound in `XDG_RUNTIME_DIR`
//!      as `wayland-axiom-<pid>` by `AxiomSmithayBackendReal::new`),
//!   2. bind `wl_compositor` + `wl_shm` + `xdg_wm_base`,
//!   3. create an `xdg_toplevel` and attach/commit a shared-memory buffer,
//!   4. and the compositor registers that surface (`window_count() >= 1`).
//!
//! The compositor runs in-process on the Noop backend (no GPU/winit), ticked
//! explicitly from this test. The client runs on a separate thread because the
//! `wayland-client` API is synchronous and the server only dispatches clients
//! when we tick.

use anyhow::Result;
use axiom::{
    compositor::AxiomCompositor,
    config::AxiomConfig,
    input::InputManager,
    ipc::AxiomIPCServer,
    window::WindowManager,
    workspace::ScrollableWorkspaces,
};
use parking_lot::RwLock;
use std::os::fd::AsFd;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// Build a fully-initialized headless (Noop) compositor and keep a clone of the
/// `window_manager` Arc so the test can observe surface registration. Mirrors
/// `make_test_compositor` from `integration_tests.rs` but returns the Arc too.
async fn make_headless_compositor(
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
    )
    .await?;

    Ok((compositor, window_manager))
}

struct ClientState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    configured: bool,
    toplevel_created: bool,
}

impl ClientState {
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
        self.toplevel_created = true;
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
                    let shm = registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ());
                    let (w, h) = (64u32, 64u32);

                    let mut file = tempfile::tempfile().expect("tempfile for shm pool");
                    let bytes = (w * h * 4) as usize;
                    file.set_len(bytes as u64).expect("size shm pool");
                    {
                        use std::io::Write;
                        let mut buf = std::io::BufWriter::new(&mut file);
                        let pixel = [0u8, 0, 0, 0xFFu8]; // opaque black, ARGB
                        for _ in 0..bytes / 4 {
                            buf.write_all(&pixel).unwrap();
                        }
                        buf.flush().unwrap();
                    }

                    let pool = shm.create_pool(file.as_fd(), bytes as i32, qh, ());
                    let buffer = pool.create_buffer(
                        0,
                        w as i32,
                        h as i32,
                        (w * 4) as i32,
                        wl_shm::Format::Argb8888,
                        qh,
                        (),
                    );

                    if let Some(surface) = state.surface.as_ref() {
                        surface.attach(Some(&buffer), 0, 0);
                        surface.commit();
                    }
                    let _ = buffer;
                    state.shm = Some(shm);
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
delegate_noop!(ClientState: ignore wl_buffer::WlBuffer);

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

/// Drive a real Wayland client to completion on a worker thread. Returns an
/// error string if anything goes wrong so the test can surface it.
fn run_client(done: Arc<AtomicBool>, result_tx: mpsc::Sender<String>) {
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
            surface: None,
            configured: false,
            toplevel_created: false,
        };

        // Pump the queue until the toplevel exists and the surface is configured.
        for _ in 0..64 {
            event_queue.blocking_dispatch(&mut state)?;
            if state.toplevel_created && state.configured {
                break;
            }
        }
        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
    done.store(true, Ordering::SeqCst);
}

#[tokio::test]
#[serial_test::serial]
async fn test_real_client_connects_and_maps_toplevel() -> Result<()> {
    let config = AxiomConfig::default();
    let (mut compositor, window_manager) = make_headless_compositor(config).await?;

    // Point the client at the compositor's socket. `AxiomSmithayBackendReal::new`
    // binds `wayland-axiom-<pid>` in XDG_RUNTIME_DIR, so we expose it via
    // WAYLAND_DISPLAY before the client connects.
    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let client_done = done.clone();
    let client_handle = thread::spawn(move || run_client(client_done, tx));

    // Tick the compositor until the client has finished AND the surface is tracked.
    let mut ticks = 0;
    while !done.load(Ordering::SeqCst) && ticks < 200 {
        compositor.tick_for_test().await?;
        ticks += 1;
        thread::sleep(Duration::from_millis(5));
    }

    // Give the client thread a chance to report once `done` is set.
    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();

    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    let count = window_manager.read().window_count();
    assert!(
        count >= 1,
        "compositor did not track the client toplevel (window_count={count})"
    );

    Ok(())
}
