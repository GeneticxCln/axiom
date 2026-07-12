//! Clipboard bridge helpers extracted from `src/backend/mod.rs`.
//!
//! These helpers are intentionally small and backend-focused:
//! - creating the pipe used for Wayland selection extraction
//! - reading clipboard bytes on a worker thread
//! - writing compositor-owned bytes back to a selection fd

use anyhow::Result;
use log::warn;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, OwnedFd};
use std::sync::mpsc;

pub(super) fn create_pipe() -> Result<(OwnedFd, OwnedFd)> {
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

pub(super) fn spawn_clipboard_read_worker(read_fd: OwnedFd, tx: mpsc::Sender<Vec<u8>>) {
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
