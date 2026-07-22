//! Screencopy protocol (zwlr_screencopy_manager_v1) implementation.
//!
//! Contains the `GlobalDispatch` and `Dispatch` implementations for
//! `ZwlrScreencopyManagerV1` and `ZwlrScreencopyFrameV1` on `State`.
//! A submodule of `backend` can read the private fields of `State`
//! (descendant modules see ancestor privates).

use log::warn;

use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1;
use smithay::reexports::wayland_server::{DataInit, Dispatch, GlobalDispatch, New};
use smithay::utils::Size;
use wayland_server::protocol::wl_shm::Format;
use wayland_server::{Client, Resource};
use zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use super::state::{PendingCapture, State};

// ── Screencopy protocol (zwlr_screencopy_manager_v1, V1 SHM-only) ──

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn bind(
        _state: &mut State,
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        _client: &Client,
        _resource: New<ZwlrScreencopyManagerV1>,
        _data: &(),
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        request: <ZwlrScreencopyManagerV1 as Resource>::Request,
        _data: &(),
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor: _,
                output: _,
            } => {
                let w = state.window_width;
                let h = state.window_height;

                if w == 0 || h == 0 {
                    warn!("Screencopy: output has zero area, refusing capture");
                    return;
                }

                let frame = data_init.init(frame, ());
                let stride = w * 4;
                frame.buffer(Format::Argb8888, w, h, stride);
                frame.buffer_done();
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { .. } => {
                warn!("Screencopy: capture_output_region not supported in V1");
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, (), State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        resource: &ZwlrScreencopyFrameV1,
        request: <ZwlrScreencopyFrameV1 as Resource>::Request,
        _data: &(),
        _dh: &smithay::reexports::wayland_server::DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                if state.pending_capture.is_some() {
                    warn!("Screencopy: already have a pending capture, ignoring duplicate");
                    return;
                }
                let w = state.window_width;
                let h = state.window_height;
                if w == 0 || h == 0 {
                    warn!("Screencopy: cannot capture, output has zero area");
                    return;
                }
                state.pending_capture = Some(PendingCapture {
                    frame: resource.clone(),
                    buffer: buffer.clone(),
                    size: Size::from((w as i32, h as i32)),
                });
                state.needs_redraw = true;
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {
                if let Some(ref pc) = state.pending_capture {
                    if pc.frame.id() == resource.id() {
                        state.pending_capture = None;
                    }
                }
            }
            _ => {}
        }
    }
}
