//! Clipboard bridging for the Smithay winit/GLES backend.
//!
//! Contains the clipboard-related state helpers (`drain_clipboard_updates`,
//! `set_clipboard_data`) and the Wayland-selection extraction workers. A
//! submodule of `backend` can read the private fields of `State` and
//! `AxiomSmithayBackendReal` (descendant modules see ancestor privates), so
//! no fields were made public for this move.
//!
//! `SelectionHandler::new_selection` (the Wayland→compositor direction) stays
//! in `mod.rs` because it is a trait method of `State` and the `delegate_*`
//! macros / trait impls must remain co-located there; it calls the workers
//! defined here.

use log::{debug, info, warn};
use smithay::input::pointer::GrabStartData;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::utils::{Point, Serial};
use smithay::wayland::selection::data_device::{
    set_data_device_selection, start_dnd, SourceMetadata,
};
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, OwnedFd};
use std::sync::mpsc;

use super::{AxiomSmithayBackendReal, ClipboardUpdate, State};

// Clipboard helpers (formerly the clipboard_bridge module). These are small,
// backend-focused utilities for extracting/serving Wayland selection payloads.
pub(super) fn create_clipboard_pipe() -> anyhow::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `pipe2` initializes both fds on success; the returned raw fds
    // are immediately wrapped in `OwnedFd` so ownership is tracked safely.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(anyhow::anyhow!(
            "pipe2 failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: `pipe2` succeeded, so both file descriptors are valid and owned
    // by this function until they are wrapped.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: same rationale as `read_fd` above.
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read_fd, write_fd))
}

pub(super) fn spawn_clipboard_read_worker(read_fd: OwnedFd, tx: mpsc::Sender<ClipboardUpdate>) {
    std::thread::spawn(move || {
        let mut file = std::fs::File::from(read_fd);
        let mut data = Vec::new();
        match file.read_to_end(&mut data) {
            Ok(_) => {
                if !data.is_empty() {
                    let _ = tx.send(data);
                }
            }
            Err(e) => {
                warn!("⚠️ Failed to read Wayland clipboard pipe: {}", e);
            }
        }
    });
}

pub(super) fn write_selection_bytes_to_fd(fd: OwnedFd, data: &[u8]) {
    let mut file = std::fs::File::from(fd);
    if let Err(e) = file.write_all(data) {
        warn!("⚠️ Failed to write compositor selection to pipe: {}", e);
    }
}

impl State {
    /// Fold asynchronously-read Wayland selection payloads into the cache.
    pub(super) fn drain_clipboard_updates(&mut self) {
        while let Ok(data) = self.clipboard_update_rx.try_recv() {
            debug!(
                "📋 Clipboard cache refreshed from Wayland selection ({} bytes)",
                data.len()
            );
            self.clipboard_cache = Some(data);
        }
    }
}

impl AxiomSmithayBackendReal {
    /// Populate the clipboard cache with data from an external source
    /// (e.g., Lazy UI IPC, compositor-managed text). When X11 apps request
    /// clipboard contents, this data is served back to them.
    pub fn set_clipboard_data(&mut self, data: Vec<u8>) {
        debug!("📋 Clipboard cache populated ({} bytes)", data.len());
        self.state.clipboard_cache = Some(data);

        // Expose the same compositor-owned clipboard payload to Wayland
        // clients through smithay's server-side data-device selection path.
        set_data_device_selection::<State>(
            &self.display.handle(),
            &self.state.seat,
            vec![
                "text/plain;charset=utf-8".to_string(),
                "text/plain".to_string(),
            ],
            (),
        );
    }

    /// Start a server-initiated drag-and-drop session with the given payload.
    ///
    /// Populates the clipboard cache (so `ServerDndGrabHandler::send` serves the
    /// data) and initiates a real DnD grab on the seat's pointer if one exists.
    /// In headless/test environments without a pointer, the data is still cached
    /// and can be served via `ServerDndGrabHandler::send` directly.
    pub fn start_server_dnd(&mut self, data: Vec<u8>, mime_type: String) {
        info!(
            "📱 Server-initiated DnD: {} bytes via {}",
            data.len(),
            mime_type
        );
        self.state.clipboard_cache = Some(data.clone());

        // Try to start a real DnD grab if a pointer is available.
        // Extract what we need before the mutable borrow.
        let seat = self.state.seat.clone();
        let dh = self.display.handle();

        let metadata = SourceMetadata {
            mime_types: vec![mime_type.clone()],
            dnd_action: DndAction::Copy,
        };

        if let Some(pointer) = seat.get_pointer() {
            let grab_start_data = pointer
                .grab_start_data()
                .unwrap_or(GrabStartData {
                    focus: None,
                    button: 0,
                    location: Point::from((0.0, 0.0)),
                });
            let serial = Serial::from(0);
            start_dnd(
                &dh,
                &seat,
                &mut self.state,
                serial,
                Some(grab_start_data),
                None,
                metadata,
            );
            info!("📱 DnD grab started on pointer");
        } else {
            info!("📱 No pointer available — DnD data cached, grab deferred");
        }
    }
}
