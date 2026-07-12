//! DRM/KMS Backend for Axiom Compositor
//!
//! This module provides the production DRM/KMS session-compositor backend.
//! It implements KMS modesetting via the `drm` crate with GBM-allocated
//! scanout buffers. EGL + GlesRenderer integration for GPU rendering is
//! deferred to a follow-up PR.
//!
//! ## Architecture
//!
//! ```text
//! DrmBackend
//!   ├── Card          — DRM device FD wrapper (implements drm::Device + control::Device)
//!   ├── KmsState      — Connector/CRTC/encoder enumeration, mode setting, page-flip
//!   ├── GbmRenderState — GBM device + surface for GPU-accelerated scanout buffers
//!   └── DrmSession    — libinput + calloop event loop
//! ```

use calloop::generic::Generic;
use calloop::{EventLoop, Interest, LoopHandle, Mode as CalloopMode, PostAction};
use drm::control::{
    connector, crtc, encoder, framebuffer, Device as ControlDevice, Event, Mode, ResourceHandles,
};
use drm::Device;
use gbm::{BufferObjectFlags, Device as GbmDevice, Format as GbmFormat};
use input::{Libinput, LibinputInterface};
use log::{debug, info, warn};
use std::fs::File;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;
use udev::MonitorSocket;

// ============================================================================
// Card — DRM device node wrapper
// ============================================================================

/// Simple wrapper around a DRM device node file descriptor.
/// Implements both [`drm::Device`] and [`drm::control::Device`] with all
/// default ioctl implementations derived from the [`AsFd`] trait.
pub struct Card {
    fd: OwnedFd,
}

impl Card {
    /// Open a DRM card device node (e.g. `/dev/dri/card0`).
    pub fn open(path: &str) -> Result<Self, anyhow::Error> {
        let file = File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open DRM device {}: {}", path, e))?;
        let fd = OwnedFd::from(file);
        info!("Opened DRM device: {}", path);
        Ok(Card { fd })
    }

    /// Return the raw file descriptor number.
    pub fn raw_fd(&self) -> std::os::unix::io::RawFd {
        self.fd.as_raw_fd()
    }
}

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl std::io::Read for Card {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cannot read from DRM card device",
        ))
    }
}

impl std::io::Write for Card {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cannot write to DRM card device",
        ))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

// ============================================================================
// BackendKind — backend selection enum
// ============================================================================

/// Backend kind selection for the Axiom compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// Nested-session Winit backend (default, development-friendly).
    Winit,
    /// Real DRM/KMS session-compositor (production).
    Drm,
    /// Headless no-op backend (tests / CI).
    Noop,
}

impl BackendKind {
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "winit" | "windowed" | "dev" => BackendKind::Winit,
            "drm" | "kms" | "session" | "tty" => BackendKind::Drm,
            "noop" | "test" | "headless" => BackendKind::Noop,
            unknown => {
                warn!(
                    "Unknown backend kind '{}' — falling back to 'winit'. \
                     Valid values: winit, drm, noop (and aliases)",
                    unknown
                );
                BackendKind::Winit
            }
        }
    }
}

// ============================================================================
// DRM device probe
// ============================================================================

/// Probe whether DRM/KMS devices are available on this system.
pub fn probe_drm_available() -> bool {
    let candidates = &["/dev/dri/card0", "/dev/dri/card1", "/dev/dri/card2"];
    let found: Vec<&str> = candidates
        .iter()
        .filter(|path| Path::new(path).exists())
        .copied()
        .collect();

    if found.is_empty() {
        debug!("No DRM device nodes found — DRM backend unavailable");
        false
    } else {
        debug!("DRM device(s) detected: {}", found.join(", "));
        true
    }
}

// ============================================================================
// GbmRenderState — GBM device + surface for GPU-accelerated scanout buffers
// ============================================================================

/// Holds the GBM device and scanout surface that replace the old dumb-buffer
/// path. The surface is created with `SCANOUT | RENDERING` flags so buffers
/// are suitable for both GPU rendering (future: EGL/GlesRenderer) and KMS
/// scanout.
pub struct GbmRenderState {
    /// GBM device opened on a dup'd DRM card file descriptor.
    pub device: GbmDevice<OwnedFd>,
    /// GBM surface allocating scanout-capable buffers.
    pub surface: gbm::Surface<OwnedFd>,
}

