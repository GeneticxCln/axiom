//! XWayland message dispatch glue.
//!
//! This module sits between `AxiomSmithayBackendReal::run_one_cycle` and
//! the X11-side `AxiomXwm` in `crate::backend::xwm`. It owns:
//!
//! - The X11 event poll loop
//! - Plumbing of X11 `SelectionRequest` events to the cached Wayland
//!   clipboard payload stored in [`State::clipboard_cache`]
//! - Best-effort ingestion of external X11 clipboard owners back into the
//!   compositor cache and Wayland data-device state
//! - The clipboard-data-to-X11-owner wiring exposed as
//!   [`set_clipboard_data`].
//!
//! Extracted from `src/backend/mod.rs` so the X11 dispatch path can be
//! read in isolation from the Wayland protocol-state machine.

use anyhow::Result;
use log::{info, warn};
use smithay::{
    reexports::wayland_server::DisplayHandle,
    wayland::selection::data_device::set_data_device_selection,
};

use super::xwm::{AxiomXwm, XwmEvent};
use super::State;

/// Collect and dispatch X11 events from XWayland.
///
/// Returns `Ok(())` if no XWM is wired (no-op). Propagates errors from
/// `xwm.handle_event`, clipboard owner polling, or `xwm.handle_selection_request`.
pub(super) fn poll_and_dispatch_events(
    display_handle: &DisplayHandle,
    state: &mut State,
) -> Result<()> {
    let mut clipboard_requests: Vec<(
        x11rb::protocol::xproto::Window,
        x11rb::protocol::xproto::Atom,
        x11rb::protocol::xproto::Atom,
        x11rb::protocol::xproto::Atom,
        x11rb::protocol::xproto::Timestamp,
    )> = Vec::new();
    let mut clipboard_updates: Vec<(u32, &'static str, Vec<u8>)> = Vec::new();
    let mut clipboard_cleared = false;
    let mut window_mapped: Vec<(u32, String, Option<String>)> = Vec::new();
    let mut window_unmapped: Vec<u32> = Vec::new();
    let mut had_errors: Vec<String> = Vec::new();

    {
        let Some(xwm) = state.xwm.as_mut() else {
            return Ok(());
        };

        match xwm.poll_external_clipboard_owner() {
            Ok(Some(XwmEvent::ClipboardCleared)) => clipboard_cleared = true,
            Ok(Some(XwmEvent::ClipboardUpdated {
                owner,
                mime_type,
                data,
            })) => clipboard_updates.push((owner, mime_type, data)),
            Ok(Some(other)) => had_errors.push(format!(
                "unexpected XWM event returned from clipboard owner poll: {:?}",
                other
            )),
            Ok(None) => {}
            Err(e) => had_errors.push(format!("XWM clipboard owner poll error: {}", e)),
        }

        while let Some(event) = xwm.poll_event() {
            match xwm.handle_event(&event) {
                Ok(Some(XwmEvent::ClipboardRequest {
                    requestor,
                    selection,
                    target,
                    property,
                    time,
                })) => {
                    clipboard_requests.push((requestor, selection, target, property, time));
                }
                Ok(Some(XwmEvent::ClipboardUpdated {
                    owner,
                    mime_type,
                    data,
                })) => {
                    clipboard_updates.push((owner, mime_type, data));
                }
                Ok(Some(XwmEvent::ClipboardCleared)) => {
                    clipboard_cleared = true;
                }
                Ok(Some(XwmEvent::WindowMapped {
                    x11_window_id,
                    title,
                    class,
                })) => {
                    window_mapped.push((x11_window_id, title, class));
                }
                Ok(Some(XwmEvent::WindowUnmapped { x11_window_id })) => {
                    window_unmapped.push(x11_window_id);
                }
                Ok(None) => {}
                Err(e) => had_errors.push(format!("XWM event error: {}", e)),
            }
        }
    }

    for (requestor, selection, target, property, time) in clipboard_requests {
        let clipboard_data = state.clipboard_cache.as_deref();
        if let Some(xwm) = state.xwm.as_ref() {
            if let Err(e) = handle_clipboard_request(
                xwm,
                requestor,
                selection,
                target,
                property,
                time,
                clipboard_data,
            ) {
                warn!("Failed to serve X11 clipboard request: {}", e);
            }
        }
    }

    for (owner, mime_type, data) in clipboard_updates {
        info!(
            "📋 Ingested external X11 clipboard owner {} as {} ({} bytes)",
            owner,
            mime_type,
            data.len()
        );
        state.clipboard_cache = Some(data);
        set_data_device_selection::<State>(
            display_handle,
            &state.seat,
            vec![mime_type.to_string(), "text/plain".to_string()],
            (),
        );
    }

    if clipboard_cleared {
        info!("📋 External X11 clipboard owner cleared");
        state.clipboard_cache = None;
    }

    for (x11_window_id, title, class) in window_mapped {
        let window_id = state.create_window_from_x11(x11_window_id, title.clone(), class.clone());
        info!(
            "X11 window {} mapped as compositor window {} (title='{}', class={:?})",
            x11_window_id, window_id, title, class
        );
        state.needs_redraw = true;
    }

    for x11_window_id in window_unmapped {
        if let Some(window_id) = state.x11_window_map.remove(&x11_window_id) {
            state.workspace_manager.write().remove_window(window_id);
            state.window_manager.write().remove_window(window_id);
            state.effects_engine.write().remove_window(window_id);
            if let Some(ref renderer) = state.renderer {
                renderer.write().remove_window(window_id);
            }
            state.decoration_manager.write().remove_window(window_id);

            // foreign_toplevel_list_state is disabled for now.
            info!(
                "X11 window {} removed (compositor window {})",
                x11_window_id, window_id
            );
        } else {
            info!(
                "X11 window {} unmapped (not tracked in compositor)",
                x11_window_id
            );
        }
        state.needs_redraw = true;
    }

    for err_msg in had_errors {
        warn!("⚠️ {}", err_msg);
    }

    Ok(())
}

/// Serve the cached Wayland clipboard data (or a placeholder) to a
/// requesting X11 client.
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
        warn!("Failed to serve X11 clipboard request: {}", e);
    }
    Ok(())
}
