//! Core Wayland protocol registration helpers for Smithay-based backends
//!
//! These functions wrap Smithay's protocol state constructors and provide a
//! single place to register globals on a given DisplayHandle.

use anyhow::Result;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::{
    compositor::CompositorState,
    output::OutputManagerState,
    shell::xdg::XdgShellState,
    shm::ShmState,
    seat::{Seat, SeatState},
};
use smithay::input::{keyboard::KeyboardHandle, pointer::PointerHandle};

/// Register wl_compositor global and return its state
pub fn register_compositor<S>(display: &mut Display<S>) -> CompositorState
where
    S: smithay::wayland::compositor::CompositorHandler + 'static,
{
    CompositorState::new::<S>(display)
}

/// Register wl_shm global and return its state
pub fn register_shm<S>(display: &mut Display<S>) -> ShmState
where
    S: smithay::wayland::shm::ShmHandler + 'static,
{
    ShmState::new::<S>(display, vec![])
}

/// Register xdg_wm_base global and return its state
pub fn register_xdg_shell<S>(display: &mut Display<S>) -> XdgShellState
where
    S: smithay::wayland::shell::xdg::XdgShellHandler + 'static,
{
    XdgShellState::new::<S>(display)
}

/// Register wl_output/xdg_output globals and return the output manager state
pub fn register_output_manager<S>(display: &mut Display<S>) -> OutputManagerState
where
    S: 'static,
{
    // Smithay 0.3 uses OutputManagerState::new()
    OutputManagerState::new::<S>(display)
}

/// Create a wl_seat with keyboard and pointer and return handles
pub fn create_seat<S>(
    display: &mut Display<S>,
    seat_state: &mut SeatState<S>,
    name: &str,
) -> Result<(Seat<S>, KeyboardHandle<S>, PointerHandle<S>)>
where
    S: smithay::wayland::seat::SeatHandler + 'static,
{
    let mut seat = seat_state.new_wl_seat(display, name);
    let keyboard = seat.add_keyboard(Default::default(), 200, 25)?;
    let pointer = seat.add_pointer();
    Ok((seat, keyboard, pointer))
}