impl GbmRenderState {
    /// Create a new GBM device + surface from a dup'd DRM card FD.
    ///
    /// The `card_fd` is duplicated internally so the original [`Card`]
    /// retains ownership of its FD. The surface is sized to `(width,
    /// height)` with XRGB8888 format for display scanout.
    pub fn new(raw_card_fd: RawFd, width: u32, height: u32) -> Result<Self, anyhow::Error> {
        let duped = unsafe { libc::dup(raw_card_fd) };
        if duped < 0 {
            return Err(anyhow::anyhow!(
                "Failed to dup DRM fd for GBM device: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: `duped` is a valid, newly-allocated FD from dup().
        let gbm_fd = unsafe { OwnedFd::from_raw_fd(duped) };

        let device = GbmDevice::new(gbm_fd)
            .map_err(|e| anyhow::anyhow!("Failed to create GBM device: {}", e))?;

        info!("GBM device created (backend: {:?})", device.backend_name());

        let surface = device
            .create_surface(
                width,
                height,
                GbmFormat::Xrgb8888,
                BufferObjectFlags::SCANOUT | BufferObjectFlags::RENDERING,
            )
            .map_err(|e| anyhow::anyhow!("Failed to create GBM surface: {}", e))?;

        info!("GBM surface created ({}x{} XRGB8888)", width, height);

        Ok(Self { device, surface })
    }

    /// Lock the front buffer of the GBM surface and create a DRM
    /// framebuffer from it. Returns `(fb_handle, buffer_object)`.
    ///
    /// The caller **must** keep the returned [`gbm::BufferObject`] alive
    /// for as long as the DRM framebuffer is in use (displayed on screen),
    /// because dropping it may release the underlying GBM buffer object.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `eglSwapBuffers`-equivalent rendering
    /// has been completed before calling this function.
    pub unsafe fn lock_and_create_fb(
        &self,
        card: &Card,
    ) -> Result<(framebuffer::Handle, gbm::BufferObject<OwnedFd>), anyhow::Error> {
        let bo = self
            .surface
            .lock_front_buffer()
            .map_err(|e| anyhow::anyhow!("Failed to lock GBM front buffer: {}", e))?;

        let fb = card
            .add_framebuffer(&bo, 24, 32)
            .map_err(|e| anyhow::anyhow!("Failed to create DRM framebuffer from GBM BO: {}", e))?;

        debug!(
            "GBM front buffer locked ({}x{}, stride={}) → DRM FB {:?}",
            bo.width(),
            bo.height(),
            bo.stride(),
            fb,
        );

        Ok((fb, bo))
    }
}

impl std::fmt::Debug for GbmRenderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GbmRenderState")
            .field("backend", &self.device.backend_name())
            .finish()
    }
}

// ============================================================================
// KmsOutput — per-display KMS state
// ============================================================================

/// Holds the full KMS modesetting state for a single display output.
/// One [`KmsState`] can hold multiple [`KmsOutput`]s (one per connected
/// connector), each with its own CRTC, encoder, mode, GBM surface, and
/// DPI scale factor derived from EDID physical dimensions.
pub struct KmsOutput {
    pub connector: connector::Handle,
    pub crtc: crtc::Handle,
    pub encoder: encoder::Handle,
    pub mode: Mode,
    pub width: u32,
    pub height: u32,
    /// EDID width in millimetres (for DPI calculation).
    pub physical_width_mm: u32,
    /// EDID height in millimetres (for DPI calculation).
    pub physical_height_mm: u32,
    /// Output scale factor derived from physical size vs mode resolution.
    /// 1.0 = 96 DPI reference; 2.0 = HiDPI (192 DPI).
    pub scale_factor: f64,
    /// Human-readable connector name (e.g. "HDMI-A-1").
    pub name: String,
    /// GBM device + surface for future GPU-accelerated scanout.
    pub gbm: Option<GbmRenderState>,
    /// CPU-writable dumb-buffer scanout used by the current standalone alpha path.
    cpu_scanout: Option<CpuScanoutBuffer>,
    /// Currently displayed DRM framebuffer handle.
    current_fb: framebuffer::Handle,
    /// Saved CRTC state for restoration on shutdown.
    saved_crtc: Option<crtc::Info>,
}

impl std::fmt::Debug for KmsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KmsOutput")
            .field("name", &self.name)
            .field("connector", &u32::from(self.connector))
            .field("crtc", &u32::from(self.crtc))
            .field("encoder", &u32::from(self.encoder))
            .field("width", &self.width)
            .field("height", &self.height)
            .field(
                "physical_mm",
                &(self.physical_width_mm, self.physical_height_mm),
            )
            .field("scale_factor", &self.scale_factor)
            .field("gbm", &self.gbm.is_some())
            .field("cpu_scanout", &self.cpu_scanout.is_some())
            .finish()
    }
}

/// CPU-writable dumb-buffer scanout backing the current standalone alpha path.
struct CpuScanoutBuffer {
    fb: framebuffer::Handle,
    buffer: drm::control::dumbbuffer::DumbBuffer,
}

/// Compute a DPI scale factor from EDID physical dimensions and mode
/// resolution. Uses a 96 DPI reference baseline:
///
/// ```text
/// diagonal_px = sqrt(width² + height²)
/// diagonal_mm = sqrt(physical_w² + physical_h²)
/// dpi = diagonal_px / (diagonal_mm / 25.4)
/// scale = (dpi / 96.0).round()
/// ```
///
/// Falls back to 1.0 when physical dimensions are zero or NaN.
fn scale_factor_from_edid(
    mode_width: u32,
    mode_height: u32,
    physical_w_mm: u32,
    physical_h_mm: u32,
) -> f64 {
    if physical_w_mm == 0 || physical_h_mm == 0 {
        return 1.0;
    }
    let w = mode_width as f64;
    let h = mode_height as f64;
    let pw = physical_w_mm as f64;
    let ph = physical_h_mm as f64;
    let diagonal_px = (w * w + h * h).sqrt();
    let diagonal_mm = (pw * pw + ph * ph).sqrt();
    if diagonal_mm < 1.0 {
        return 1.0;
    }
    let dpi = diagonal_px / (diagonal_mm / 25.4);
    // Quantize to 0.25 steps so the output can advertise a stable fractional
    // scale (e.g. 1.25x, 1.5x, 1.75x) instead of snapping everything to a whole
    // integer. Cap at 4.0 to avoid absurd EDID-derived values.
    let raw_scale = dpi / 96.0;
    let scale = ((raw_scale * 4.0).round() / 4.0).clamp(1.0, 4.0);
    info!(
        "DPI calc: {}x{} px, {}x{} mm → {:.1} DPI → scale {:.2}x",
        mode_width, mode_height, physical_w_mm, physical_h_mm, dpi, scale
    );
    scale
}

/// Copy a BGRA8 compositor frame into an XRGB8888 dumb buffer.
///
/// The compositor currently reads back a BGRA8 image from the WGPU path.
/// Dumb buffers created for scanout use XRGB8888, whose little-endian memory
/// layout is effectively B, G, R, X. This helper copies the overlapping region
/// and clears any uncovered destination pixels to black.
fn copy_bgra_to_xrgb8888(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    src_origin_x: u32,
    src_origin_y: u32,
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    dst_pitch: u32,
) {
    if src_origin_x >= src_width || src_origin_y >= src_height {
        dst.fill(0);
        return;
    }

    let copy_w = (src_width - src_origin_x).min(dst_width) as usize;
    let copy_h = (src_height - src_origin_y).min(dst_height) as usize;
    let src_pitch = src_width as usize * 4;
    let dst_pitch = dst_pitch as usize;

    dst.fill(0);

    for y in 0..copy_h {
        let src_y = src_origin_y as usize + y;
        let src_row = &src[src_y * src_pitch..(src_y + 1) * src_pitch];
        let dst_row = &mut dst[y * dst_pitch..(y + 1) * dst_pitch];
        for x in 0..copy_w {
            let src_x = src_origin_x as usize + x;
            let si = src_x * 4;
            let di = x * 4;
            dst_row[di] = src_row[si];
            dst_row[di + 1] = src_row[si + 1];
            dst_row[di + 2] = src_row[si + 2];
            dst_row[di + 3] = 0xFF;
        }
    }
}

// ============================================================================
// KMS State — multi-output modesetting (GBM-backed)
// ============================================================================

