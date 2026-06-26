//! XWayland message dispatch glue.
//!
//! This module sits between `AxiomSmithayBackendReal::run_one_cycle` and
//! the X11-side `AxiomXwm` in `crate::backend::xwm`. It owns:
//!
//! - The X11 event poll loop
//! - Plumbing of X11 `SelectionRequest` events to the cached Wayland
//!   clipboard payload stored in [`State::clipboard_cache`]
//! - The clipboard-data-to-X11-owner wiring exposed as
//!   [`set_clipboard_data`].
//!
//! Extracted from `src/backend/mod.rs` so the X11 dispatch path can be
//! read in isolation from the Wayland protocol-state machine.

use anyhow::Result;
use log::warn;

// Re-export the parent module's State struct so signatures below can spell
// `&State` / `&mut State` without the full `crate::backend::State` path.
use super::State;

use super::xwm::{AxiomXwm, XwmEvent};

/// Poll X11 events coming from the XWayland manager and dispatch them
/// to the compositor. Currently:
///
/// - `SelectionRequest` → [`handle_clipboard_request`]
/// - All other events are handled inside `AxiomXwm::handle_event`
///   (MapRequest bookkeeping, ConfigureRequest grants, UnmapNotify
///   removal, SelectionNotify logging).
///
/// Returns `Ok(())` if no XWM is wired (no-op). Propagates errors from
/// `xwm.handle_event` or `xwm.handle_selection_request`.
pub(super) fn poll_and_dispatch_events(state: &mut State) -> Result<()> {
    let Some(xwm) = state.xwm.as_mut() else {
        return Ok(());
    };
    while let Some(event) = xwm.poll_event() {
        match xwm.handle_event(&event) {
            Ok(Some(XwmEvent::ClipboardRequest {
                requestor,
                selection,
                target,
                property,
                time,
            })) => {
                let clipboard_data = state.clipboard_cache.as_deref();
                handle_clipboard_request(
                    xwm,
                    requestor,
                    selection,
                    target,
                    property,
                    time,
                    clipboard_data,
                )?;
            }
            Ok(_) => {}
            Err(e) => warn!("⚠️ XWM event error: {}", e),
        }
    }
    Ok(())
}

/// Serve the cached Wayland clipboard data (or a placeholder) to a
/// requesting X11 client. See [`super::xwm::AxiomXwm::handle_selection_request`]
/// for the per-target handling logic.
fn handle_clipboard_request(
    xwm: &AxiomXwm,
    requestor: x11rb::protocol::xproto::Window,
    selection: x11rb::protocol::xproto::Atom,
    target: x11rb::protocol::xproto::Atom,
    property: x11rb::protocol::xproto::Atom,
    time: x11rb::protocol::xproto::Timestamp,
    clipboard_data: Option<&[u8]>,
) -> Result<()> {
    if let Err(e) =
        xwm.handle_selection_request(requestor, selection, target, property, time, clipboard_data)
    {
        warn!("⚠️ Failed to serve X11 clipboard request: {}", e);
    }
    Ok(())
}

// (No second `use super::State` re-import needed — the line at the top
// of this file aliases `super::State` directly so all signatures below
// can spell `State` plain.)
