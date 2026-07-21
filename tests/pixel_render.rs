//! Pixel-level render verification for the Axiom compositor (Winit / GLES backend).
//!
//! This is the project's last unproven risk: does a *real* Wayland client's
//! SHM buffer actually reach the screen with correct scale/layout, and does
//! the SSD titlebar fail to paint over the client's content?
//!
//! It starts the compositor on the **Winit** backend (real GL render to a
//! window — the `Noop` backend early-returns and never produces pixels),
//! connects a real Wayland client that creates an `xdg_toplevel` with a
//! bright-red SHM buffer, ticks until the client is configured/committed/
//! rendered, then reads back the framebuffer via
//! `AxiomSmithayBackendReal::capture_pixels` and asserts:
//!
//!   1. the client's red color is present in the frame (not a black screen),
//!   2. red content appears well below the SSD titlebar band (no overlap
//!      bug where the titlebar paints over the whole client), and
//!   3. the red region is a sane size (the client was composited at a real
//!      scale, not a stray pixel).
//!
//! Requires an X display + GL context, so it is `#[ignore]` and run under
//! `xvfb-run`. A plain `cargo test` (no display) skips it and still passes.
//!
//! CI / local command:
//! ```text
//! xvfb-run -a cargo test --test pixel_render -- --ignored --nocapture
//! ```

use anyhow::Result;
use axiom::{
    backend::AxiomSmithayBackendReal,
    config::AxiomConfig,
    input::InputManager,
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

/// Size of the client's known-solid-red SHM buffer.
const CLIENT_W: u32 = 256;
const CLIENT_H: u32 = 192;
/// Bright red, opaque, in ARGB8888 byte order (B, G, R, A).
const RED_PIXEL: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];

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
                    let mut file = tempfile::tempfile().expect("tempfile for shm pool");
                    let bytes = (CLIENT_W * CLIENT_H * 4) as usize;
                    file.set_len(bytes as u64).expect("size shm pool");
                    {
                        use std::io::Write;
                        let mut buf = std::io::BufWriter::new(&mut file);
                        for _ in 0..bytes / 4 {
                            buf.write_all(&RED_PIXEL).unwrap();
                        }
                        buf.flush().unwrap();
                    }

                    let pool = shm.create_pool(file.as_fd(), bytes as i32, qh, ());
                    let buffer = pool.create_buffer(
                        0,
                        CLIENT_W as i32,
                        CLIENT_H as i32,
                        (CLIENT_W * 4) as i32,
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

/// Drive a real Wayland client to completion on a worker thread.
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

        for _ in 0..128 {
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

#[test]
#[ignore]
#[serial_test::serial]
fn test_pixel_render_shows_client_and_respects_titlebar() -> Result<()> {
    let mut config = AxiomConfig::default();
    // Use the real Winit/GL backend (the only one that actually renders pixels).
    config.backend.kind = "winit".to_string();

    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(
        &config.input,
        &config.bindings,
    )));
    let decoration_manager = Arc::new(RwLock::new(axiom::decoration::DecorationManager::new(
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

    // Initialize winit + GL (requires a display; provided by xvfb-run in CI).
    backend.initialize()?;

    // Point the client at the compositor's socket (bound as wayland-axiom-<pid>).
    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let client_done = done.clone();
    let client_handle = thread::spawn(move || run_client(client_done, tx));

    // Tick the compositor: accept the client, dispatch, render. Capture once
    // the client's red buffer has actually been composited.
    let captured: Option<(u32, u32, Vec<u8>)> = {
        let mut captured: Option<(u32, u32, Vec<u8>)> = None;
        for _ in 0..240 {
            backend.run_one_cycle()?;
            if let Some((cw, ch, px)) = backend.capture_pixels() {
                // Count red pixels inline to decide when we have a real frame.
                if px.len() == (cw as usize) * (ch as usize) * 4
                    && red_pixel_count(&px) > 2000
                {
                    captured = Some((cw, ch, px));
                    break;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        captured
    };

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    let (w, h, pixels) = captured.ok_or_else(|| {
        anyhow::anyhow!(
            "capture_pixels returned None / no red frame — no GL frame available (display/GL missing?)"
        )
    })?;

    assert_eq!(
        pixels.len(),
        (w as usize) * (h as usize) * 4,
        "captured pixel buffer has wrong length"
    );

    // Analyze the frame. glReadPixels returns bottom-left origin, so buffer row
    // `row` maps to display y = h - 1 - row.
    let mut red_count: usize = 0;
    let mut band_red: usize = 0; // red pixels in the top 30px display band (titlebar)
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut min_dy = u32::MAX;
    let mut max_dy = 0u32;
    let mut red_below_titlebar = false;

    for row in 0..(h as i32) {
        let display_y = (h as i32) - 1 - row;
        let in_band = display_y < 30;
        for x in 0..(w as i32) {
            let o = ((row as usize) * (w as usize) + (x as usize)) * 4;
            let (r, g, b, a) = (pixels[o], pixels[o + 1], pixels[o + 2], pixels[o + 3]);
            if is_red(r, g, b, a) {
                red_count += 1;
                if in_band {
                    band_red += 1;
                }
                if display_y > 60 {
                    red_below_titlebar = true;
                }
                let (xu, du) = (x as u32, display_y as u32);
                min_x = min_x.min(xu);
                max_x = max_x.max(xu);
                min_dy = min_dy.min(du);
                max_dy = max_dy.max(du);
            }
        }
    }

    // 1) The client's color actually reached the screen (not a black frame).
    assert!(
        red_count > 2000,
        "expected client red pixels to be composited; found only {red_count} red pixels"
    );

    // 2) Client content is visible well below the SSD titlebar — i.e. the
    //    titlebar did NOT paint over the entire client.
    assert!(
        red_below_titlebar,
        "no client red content found below the titlebar band (possible titlebar/content overlap)"
    );

    // 3) The red region is a sane size: composited at a real scale, not a dot.
    let bb_w = max_x.saturating_sub(min_x) + 1;
    let bb_h = max_dy.saturating_sub(min_dy) + 1;
    assert!(
        bb_w > 40 && bb_h > 40,
        "red region bounding box too small (w={bb_w}, h={bb_h}) — client not composited at expected scale"
    );

    // 4) The titlebar band is mostly NOT client-red (only the small close
    //    button should be red there), proving the titlebar is drawn on top
    //    rather than the client covering it.
    let band_total = (w as usize) * 30.min(h as usize);
    let band_fraction = band_red as f64 / band_total.max(1) as f64;
    assert!(
        band_fraction < 0.10,
        "titlebar band is {:.1}% red — looks like the titlebar painted over the client",
        band_fraction * 100.0
    );

    Ok(())
}

#[inline]
fn is_red(r: u8, g: u8, b: u8, a: u8) -> bool {
    r >= 180 && g <= 80 && b <= 80 && a >= 128
}

fn red_pixel_count(px: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i + 3 < px.len() {
        if is_red(px[i], px[i + 1], px[i + 2], px[i + 3]) {
            n += 1;
        }
        i += 4;
    }
    n
}