/// Holds the full KMS modesetting state for ALL connected display outputs.
/// Each output gets its own [`KmsOutput`] with per-connector CRTC/encoder,
/// GBM-backed scanout buffers, and DPI scale factor. The shared [`Card`]
/// (DRM device FD) is owned here and used for all ioctls.
pub struct KmsState {
    pub card: Card,
    pub outputs: Vec<KmsOutput>,
}

impl KmsState {
    /// Open a DRM device, find ALL connected connectors with compatible
    /// CRTCs, and set up GBM-accelerated modesetting for each.
    pub fn open(path: &str) -> Result<Self, anyhow::Error> {
        let card = Card::open(path)?;

        if let Err(e) = card.acquire_master_lock() {
            warn!("Could not acquire DRM master (may already hold it): {}", e);
        }

        let resources: ResourceHandles = card.resource_handles()?;
        debug!(
            "DRM resources — connectors: {:?}, crtcs: {:?}, encoders: {:?}",
            resources.connectors(),
            resources.crtcs(),
            resources.encoders(),
        );

        let candidates = Self::find_all_connected_connectors(&card, &resources)?;
        if candidates.is_empty() {
            return Err(anyhow::anyhow!(
                "No connected display connector with a compatible encoder/CRTC found"
            ));
        }

        info!("Found {} connected display(s)", candidates.len());

        // Track which CRTCs are already in use so two outputs never
        // claim the same CRTC.
        let mut used_crtcs: std::collections::HashSet<crtc::Handle> =
            std::collections::HashSet::new();
        let mut outputs: Vec<KmsOutput> = Vec::with_capacity(candidates.len());

        for (connector, encoder, crtc, mode, physical_w_mm, physical_h_mm, conn_name) in candidates
        {
            if used_crtcs.contains(&crtc) {
                warn!(
                    "Connector {} wants CRTC {:?} which is already claimed — skipping",
                    conn_name,
                    u32::from(crtc)
                );
                continue;
            }
            used_crtcs.insert(crtc);

            let (width, height) = mode.size();
            let width = width as u32;
            let height = height as u32;
            info!(
                "Display '{}': {}x{} @ {} Hz, {}x{}mm",
                conn_name,
                width,
                height,
                mode.vrefresh() as f32 / 1000.0,
                physical_w_mm,
                physical_h_mm,
            );

            let scale_factor = scale_factor_from_edid(width, height, physical_w_mm, physical_h_mm);
            let saved_crtc = Some(card.get_crtc(crtc)?);

            // Skip GBM in test mode to avoid SIGSEGV on CI/VMs.
            let gbm = if cfg!(test) {
                debug!("Skipping GBM init in test mode — using dumb-buffer fallback");
                None
            } else {
                let raw_fd = card.raw_fd();
                GbmRenderState::new(raw_fd, width, height)
                    .map_err(|e| {
                        warn!(
                            "GBM init failed for '{}' ({}); falling back to dumb buffer",
                            conn_name, e
                        );
                        e
                    })
                    .ok()
            };

            // Allocate a CPU-writable scanout buffer for the current standalone
            // alpha path. GBM is kept around for future GPU-direct presentation,
            // but the compositor output currently lands in a dumb buffer.
            let cpu_scanout = match Self::create_cpu_scanout_buffer(&card, width, height) {
                Ok(scanout) => Some(scanout),
                Err(e) => {
                    warn!(
                        "Failed to create CPU scanout for '{}' ({}); falling back to legacy path",
                        conn_name, e
                    );
                    None
                }
            };

            if let Some(scanout_fb) = cpu_scanout.as_ref().map(|scanout| scanout.fb) {
                card.set_crtc(crtc, Some(scanout_fb), (0, 0), &[connector], Some(mode))?;

                outputs.push(KmsOutput {
                    connector,
                    crtc,
                    encoder,
                    mode,
                    width,
                    height,
                    physical_width_mm: physical_w_mm,
                    physical_height_mm: physical_h_mm,
                    scale_factor,
                    name: conn_name,
                    gbm,
                    cpu_scanout,
                    current_fb: scanout_fb,
                    saved_crtc,
                });
            } else if let Some(ref gbm_state) = gbm {
                // SAFETY: Initial modeset — no prior EGL rendering.
                let (fb, front) = unsafe { gbm_state.lock_and_create_fb(&card)? };
                card.set_crtc(crtc, Some(fb), (0, 0), &[connector], Some(mode))?;
                drop(front);

                outputs.push(KmsOutput {
                    connector,
                    crtc,
                    encoder,
                    mode,
                    width,
                    height,
                    physical_width_mm: physical_w_mm,
                    physical_height_mm: physical_h_mm,
                    scale_factor,
                    name: conn_name,
                    gbm,
                    cpu_scanout: None,
                    current_fb: fb,
                    saved_crtc,
                });
            } else {
                let (fb, _dumb) = Self::create_dumb_framebuffer(&card, width, height)?;
                card.set_crtc(crtc, Some(fb), (0, 0), &[connector], Some(mode))?;

                outputs.push(KmsOutput {
                    connector,
                    crtc,
                    encoder,
                    mode,
                    width,
                    height,
                    physical_width_mm: physical_w_mm,
                    physical_height_mm: physical_h_mm,
                    scale_factor,
                    name: conn_name,
                    gbm: None,
                    cpu_scanout: None,
                    current_fb: fb,
                    saved_crtc,
                });
            }

            info!(
                "Output '{}' initialized: {}x{} @ {:.1}x scale",
                outputs.last().unwrap().name,
                width,
                height,
                scale_factor,
            );
        }

        Ok(KmsState { card, outputs })
    }

