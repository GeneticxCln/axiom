//! Screencopy protocol integration test.
//!
//! Verifies that `zwlr_screencopy_manager_v1` version 1 SHM capture works end-to-end.
//! The compositor runs on the Winit/GL backend (needs display), connects a real
//! Wayland client with a visible surface, captures the composited scene, and
//! asserts the capture succeeded (via `ready` event).
//!
//! Run with:
//! ```text
//! xvfb-run -a cargo test --test screencopy_test -- --ignored --nocapture
//! ```

use anyhow::Result;
use axiom::{
    backend::AxiomSmithayBackendReal, config::AxiomConfig, decoration::DecorationManager,
    input::InputManager, window::WindowManager, workspace::ScrollableWorkspaces,
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
    protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1;
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1;
use zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

const CLIENT_W: i32 = 256;
const CLIENT_H: i32 = 192;

struct ClientState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    output: Option<wl_output::WlOutput>,
    surface: Option<wl_surface::WlSurface>,
    configured: bool,
    // Screencopy state
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    capture_frame: Option<ZwlrScreencopyFrameV1>,
    capture_buffer: Option<wl_buffer::WlBuffer>,
    capture_done: bool,
    capture_ok: bool,
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
        surface.commit();
        self.surface = Some(surface);
    }

    fn try_capture(&mut self, qh: &QueueHandle<Self>) {
        let (manager, output) = match (self.screencopy_manager.as_ref(), self.output.as_ref()) {
            (Some(m), Some(out)) => (m, out),
            _ => return,
        };
        if self.capture_frame.is_some() {
            return;
        }
        let frame = manager.capture_output(0, output, qh, ());
        self.capture_frame = Some(frame);
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
                    state.compositor = Some(registry.bind(name, 1, qh, ()));
                    state.init_xdg_surface(qh);
                }
                "wl_shm" => {
                    let shm: wl_shm::WlShm = registry.bind(name, 1, qh, ());
                    let mut file = tempfile::tempfile().expect("tempfile");
                    let bytes = (CLIENT_W * CLIENT_H * 4) as u64;
                    file.set_len(bytes).expect("set_len");
                    {
                        use std::io::Write;
                        let mut buf = std::io::BufWriter::new(&mut file);
                        for _ in 0..(CLIENT_W * CLIENT_H) {
                            buf.write_all(&[0x00, 0x00, 0xFF, 0xFF]).unwrap(); // RGBA red
                        }
                    }
                    let pool = shm.create_pool(file.as_fd(), bytes as i32, qh, ());
                    let buffer = pool.create_buffer(
                        0,
                        CLIENT_W,
                        CLIENT_H,
                        CLIENT_W * 4,
                        wl_shm::Format::Argb8888,
                        qh,
                        (),
                    );
                    if let Some(surface) = state.surface.as_ref() {
                        surface.attach(Some(&buffer), 0, 0);
                        surface.commit();
                    }
                    state.shm = Some(shm);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, 1, qh, ()));
                    state.init_xdg_surface(qh);
                }
                "zwlr_screencopy_manager_v1" => {
                    let mgr: ZwlrScreencopyManagerV1 = registry.bind(name, 1, qh, ());
                    state.screencopy_manager = Some(mgr);
                    state.try_capture(qh);
                }
                "wl_output" => {
                    let output: wl_output::WlOutput = registry.bind(name, 1, qh, ());
                    state.output = Some(output);
                    state.try_capture(qh);
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
delegate_noop!(ClientState: ignore wl_output::WlOutput);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
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
        event: <xdg_surface::XdgSurface as Proxy>::Event,
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
        _: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyManagerV1,
        _: <ZwlrScreencopyManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _frame: &ZwlrScreencopyFrameV1,
        event: <ZwlrScreencopyFrameV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            // ponytail: `format` is WEnum<Format> — unwrap to the inner value
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let Some(shm) = state.shm.as_ref() {
                    let fmt = match format {
                        WEnum::Value(f) => f,
                        _ => return,
                    };
                    let pool_size = (stride * height) as i64;
                    let file = tempfile::tempfile().expect("tempfile");
                    file.set_len(pool_size as u64).expect("set_len");
                    let pool = shm.create_pool(file.as_fd(), pool_size as i32, qh, ());
                    let buf = pool.create_buffer(
                        0,
                        width as i32,
                        height as i32,
                        stride as i32,
                        fmt,
                        qh,
                        (),
                    );
                    state.capture_buffer = Some(buf);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                if let (Some(frame), Some(buffer)) =
                    (state.capture_frame.as_ref(), state.capture_buffer.as_ref())
                {
                    frame.copy(buffer);
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags { .. } => {}
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.capture_ok = true;
                state.capture_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.capture_ok = false;
                state.capture_done = true;
            }
            _ => {}
        }
    }
}

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
            output: None,
            surface: None,
            configured: false,
            screencopy_manager: None,
            capture_frame: None,
            capture_buffer: None,
            capture_done: false,
            capture_ok: false,
        };

        for _ in 0..256 {
            event_queue.blocking_dispatch(&mut state)?;
            if state.capture_done {
                break;
            }
        }
        if !state.capture_done {
            anyhow::bail!("timeout: capture never completed");
        }
        if !state.capture_ok {
            anyhow::bail!("capture failed");
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

#[test]
#[ignore]
#[serial_test::serial]
fn test_screencopy_capture() -> Result<()> {
    let mut config = AxiomConfig::default();
    config.backend.kind = "winit".to_string();

    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let decoration_manager = Arc::new(RwLock::new(DecorationManager::new(
        &config.window,
        config.features.enable_minimize,
    )));

    let mut backend = AxiomSmithayBackendReal::new(
        config,
        window_manager,
        workspace_manager,
        input_manager,
        decoration_manager,
    )?;
    backend.initialize()?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let client_done = done.clone();
    let client_handle = thread::spawn(move || run_client(client_done, tx));

    for _ in 0..240 {
        backend.run_one_cycle()?;
        if done.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    Ok(())
}
