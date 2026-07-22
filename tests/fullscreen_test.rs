//! Fullscreen protocol notification test.
//!
//! Verifies the compositor sends `xdg_toplevel::State::Fullscreen` in configure
//! events when fullscreen is toggled, and clears it when toggled off.
//!
//! Requires an X display + GL context, so it is `#[ignore]` and run under
//! `xvfb-run`. A plain `cargo test` (no display) skips it and still passes.
//!
//! CI / local command:
//! ```text
//! xvfb-run -a cargo test --test fullscreen_test -- --ignored --nocapture
//! ```

use anyhow::Result;
use axiom::{
    backend::AxiomSmithayBackendReal, config::AxiomConfig, decoration::DecorationManager,
    input::InputManager, window::WindowManager, workspace::ScrollableWorkspaces,
};
use parking_lot::RwLock;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use wayland_client::{
    delegate_noop,
    protocol::{wl_compositor, wl_registry, wl_surface},
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

// ── Client-side state ───────────────────────────────────────────────────────

struct FullscreenClientState {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    /// Shared: the last configure states received from the compositor.
    last_states: Arc<Mutex<Vec<xdg_toplevel::State>>>,
    /// Shared: set to true once the initial xdg_surface.configure is acked.
    mapped: Arc<AtomicBool>,
    /// Shared: when true, the client loop should exit.
    done: Arc<AtomicBool>,
}

// ── Dispatch implementations ────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for FullscreenClientState {
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
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
            state.try_init_toplevel(qh);
        }
    }
}

delegate_noop!(FullscreenClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(FullscreenClientState: ignore wl_surface::WlSurface);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for FullscreenClientState {
    fn event(
        _state: &mut Self,
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

impl Dispatch<xdg_surface::XdgSurface, ()> for FullscreenClientState {
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
            state.mapped.store(true, Ordering::SeqCst);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for FullscreenClientState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { states, .. } = event {
            // `states` arrives as raw bytes (wire-format array of u32).
            // Decode into State enum values.
            let decoded: Vec<xdg_toplevel::State> = states
                .chunks_exact(4)
                .filter_map(|c| {
                    xdg_toplevel::State::try_from(u32::from_ne_bytes(
                        c.try_into().unwrap(),
                    ))
                    .ok()
                })
                .collect();
            let mut last = state.last_states.lock().unwrap();
            *last = decoded;
        }
    }
}

impl FullscreenClientState {
    fn try_init_toplevel(&mut self, qh: &QueueHandle<Self>) {
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

/// Drive the Wayland client on a worker thread.
fn run_client(
    last_states: Arc<Mutex<Vec<xdg_toplevel::State>>>,
    mapped: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    result_tx: mpsc::Sender<String>,
) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<FullscreenClientState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = FullscreenClientState {
            compositor: None,
            wm_base: None,
            surface: None,
            last_states,
            mapped,
            done,
        };

        // Phase 1 — wait until the initial configure arrives (surface is mapped)
        while !state.done.load(Ordering::SeqCst) {
            event_queue.blocking_dispatch(&mut state)?;
            if state.mapped.load(Ordering::SeqCst) {
                break;
            }
        }

        // Phase 2 — keep dispatching so we receive fullscreen configure
        // events.  Exit when the test framework signals done.
        while !state.done.load(Ordering::SeqCst) {
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

// ── Integration test ────────────────────────────────────────────────────────

#[test]
#[ignore = "needs xvfb-run (Winit/GL)"]
#[serial_test::serial]
fn test_fullscreen_protocol_notification() -> Result<()> {
    let mut config = AxiomConfig::default();
    config.backend.kind = "winit".to_string();

    // Build shared subsystems (pattern follows pixel_render.rs)
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
        window_manager.clone(),
        workspace_manager,
        input_manager,
        decoration_manager,
    )?;

    // Initialize winit + GL (requires a display; provided by xvfb-run in CI).
    backend.initialize()?;

    // Point the client at the compositor's socket (bound as wayland-axiom-<pid>).
    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    // Shared state between test and client thread
    let last_states: Arc<Mutex<Vec<xdg_toplevel::State>>> = Arc::new(Mutex::new(Vec::new()));
    let mapped = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let (tx, rx) = mpsc::channel();
    let c_last_states = Arc::clone(&last_states);
    let c_mapped = Arc::clone(&mapped);
    let c_done = Arc::clone(&done);
    let client_handle = thread::spawn(move || run_client(c_last_states, c_mapped, c_done, tx));

    // ── Wait for the client to map its surface ────────────────
    for _ in 0..200 {
        backend.run_one_cycle()?;
        if mapped.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        mapped.load(Ordering::SeqCst),
        "Client did not map within timeout"
    );

    // Grab the window ID the compositor assigned to the client's toplevel.
    let window_id = window_manager
        .read()
        .focused_window_id()
        .expect("No focused window after client mapped");

    // ── Toggle fullscreen ON ───────────────────────────────────
    backend.state.toggle_fullscreen_window(window_id);

    // Tick the compositor until the configure event reaches the client.
    for _ in 0..30 {
        backend.run_one_cycle()?;
        thread::sleep(Duration::from_millis(5));
    }

    {
        let states = last_states.lock().unwrap();
        assert!(
            states.contains(&xdg_toplevel::State::Fullscreen),
            "Expected Fullscreen state after toggle ON, got: {states:?}",
        );
    }

    // ── Toggle fullscreen OFF ──────────────────────────────────
    backend.state.toggle_fullscreen_window(window_id);

    for _ in 0..30 {
        backend.run_one_cycle()?;
        thread::sleep(Duration::from_millis(5));
    }

    {
        let states = last_states.lock().unwrap();
        assert!(
            !states.contains(&xdg_toplevel::State::Fullscreen),
            "Expected Fullscreen cleared after toggle OFF, got: {states:?}",
        );
    }

    // ── Clean shutdown ─────────────────────────────────────────
    done.store(true, Ordering::SeqCst);
    for _ in 0..10 {
        backend.run_one_cycle()?;
        thread::sleep(Duration::from_millis(1));
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    Ok(())
}