    /// Find ALL connected connectors with compatible encoder + CRTC.
    /// Returns tuples of (connector, encoder, crtc, mode, physical_w_mm,
    /// physical_h_mm, connector_name). CRTC allocation is handled by the
    /// caller so duplicates can be detected.
    fn find_all_connected_connectors(
        card: &Card,
        resources: &ResourceHandles,
    ) -> Result<Vec<ConnectorInfo>, anyhow::Error> {
        let mut results = Vec::new();

        for &conn in resources.connectors() {
            let conn_info = card
                .get_connector(conn, true)
                .map_err(|e| anyhow::anyhow!("Failed to get connector info: {}", e))?;

            if conn_info.state() != connector::State::Connected {
                debug!("Connector {:?} not connected, skipping", conn);
                continue;
            }

            let modes = conn_info.modes();
            if modes.is_empty() {
                warn!("Connector {:?} connected but has no modes — skipping", conn);
                continue;
            }

            let mode = modes[0];
            let (w, h) = mode.size();

            // Read EDID physical dimensions from connector info.
            let size_mm = conn_info.size().unwrap_or((0, 0));
            let physical_w_mm = size_mm.0;
            let physical_h_mm = size_mm.1;

            // Build a human-readable connector name.
            let conn_name = format!("{:?}-{}", conn_info.interface(), conn_info.interface_id());
            debug!(
                "Connector '{}' connected, mode: {}x{}, physical: {}x{}mm",
                conn_name, w, h, physical_w_mm, physical_h_mm
            );

            for &enc in conn_info.encoders() {
                let enc_info = card
                    .get_encoder(enc)
                    .map_err(|e| anyhow::anyhow!("Failed to get encoder info: {}", e))?;

                let compatible = resources.filter_crtcs(enc_info.possible_crtcs());
                if let Some(&crtc_h) = compatible.first() {
                    info!(
                        "KMS config: {conn_name} {:?} → encoder {:?} → CRTC {:?} @ {}x{}",
                        conn, enc, crtc_h, w, h
                    );
                    results.push((
                        conn,
                        enc,
                        crtc_h,
                        mode,
                        physical_w_mm,
                        physical_h_mm,
                        conn_name.clone(),
                    ));
                    // Only take the first compatible encoder for this
                    // connector — we move on to the next connector.
                    break;
                }
            }

            if results.iter().all(|(c, ..)| *c != conn) {
                warn!(
                    "Connector {:?} connected but no compatible encoder + CRTC found",
                    conn
                );
            }
        }

        Ok(results)
    }

    /// Render frames to ALL outputs. Iterates every [`KmsOutput`], locks
    /// the GBM front buffer, creates a new framebuffer, and performs a
    /// synchronous CRTC mode set. Dumb-buffer outputs are skipped (no-op).
    ///
    /// Returns the count of outputs that were actually rendered (GBM path).
    pub fn render_all_frames(&mut self) -> usize {
        let mut rendered = 0usize;
        for output in &mut self.outputs {
            let gbm = match output.gbm.as_ref() {
                Some(g) => g,
                None => continue,
            };

            // SAFETY: Synchronous set_crtc ensures the previous frame's
            // buffer is no longer being scanned out before we lock the
            // next one. The kernel holds a GEM reference across frames.
            let (new_fb, _front) = match unsafe { gbm.lock_and_create_fb(&self.card) } {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("Failed to lock GBM buffer for '{}': {}", output.name, e);
                    continue;
                }
            };

            let old_fb = output.current_fb;
            if let Err(e) = self.card.set_crtc(
                output.crtc,
                Some(new_fb),
                (0, 0),
                &[output.connector],
                Some(output.mode),
            ) {
                warn!("set_crtc failed for '{}': {}", output.name, e);
                continue;
            }

            let _ = self.card.destroy_framebuffer(old_fb);
            output.current_fb = new_fb;
            rendered += 1;
        }
        if rendered > 0 {
            debug!("Rendered {} output(s)", rendered);
        }
        rendered
    }

    /// Drain pending DRM events (page-flip, vblank) from the card.
    pub fn receive_events(&mut self) -> Result<Vec<Event>, anyhow::Error> {
        let fd = self.card.raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags >= 0 {
            unsafe {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        let mut events = Vec::new();
        match self.card.receive_events() {
            Ok(ev_iter) => {
                for ev in ev_iter {
                    match &ev {
                        Event::PageFlip(flip) => {
                            debug!("Page flip complete (frame: {})", flip.frame);
                        }
                        Event::Vblank(vb) => {
                            debug!("Vblank (frame: {})", vb.frame);
                        }
                        _ => {}
                    }
                    events.push(ev);
                }
            }
            Err(e) => {
                if let Some(libc_err) = e.raw_os_error() {
                    if libc_err == libc::EAGAIN || libc_err == libc::EWOULDBLOCK {
                        return Ok(events);
                    }
                }
                warn!("Error receiving DRM events: {}", e);
            }
        }
        Ok(events)
    }

    /// Restore original CRTC states and free all framebuffers.
    pub fn cleanup(&mut self) -> Result<(), anyhow::Error> {
        info!("Cleaning up KMS state ({} outputs)", self.outputs.len());

        for output in &mut self.outputs {
            if let Some(ref saved) = output.saved_crtc {
                if let Some(fb_handle) = saved.framebuffer() {
                    let (saved_x, saved_y) = saved.position();
                    let _ = self.card.set_crtc(
                        output.crtc,
                        Some(fb_handle),
                        (saved_x, saved_y),
                        &[output.connector],
                        saved.mode(),
                    );
                }
            }
            if let Some(scanout) = output.cpu_scanout.take() {
                if output.current_fb != scanout.fb {
                    let _ = self.card.destroy_framebuffer(output.current_fb);
                }
                let _ = self.card.destroy_framebuffer(scanout.fb);
                let _ = self.card.destroy_dumb_buffer(scanout.buffer);
            } else {
                let _ = self.card.destroy_framebuffer(output.current_fb);
            }
            drop(output.gbm.take());
        }

        self.outputs.clear();
        let _ = self.card.release_master_lock();
        info!("KMS state cleaned up");
        Ok(())
    }

    /// Create a persistent CPU-writable scanout buffer for compositor output.
    fn create_cpu_scanout_buffer(
        card: &Card,
        width: u32,
        height: u32,
    ) -> Result<CpuScanoutBuffer, anyhow::Error> {
        let (fb, dumb) = Self::create_dumb_framebuffer(card, width, height)?;
        Ok(CpuScanoutBuffer { fb, buffer: dumb })
    }

    /// Present a composed BGRA frame to every output through the CPU dumb-buffer
    /// scanout path. Returns the number of outputs updated.
    pub fn present_composited_frame(
        &mut self,
        src_width: u32,
        src_height: u32,
        bgra: &[u8],
    ) -> Result<usize, anyhow::Error> {
        let mut presented = 0usize;
        let mut output_origin_x = 0u32;
        for output in &mut self.outputs {
            let Some(scanout) = output.cpu_scanout.as_mut() else {
                output_origin_x = output_origin_x.saturating_add(output.width);
                continue;
            };
            let mut mapping = self
                .card
                .map_dumb_buffer(&mut scanout.buffer)
                .map_err(|e| anyhow::anyhow!("Failed to map dumb buffer for '{}': {}", output.name, e))?;
            copy_bgra_to_xrgb8888(
                bgra,
                src_width,
                src_height,
                output_origin_x,
                0,
                &mut mapping,
                output.width,
                output.height,
                drm::buffer::Buffer::pitch(&scanout.buffer),
            );
            output_origin_x = output_origin_x.saturating_add(output.width);
            presented += 1;
        }
        if presented > 0 {
            debug!(
                "Presented composed frame {}x{} to {} CPU scanout output(s)",
                src_width, src_height, presented
            );
        }
        Ok(presented)
    }

    /// Fallback: create a dumb buffer + framebuffer (used when GBM is
    /// unavailable).
    fn create_dumb_framebuffer(
        card: &Card,
        width: u32,
        height: u32,
    ) -> Result<(framebuffer::Handle, drm::control::dumbbuffer::DumbBuffer), anyhow::Error> {
        use drm::buffer::Buffer as _;
        use drm_fourcc::DrmFourcc;

        let bpp = 32u32;
        let dumb = card
            .create_dumb_buffer((width, height), DrmFourcc::Xrgb8888, bpp)
            .map_err(|e| anyhow::anyhow!("Failed to create dumb buffer: {}", e))?;

        let fb = card
            .add_framebuffer(&dumb, 24, bpp)
            .map_err(|e| anyhow::anyhow!("Failed to add framebuffer: {}", e))?;

        debug!(
            "Created dumb framebuffer fallback ({}x{}, pitch {})",
            width,
            height,
            dumb.pitch(),
        );

        Ok((fb, dumb))
    }
}

impl std::fmt::Debug for KmsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KmsState")
            .field("outputs", &self.outputs)
            .finish()
    }
}

