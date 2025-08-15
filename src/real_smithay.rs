//! Real Smithay compositor (Phase 5): actual Wayland globals + event loop

use anyhow::Result;
use log::{info, warn};
use std::{sync::Arc, time::Duration};

use smithay::{
    reexports::calloop::{self, EventLoop},
    reexports::wayland_server::{backend::ClientData, Display, DisplayHandle},
    utils::{Physical, Rectangle, Size},
    wayland::{
        compositor::{CompositorClientState, CompositorHandler, CompositorState, Surface},
        output::{Mode, Output, OutputManagerState, PhysicalProperties, Subpixel},
        seat::{Seat, SeatHandler, SeatState},
        shell::xdg::{XdgShellHandler, XdgShellState, XdgToplevelSurface},
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
    },
};

use crate::config::AxiomConfig;

/// Simple client data for the compositor
struct AxiomClientData;
impl ClientData for AxiomClientData {}

// Minimal compositor state implementing Smithay handler traits.
struct State {
    compositor_state: CompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
    seat_state: SeatState<Self>,
    output_manager_state: OutputManagerState,
}

impl State {
    fn new(display: &mut Display<State>, _config: &AxiomConfig, _windowed: bool) -> Self {
        let compositor_state = CompositorState::new::<State>(display);
        let shm_state = ShmState::new::<State>(display, vec![]);
        let xdg_shell_state = XdgShellState::new::<State>(display);
        let seat_state = SeatState::new();
        let output_manager_state = OutputManagerState::new();

        Self {
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            output_manager_state,
        }
    }

    fn init_output(&mut self, display: &Display<State>) {
        let mut output = Output::new(
            display,
            "AXIOM-OUTPUT".to_string(),
            PhysicalProperties {
                size: (600, 340).into(),
                subpixel: Subpixel::Unknown,
                make: "Axiom".into(),
                model: "Virtual".into(),
            },
        );

        let mode = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);
        self.output_manager_state.output_created(&output);
    }

    fn init_seat(&mut self, display: &Display<State>) {
        let seat: Seat<State> = Seat::new(display, "seat-0");
        let _ = seat.add_keyboard(Default::default(), 200, 25);
        seat.add_pointer();
    }
}

// Handlers: Minimal implementations to support clients connecting and creating toplevels.
impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn commit(&mut self, _dh: &DisplayHandle, _surface: &Surface) {}
}

impl ShmHandler for State {}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }
    fn new_toplevel(&mut self, _dh: &DisplayHandle, _toplevel: XdgToplevelSurface) {
        info!("🪟 New xdg_toplevel created");
    }
}

smithay::delegate_compositor!(State);
smithay::delegate_shm!(State);
smithay::delegate_xdg_shell!(State);
smithay::delegate_output!(State);
smithay::delegate_seat!(State);

impl SeatHandler for State {
    type KeyboardFocus = Surface;
    type PointerFocus = Surface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

/// Run the real Smithay compositor with functional Wayland globals and event loop.
pub fn run_real_compositor(config: AxiomConfig) -> Result<()> {
    info!("🚀 Starting real Smithay Wayland compositor");

    // Create Wayland display and compositor state
    let mut display: Display<State> = Display::new()?;
    let mut state = State::new(&mut display, &config, /*windowed*/ false);

    // Create a listening socket automatically and expose WAYLAND_DISPLAY
    let listening = ListeningSocketSource::new_auto()?;
    let socket_name = listening.socket_name().to_string();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    info!("✨ WAYLAND_DISPLAY={}", socket_name);

    // Prepare calloop event loop
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    // Display handle for inserting clients
    let dh = display.handle();
    handle.insert_source(listening, move |client_stream, _, state| {
        if let Err(e) = dh.insert_client(client_stream, Arc::new(CompositorClientState::default())) {
            warn!("Failed to insert client: {}", e);
        }
    })?;

    // Initialize output and seat
    state.init_output(&display);
    state.init_seat(&display);

    // Add a periodic timer to keep the loop ticking (~60fps)
    let timer = calloop::timer::Timer::new()?;
    let _ = handle.insert_source(timer, |_, _, _state| {})?;

    info!("🎬 Event loop running...");
    loop {
        event_loop.dispatch(Duration::from_millis(16), &mut state)?;
        display.flush_clients()?;
    }
}