// ============================================================================
// LibinputDevice — libinput interface for opening/closing input device nodes
// ============================================================================

/// Implements [`LibinputInterface`] using direct `open()` calls.
/// In a production compositor this would go through libseat/logind,
/// but for now we open `/dev/input/event*` directly (requires root or
/// appropriate capabilities).
struct LibinputDevice;

impl LibinputInterface for LibinputDevice {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        std::fs::OpenOptions::new()
            .custom_flags(flags)
            .open(path)
            .map(|f| f.into())
            .map_err(|e| e.raw_os_error().unwrap_or(libc::EACCES))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

// ============================================================================
// CalloopFd — owned FD wrapper for calloop Generic event sources
// ============================================================================

/// An owned file descriptor suitable for registration with a calloop
/// [`Generic`] event source. Created via [`dup`](libc::dup) so the
/// original FD held by [`Card`] or the libinput context remains valid.
struct CalloopFd(OwnedFd);

impl CalloopFd {
    fn dup(raw: RawFd) -> std::io::Result<Self> {
        let duped = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
        if duped < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: `duped` is a valid, newly-allocated FD from fcntl.
            Ok(CalloopFd(unsafe { OwnedFd::from_raw_fd(duped) }))
        }
    }
}

impl AsFd for CalloopFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

// ============================================================================
// DrmEventCollector — shared data passed to calloop callbacks
// ============================================================================

/// Collects readiness flags from calloop FD callbacks. After
/// [`EventLoop::dispatch`] returns, the compositor checks these flags
/// to know which FDs have data pending.
#[derive(Default)]
pub struct DrmEventCollector {
    /// Set when the DRM FD has pending page-flip / vblank events.
    pub drm_ready: bool,
    /// Set when the libinput FD has pending input events.
    pub libinput_ready: bool,
    /// Set when the udev monitor FD has pending hotplug events.
    pub udev_ready: bool,
}

/// Connector info returned by [`KmsState::find_all_connected_connectors`].
/// Fields: connector, encoder, crtc, mode, physical_w_mm, physical_h_mm, connector_name.
type ConnectorInfo = (
    connector::Handle,
    encoder::Handle,
    crtc::Handle,
    Mode,
    u32,
    u32,
    String,
);

// ======================================================================
// KmsState — Connector/CRTC/encoder enumeration, mode setting, page-flip
// ======================================================================

/// Top-level DRM backend state for the Axiom compositor.
pub struct DrmBackend {
    pub available: bool,
    pub primary_device: Option<String>,
    pub kms: Option<KmsState>,
    /// libinput context for input device discovery via udev.
    /// `None` when DRM backend was not initialized or when no
    /// `/dev/dri/card*` device was found.
    pub libinput: Option<Libinput>,
    /// Calloop event loop for polling DRM and libinput FDs.
    /// Created during [`initialize`].
    pub calloop_loop: Option<EventLoop<'static, DrmEventCollector>>,
    /// Handle to the calloop loop for registering / removing sources.
    pub calloop_handle: Option<LoopHandle<'static, DrmEventCollector>>,
    /// udev monitor for the "drm" subsystem. Detects connector
    /// hotplug events (monitor plugged/unplugged). Its FD is registered
    /// with calloop for async notification.
    pub udev_monitor: Option<MonitorSocket>,
}
impl std::fmt::Debug for DrmBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrmBackend")
            .field("available", &self.available)
            .field("primary_device", &self.primary_device)
            .field("kms", &self.kms)
            .field("libinput", &self.libinput.as_ref().map(|_| "<Libinput>"))
            .field("calloop_loop", &self.calloop_loop.is_some())
            .field("udev_monitor", &self.udev_monitor.is_some())
            .finish()
    }
}

impl DrmBackend {
    pub fn new() -> Self {
        let available = probe_drm_available();
        let primary_device = if available {
            ["/dev/dri/card0", "/dev/dri/card1", "/dev/dri/card2"]
                .iter()
                .find(|path| Path::new(path).exists())
                .map(|s| s.to_string())
        } else {
            None
        };

        if available {
            info!(
                "DRM backend probed — primary device: {}",
                primary_device.as_deref().unwrap_or("none")
            );
        } else {
            warn!("No DRM devices found — DRM backend will not function");
        }

        Self {
            available,
            primary_device,
            kms: None,
            libinput: None,
            calloop_loop: None,
            calloop_handle: None,
            udev_monitor: None,
        }
    }

    /// Initialize the DRM session: open the device, find all connected
    /// displays, set up modesetting for each, and create initial
    /// framebuffers. Returns the count of initialized outputs.
    pub fn initialize(&mut self) -> Result<usize, anyhow::Error> {
        if !self.available {
            warn!("DRM backend: no devices available, initialize is a no-op");
            return Ok(0);
        }

        let device_path = self.primary_device.as_deref().unwrap_or("/dev/dri/card0");
        info!("Initializing DRM/KMS backend on {}", device_path);

        let kms = KmsState::open(device_path)?;
        let output_count = kms.outputs.len();

        let gbm_count = kms.outputs.iter().filter(|o| o.gbm.is_some()).count();
        info!(
            "KMS initialized: {} output(s) ({} GBM-accelerated, {} dumb-buffer)",
            output_count,
            gbm_count,
            output_count - gbm_count,
        );

        self.kms = Some(kms);

        self.init_libinput();
        self.init_calloop_loop()?;

        info!("DRM/KMS backend fully initialized");
        Ok(output_count)
    }

    /// Present a composed BGRA frame to every connected output through the
    /// current CPU-writable dumb-buffer scanout path.
    pub fn present_composited_frame(
        &mut self,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Result<usize, anyhow::Error> {
        match self.kms.as_mut() {
            Some(kms) => kms.present_composited_frame(width, height, bgra),
            None => Ok(0),
        }
    }

    /// Legacy fallback path: render frames to all connected displays via GBM surface → DRM
    /// framebuffer → synchronous CRTC mode set. Returns the count of
    /// outputs that were actually rendered (GBM path). Dumb-buffer
    /// outputs are skipped (no-op).
    pub fn render_frame(&mut self) -> usize {
        match self.kms.as_mut() {
            Some(kms) => kms.render_all_frames(),
            None => {
                debug!("DRM render_frame called but KMS not initialized — no-op");
                0
            }
        }
    }

    /// Return the number of initialized KMS outputs.
    pub fn output_count(&self) -> usize {
        self.kms.as_ref().map(|k| k.outputs.len()).unwrap_or(0)
    }

    /// Iterate over KMS outputs for the caller (compositor).
    /// Returns an empty slice when KMS is not initialized.
    pub fn kms_outputs(&self) -> &[KmsOutput] {
        self.kms
            .as_ref()
            .map(|k| k.outputs.as_slice())
            .unwrap_or(&[])
    }

    /// Drain pending DRM events.
    pub fn receive_events(&mut self) -> Result<Vec<Event>, anyhow::Error> {
        match self.kms.as_mut() {
            Some(kms) => kms.receive_events(),
            None => Ok(Vec::new()),
        }
    }

    /// Shut down the DRM backend, restore CRTC, release resources.
    pub fn shutdown(&mut self) -> Result<(), anyhow::Error> {
        info!("Shutting down DRM backend");

        // Drop calloop sources first (they hold dup'd FDs).
        self.calloop_loop.take();
        self.calloop_handle.take();

        // Drop the udev monitor (releases the socket FD).
        self.udev_monitor.take();

        // Restore original CRTC state and free framebuffers.
        if let Some(mut kms) = self.kms.take() {
            kms.cleanup()?;
        }

        // Close the libinput context (drops all device FDs).
        self.libinput.take();

        self.available = false;
        self.primary_device = None;
        Ok(())
    }

    /// Block until a DRM event is available (up to `timeout`).
    pub fn poll_event(&self, timeout: Duration) -> Result<bool, anyhow::Error> {
        let kms = self
            .kms
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("KMS not initialized"))?;
        let fd = kms.card.raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: poll() is safe with a valid fd and initialized struct.
        let ret = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ret < 0 {
            Err(anyhow::anyhow!(
                "DRM poll failed: {}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(ret > 0)
        }
    }

    /// Create a libinput context with udev discovery for the given seat.
    ///
    /// This must be called after [`initialize`] so that a KMS session is
    /// active. The libinput FD can then be polled alongside the DRM FD
    /// in the compositor's event loop.
    pub fn init_libinput(&mut self) {
        if self.libinput.is_some() {
            debug!("libinput context already initialized");
            return;
        }

        info!("Initializing libinput context with udev seat discovery");

        let mut libinput = Libinput::new_with_udev(LibinputDevice);

        if libinput.udev_assign_seat("seat0").is_err() {
            warn!("libinput: udev_assign_seat failed — no input devices will be discovered");
            // Still store the context so dispatch() doesn't panic.
        } else {
            info!("libinput: seat 'seat0' assigned");
        }

        self.libinput = Some(libinput);
    }

    /// Create the calloop event loop and register DRM + libinput + udev FDs as
    /// [`Generic`] event sources. The callbacks set readiness flags on
    /// a [`DrmEventCollector`] which the compositor reads after dispatch.
    pub fn init_calloop_loop(&mut self) -> Result<(), anyhow::Error> {
        if self.calloop_loop.is_some() {
            debug!("calloop loop already initialized");
            return Ok(());
        }

        let event_loop: EventLoop<'static, DrmEventCollector> = EventLoop::try_new()?;
        let handle = event_loop.handle();

        // Register DRM FD
        if let Some(ref kms) = self.kms {
            let drm_fd = kms.card.raw_fd();
            let dup_fd = CalloopFd::dup(drm_fd)?;
            handle.insert_source(
                Generic::new(dup_fd, Interest::READ, CalloopMode::Level),
                |_, _, collector: &mut DrmEventCollector| {
                    collector.drm_ready = true;
                    Ok(PostAction::Continue)
                },
            )?;
            debug!("Registered DRM FD {} with calloop", drm_fd);
        }

        // Register libinput FD
        if let Some(ref li) = self.libinput {
            let li_fd = li.as_raw_fd();
            let dup_fd = CalloopFd::dup(li_fd)?;
            handle.insert_source(
                Generic::new(dup_fd, Interest::READ, CalloopMode::Level),
                |_, _, collector: &mut DrmEventCollector| {
                    collector.libinput_ready = true;
                    Ok(PostAction::Continue)
                },
            )?;
            debug!("Registered libinput FD {} with calloop", li_fd);
        }

        self.calloop_loop = Some(event_loop);
        self.calloop_handle = Some(handle);
        info!("Calloop event loop initialized");
        Ok(())
    }

    /// Dispatch the calloop loop once (non-blocking). Returns a
    /// [`DrmEventCollector`] indicating which FDs have data pending.
    pub fn dispatch_calloop(&mut self) -> Result<DrmEventCollector, anyhow::Error> {
        let Some(ref mut loop_) = self.calloop_loop else {
            return Ok(DrmEventCollector::default());
        };

        let mut collector = DrmEventCollector::default();
        loop_.dispatch(Duration::ZERO, &mut collector)?;
        Ok(collector)
    }

    /// Create a udev monitor for the "drm" subsystem and register its
    /// FD with the calloop event loop. The monitor detects connector
    /// hotplug events (monitor plugged/unplugged).
    ///
    /// Must be called after [`init_calloop_loop`] so the handle is
    /// available for FD registration.
    pub fn init_udev_monitor(&mut self) {
        if self.udev_monitor.is_some() {
            debug!("udev DRM monitor already initialized");
            return;
        }

        let monitor = match udev::MonitorBuilder::new() {
            Ok(builder) => match builder.match_subsystem("drm") {
                Ok(subsystem_builder) => match subsystem_builder.listen() {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("udev DRM monitor listen failed: {} — hotplug disabled", e);
                        return;
                    }
                },
                Err(e) => {
                    warn!("udev DRM subsystem match failed: {} — hotplug disabled", e);
                    return;
                }
            },
            Err(e) => {
                warn!(
                    "Failed to create udev monitor: {} — DRM hotplug unavailable",
                    e
                );
                return;
            }
        };

        // Register the monitor's FD with calloop.
        if let Some(ref handle) = self.calloop_handle {
            let udev_fd = monitor.as_raw_fd();
            unsafe {
                let flags = libc::fcntl(udev_fd, libc::F_GETFL);
                if flags >= 0 {
                    libc::fcntl(udev_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
            match CalloopFd::dup(udev_fd) {
                Ok(dup_fd) => {
                    if handle
                        .insert_source(
                            Generic::new(dup_fd, Interest::READ, CalloopMode::Level),
                            |_, _, collector: &mut DrmEventCollector| {
                                collector.udev_ready = true;
                                Ok(PostAction::Continue)
                            },
                        )
                        .is_ok()
                    {
                        info!("🔌 udev DRM hotplug monitor registered with calloop");
                    } else {
                        warn!("Failed to register udev monitor FD with calloop");
                        return;
                    }
                }
                Err(e) => {
                    warn!("Failed to dup udev monitor FD: {} — hotplug disabled", e);
                    return;
                }
            }
        } else {
            warn!("No calloop handle — cannot register udev monitor (init_calloop_loop must be called first)");
            return;
        }

        self.udev_monitor = Some(monitor);
    }

    /// Drain pending udev events from the monitor. Returns `true` if a
    /// DRM connector hotplug event was detected (monitor plugged/unplugged),
    /// signalling the compositor to re-enumerate outputs.
    pub fn drain_udev_events(&mut self) -> bool {
        let Some(ref monitor) = self.udev_monitor else {
            return false;
        };

        let mut hotplug_detected = false;
        {
            // The udev 0.8 MonitorSocket iterator returns events from
            // the non-blocking socket. When the FD is set O_NONBLOCK,
            // the iterator stops when the buffer is drained.
            for event in monitor.iter() {
                let action = event.action().and_then(|s| s.to_str()).unwrap_or("unknown");
                let devtype = event
                    .devtype()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                let subsystem = event
                    .subsystem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                debug!(
                    "udev DRM event: action={}, devtype={}, subsystem={}",
                    action, devtype, subsystem
                );
                // A "change" action on a drm device means connector
                // state changed (monitor plugged/unplugged).
                if subsystem == "drm" && action == "change" {
                    hotplug_detected = true;
                    info!(
                        "🔌 DRM hotplug detected (udev action={}, devtype={})",
                        action, devtype
                    );
                }
            }
        } // end of for-event scope

        hotplug_detected
    }

    /// Re-enumerate KMS outputs after a hotplug event.
    ///
    /// Re-opens the DRM device, re-scans connectors, and diffs the new
    /// output list against the existing one. Returns a tuple of
    /// `(added_names, removed_names)` so the compositor can look up
    /// the full [`KmsOutput`] details via [`kms_outputs`] and create
    /// or destroy Smithay `Output` objects and workspace tapes.
    ///
    /// The existing KMS state is replaced with the new one.
    pub fn reenumerate_outputs(&mut self) -> Result<(Vec<String>, Vec<String>), anyhow::Error> {
        let device_path = self.primary_device.as_deref().unwrap_or("/dev/dri/card0");
        info!("🔄 Re-enumerating DRM outputs on {}", device_path);

        let new_kms = KmsState::open(device_path)?;

        // Diff: which outputs are new and which are gone?
        let old_names: std::collections::HashSet<&str> = self
            .kms
            .as_ref()
            .map(|k| k.outputs.iter().map(|o| o.name.as_str()).collect())
            .unwrap_or_default();

        let new_names: std::collections::HashSet<&str> =
            new_kms.outputs.iter().map(|o| o.name.as_str()).collect();

        let added: Vec<String> = new_names
            .difference(&old_names)
            .map(|s| s.to_string())
            .collect();

        let removed: Vec<String> = old_names
            .difference(&new_names)
            .map(|s| s.to_string())
            .collect();

        if added.is_empty() && removed.is_empty() {
            info!("Hotplug re-enumeration: no changes detected");
            return Ok((Vec::new(), Vec::new()));
        }

        info!(
            "Hotplug diff: {} added, {} removed",
            added.len(),
            removed.len()
        );

        // Replace old KMS state with new.
        if let Some(mut old) = self.kms.replace(new_kms) {
            old.cleanup()?;
        }

        Ok((added, removed))
    }

    /// Dispatch pending libinput events and return them.
    ///
    /// Call this after the libinput FD polls readable. Events are
    /// returned as [`input::event::Event`] for the compositor to
    /// translate into Smithay seat events and Axiom input actions.
    pub fn dispatch_libinput(&mut self) -> Vec<input::Event> {
        let Some(ref mut libinput) = self.libinput else {
            return Vec::new();
        };

        if let Err(e) = libinput.dispatch() {
            warn!("libinput dispatch error: {}", e);
            return Vec::new();
        }

        let mut events = Vec::new();
        for ev in libinput.by_ref() {
            events.push(ev);
        }
        events
    }

    /// Return the libinput file descriptor for polling, or -1.
    pub fn libinput_fd(&self) -> i32 {
        self.libinput
            .as_ref()
            .map(|li| li.as_raw_fd())
            .unwrap_or(-1)
    }
}

impl Default for DrmBackend {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── BackendKind ────────────────────────────────────────────────────────

    #[test]
    fn test_backend_kind_winit() {
        for s in &["winit", "Winit", "WINIT", "windowed", "dev"] {
            assert_eq!(BackendKind::from_config_str(s), BackendKind::Winit);
        }
    }

    #[test]
    fn test_backend_kind_drm() {
        for s in &["drm", "DRM", "kms", "KMS", "session", "tty"] {
            assert_eq!(BackendKind::from_config_str(s), BackendKind::Drm);
        }
    }

    #[test]
    fn test_backend_kind_noop() {
        for s in &["noop", "NOOP", "test", "headless"] {
            assert_eq!(BackendKind::from_config_str(s), BackendKind::Noop);
        }
    }

    #[test]
    fn test_backend_kind_unknown_falls_back_to_winit() {
        assert_eq!(BackendKind::from_config_str("bogus"), BackendKind::Winit);
        assert_eq!(BackendKind::from_config_str(""), BackendKind::Winit);
        assert_eq!(
            BackendKind::from_config_str("drm_backend"),
            BackendKind::Winit
        );
    }

    // ── DRM probe ──────────────────────────────────────────────────────────

    #[test]
    fn test_probe_returns_false_in_ci() {
        // In CI or on systems without DRM devices, this returns false.
        // The important thing is that it doesn't panic.
        let available = probe_drm_available();
        // We don't assert true or false — just that it ran without error.
        let _ = available;
    }

    // ── DrmEventCollector ──────────────────────────────────────────────────

    #[test]
    fn test_event_collector_default() {
        let c = DrmEventCollector::default();
        assert!(!c.drm_ready);
        assert!(!c.libinput_ready);
    }

    #[test]
    fn test_copy_bgra_to_xrgb8888_copies_and_sets_padding() {
        let src = vec![
            10, 20, 30, 40, // pixel 0: B,G,R,A
            50, 60, 70, 80, // pixel 1
        ];
        let mut dst = vec![0u8; 8];
        copy_bgra_to_xrgb8888(&src, 2, 1, 0, 0, &mut dst, 2, 1, 8);
        assert_eq!(dst, vec![10, 20, 30, 255, 50, 60, 70, 255]);
    }

    #[test]
    fn test_copy_bgra_to_xrgb8888_crops_to_destination() {
        let src = vec![
            1, 2, 3, 4, 5, 6, 7, 8,
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let mut dst = vec![0u8; 4];
        copy_bgra_to_xrgb8888(&src, 2, 2, 0, 0, &mut dst, 1, 1, 4);
        assert_eq!(dst, vec![1, 2, 3, 255]);
    }

    #[test]
    fn test_copy_bgra_to_xrgb8888_respects_source_origin() {
        let src = vec![
            1, 2, 3, 4, 5, 6, 7, 8,
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let mut dst = vec![0u8; 4];
        copy_bgra_to_xrgb8888(&src, 2, 2, 1, 0, &mut dst, 1, 1, 4);
        assert_eq!(dst, vec![5, 6, 7, 255]);
    }

    #[test]
    fn test_scale_factor_from_edid_can_be_fractional() {
        // Chosen so the effective DPI is ~144, which should quantize to 1.5x.
        let scale = scale_factor_from_edid(1920, 1080, 338, 192);
        assert!((scale - 1.5).abs() < 0.01, "expected ~1.5x, got {scale}");
    }

    // ── CalloopFd ──────────────────────────────────────────────────────────

    #[test]
    fn test_calloop_fd_dup_invalid() {
        let result = CalloopFd::dup(-1);
        assert!(result.is_err());
    }

    #[test]
    fn test_calloop_fd_stdout_is_valid_fd() {
        let result = CalloopFd::dup(1); // stdout
        assert!(result.is_ok());
        let fd = result.unwrap();
        // The dup'd FD should be usable (as_fd should not panic).
        let _borrowed = fd.as_fd();
    }

    // ── DrmBackend ─────────────────────────────────────────────────────────

    #[test]
    fn test_drm_backend_new_does_not_panic() {
        let backend = DrmBackend::new();
        let _ = backend.available;
    }

    #[test]
    fn test_drm_backend_default_trait() {
        let backend = DrmBackend::default();
        assert_eq!(backend.available, probe_drm_available());
    }

    #[test]
    fn test_drm_backend_initialize_does_not_panic() {
        let mut backend = DrmBackend::new();
        // May fail if no DRM device is available (CI).
        let _result = backend.initialize();
    }

    #[test]
    fn test_drm_backend_shutdown_does_not_panic() {
        let mut backend = DrmBackend::new();
        let _ = backend.shutdown();
    }

    #[test]
    fn test_drm_backend_calloop_not_initialized_by_default() {
        let backend = DrmBackend::new();
        assert!(backend.calloop_loop.is_none());
        assert!(backend.calloop_handle.is_none());
    }

    #[test]
    fn test_dispatch_calloop_without_init_returns_default_collector() {
        let mut backend = DrmBackend::new();
        let collector = backend.dispatch_calloop().unwrap();
        assert!(!collector.drm_ready);
        assert!(!collector.libinput_ready);
    }

    #[test]
    fn test_libinput_fd_without_libinput_returns_negative_one() {
        let backend = DrmBackend::new();
        assert_eq!(backend.libinput_fd(), -1);
    }

    #[test]
    fn test_drm_backend_repeated_shutdown_is_idempotent() {
        let mut backend = DrmBackend::new();
        assert!(backend.shutdown().is_ok());
        // Second shutdown should also succeed.
        assert!(backend.shutdown().is_ok());
        // After shutdown, state is reset.
        assert!(!backend.available);
        assert!(backend.primary_device.is_none());
    }

    #[test]
    fn test_init_calloop_loop_without_kms_or_libinput_succeeds() {
        let mut backend = DrmBackend::new();
        // Even without KMS or libinput, init_calloop_loop should succeed
        // (it just won't register any FD sources).
        assert!(backend.init_calloop_loop().is_ok());
        assert!(backend.calloop_loop.is_some());
        assert!(backend.calloop_handle.is_some());
        // Calling it again is a no-op.
        assert!(backend.init_calloop_loop().is_ok());
    }

    // ── Card ───────────────────────────────────────────────────────────────

    #[test]
    fn test_card_wrapper_creation() {
        let result = Card::open("/dev/dri/card0");
        if let Err(e) = result {
            assert!(
                e.to_string().contains("Failed to open DRM device"),
                "Unexpected error: {}",
                e
            );
        }
    }
}
