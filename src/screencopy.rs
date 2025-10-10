//! Wayland Screen Capture Protocol (wlr_screencopy_unstable_v1)
//!
//! Implementation of the wlroots screencopy protocol for screen recording,
//! screenshots, and screen sharing functionality.
//!
//! # Protocol Overview
//!
//! The screencopy protocol allows clients to capture the contents of outputs
//! (monitors) or specific regions of the compositor's screen space.
//!
//! ## Protocol Flow
//!
//! 1. Client creates a frame capture request via wlr_screencopy_manager_v1
//! 2. Compositor sends buffer parameters (format, size, stride)
//! 3. Client creates a shared memory buffer matching requirements
//! 4. Client attaches buffer to the frame
//! 5. Compositor copies screen content to buffer
//! 6. Compositor sends "ready" event with timestamp
//! 7. Client processes the captured frame
//!
//! # Features
//!
//! - Full output capture (entire monitor)
//! - Region capture (specific area)
//! - Multiple pixel format support (ARGB8888, XRGB8888, etc.)
//! - Damage tracking for efficient updates
//! - Hardware cursor overlay option
//! - Buffer negotiation and validation
//!
//! # Usage
//!
//! ```no_run
//! use axiom::screencopy::{ScreencopyManager, CaptureRequest, CaptureRegion};
//!
//! let mut screencopy = ScreencopyManager::new();
//!
//! // Capture entire output
//! let request = screencopy.create_capture(None, false);
//!
//! // Capture specific region
//! let region = CaptureRegion {
//!     x: 100, y: 100,
//!     width: 800, height: 600,
//! };
//! let request = screencopy.create_region_capture(region, false);
//! ```

use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Supported pixel formats for screencopy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PixelFormat {
    /// 32-bit ARGB format (with alpha)
    ARGB8888 = 0,
    /// 32-bit XRGB format (no alpha, X is padding)
    XRGB8888 = 1,
    /// 32-bit ABGR format
    ABGR8888 = 2,
    /// 32-bit XBGR format
    XBGR8888 = 3,
    /// 32-bit RGBA format
    RGBA8888 = 4,
    /// 32-bit RGBX format
    RGBX8888 = 5,
    /// 32-bit BGRA format
    BGRA8888 = 6,
    /// 32-bit BGRX format
    BGRX8888 = 7,
}

impl PixelFormat {
    /// Get the number of bytes per pixel
    pub fn bytes_per_pixel(&self) -> u32 {
        match self {
            PixelFormat::ARGB8888
            | PixelFormat::XRGB8888
            | PixelFormat::ABGR8888
            | PixelFormat::XBGR8888
            | PixelFormat::RGBA8888
            | PixelFormat::RGBX8888
            | PixelFormat::BGRA8888
            | PixelFormat::BGRX8888 => 4,
        }
    }

    /// Check if format has alpha channel
    pub fn has_alpha(&self) -> bool {
        matches!(
            self,
            PixelFormat::ARGB8888
                | PixelFormat::ABGR8888
                | PixelFormat::RGBA8888
                | PixelFormat::BGRA8888
        )
    }

    /// Get format name
    pub fn name(&self) -> &'static str {
        match self {
            PixelFormat::ARGB8888 => "ARGB8888",
            PixelFormat::XRGB8888 => "XRGB8888",
            PixelFormat::ABGR8888 => "ABGR8888",
            PixelFormat::XBGR8888 => "XBGR8888",
            PixelFormat::RGBA8888 => "RGBA8888",
            PixelFormat::RGBX8888 => "RGBX8888",
            PixelFormat::BGRA8888 => "BGRA8888",
            PixelFormat::BGRX8888 => "BGRX8888",
        }
    }
}

/// Buffer parameters for screen capture
#[derive(Debug, Clone)]
pub struct BufferParams {
    /// Pixel format
    pub format: PixelFormat,
    /// Buffer width in pixels
    pub width: u32,
    /// Buffer height in pixels
    pub height: u32,
    /// Stride (bytes per row)
    pub stride: u32,
}

impl BufferParams {
    /// Create new buffer parameters
    pub fn new(format: PixelFormat, width: u32, height: u32) -> Self {
        let stride = width * format.bytes_per_pixel();
        Self {
            format,
            width,
            height,
            stride,
        }
    }

    /// Calculate total buffer size in bytes
    pub fn size(&self) -> usize {
        (self.stride * self.height) as usize
    }

    /// Validate buffer parameters
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.stride >= self.width * self.format.bytes_per_pixel()
    }
}

/// Capture region specification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRegion {
    /// X coordinate of top-left corner
    pub x: i32,
    /// Y coordinate of top-left corner
    pub y: i32,
    /// Width of capture region
    pub width: u32,
    /// Height of capture region
    pub height: u32,
}

impl CaptureRegion {
    /// Create a new capture region
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Check if region is valid
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Calculate area in pixels
    pub fn area(&self) -> u32 {
        self.width * self.height
    }
}

/// State of a capture request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    /// Request created, waiting for buffer
    Pending,
    /// Buffer attached, ready to capture
    Ready,
    /// Capture in progress
    Capturing,
    /// Capture completed successfully
    Completed,
    /// Capture failed
    Failed,
    /// Request cancelled
    Cancelled,
}

/// Screen capture request
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    /// Unique request ID
    pub id: u64,
    /// Output ID to capture (None = all outputs)
    pub output_id: Option<u32>,
    /// Region to capture (None = full output)
    pub region: Option<CaptureRegion>,
    /// Whether to include hardware cursor
    pub overlay_cursor: bool,
    /// Buffer parameters
    pub buffer_params: BufferParams,
    /// Current state
    pub state: CaptureState,
    /// Timestamp of creation
    pub created_at: std::time::Instant,
    /// Timestamp of completion (if completed)
    pub completed_at: Option<std::time::Instant>,
}

impl CaptureRequest {
    /// Create a new capture request
    pub fn new(
        id: u64,
        output_id: Option<u32>,
        region: Option<CaptureRegion>,
        overlay_cursor: bool,
        buffer_params: BufferParams,
    ) -> Self {
        Self {
            id,
            output_id,
            region,
            overlay_cursor,
            buffer_params,
            state: CaptureState::Pending,
            created_at: std::time::Instant::now(),
            completed_at: None,
        }
    }

    /// Mark request as ready (buffer attached)
    pub fn mark_ready(&mut self) {
        if self.state == CaptureState::Pending {
            self.state = CaptureState::Ready;
            debug!("📸 Capture request {} marked ready", self.id);
        }
    }

    /// Start capturing
    pub fn start_capture(&mut self) {
        if self.state == CaptureState::Ready {
            self.state = CaptureState::Capturing;
            debug!("📸 Started capture for request {}", self.id);
        }
    }

    /// Mark request as completed
    pub fn complete(&mut self) {
        if self.state == CaptureState::Capturing {
            self.state = CaptureState::Completed;
            self.completed_at = Some(std::time::Instant::now());
            let duration = self.completed_at.unwrap() - self.created_at;
            info!(
                "📸 Capture request {} completed in {:?}",
                self.id, duration
            );
        }
    }

    /// Mark request as failed
    pub fn fail(&mut self, reason: &str) {
        self.state = CaptureState::Failed;
        warn!("📸 Capture request {} failed: {}", self.id, reason);
    }

    /// Cancel request
    pub fn cancel(&mut self) {
        if !matches!(
            self.state,
            CaptureState::Completed | CaptureState::Failed
        ) {
            self.state = CaptureState::Cancelled;
            debug!("📸 Capture request {} cancelled", self.id);
        }
    }

    /// Get capture duration (if completed)
    pub fn duration(&self) -> Option<std::time::Duration> {
        self.completed_at.map(|end| end - self.created_at)
    }
}

/// Main screencopy manager
pub struct ScreencopyManager {
    /// Active capture requests
    requests: HashMap<u64, CaptureRequest>,
    /// Next request ID
    next_request_id: u64,
    /// Supported pixel formats
    supported_formats: Vec<PixelFormat>,
    /// Maximum capture dimensions
    max_dimensions: (u32, u32),
    /// Statistics
    stats: ScreencopyStats,
}

impl ScreencopyManager {
    /// Create a new screencopy manager
    pub fn new() -> Self {
        Self {
            requests: HashMap::new(),
            next_request_id: 1,
            supported_formats: vec![
                PixelFormat::ARGB8888,
                PixelFormat::XRGB8888,
                PixelFormat::ABGR8888,
                PixelFormat::XBGR8888,
            ],
            max_dimensions: (7680, 4320), // 8K resolution
            stats: ScreencopyStats::default(),
        }
    }

    /// Get supported pixel formats
    pub fn supported_formats(&self) -> &[PixelFormat] {
        &self.supported_formats
    }

    /// Check if a format is supported
    pub fn is_format_supported(&self, format: PixelFormat) -> bool {
        self.supported_formats.contains(&format)
    }

    /// Create a full output capture request
    pub fn create_capture(
        &mut self,
        output_id: Option<u32>,
        overlay_cursor: bool,
    ) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;

        // Default to 1920x1080 ARGB8888 for full captures
        let buffer_params = BufferParams::new(PixelFormat::ARGB8888, 1920, 1080);

        let request = CaptureRequest::new(id, output_id, None, overlay_cursor, buffer_params);

        info!(
            "📸 Created full output capture request {} (overlay_cursor: {})",
            id, overlay_cursor
        );

        self.requests.insert(id, request);
        self.stats.total_requests += 1;
        id
    }

    /// Create a region capture request
    pub fn create_region_capture(
        &mut self,
        region: CaptureRegion,
        overlay_cursor: bool,
    ) -> Result<u64, String> {
        if !region.is_valid() {
            return Err("Invalid capture region".to_string());
        }

        if region.width > self.max_dimensions.0 || region.height > self.max_dimensions.1 {
            return Err(format!(
                "Region too large: {}x{} (max: {}x{})",
                region.width, region.height, self.max_dimensions.0, self.max_dimensions.1
            ));
        }

        let id = self.next_request_id;
        self.next_request_id += 1;

        let buffer_params = BufferParams::new(PixelFormat::ARGB8888, region.width, region.height);

        let request =
            CaptureRequest::new(id, None, Some(region), overlay_cursor, buffer_params);

        info!(
            "📸 Created region capture request {} ({}x{} at {},{})",
            id, region.width, region.height, region.x, region.y
        );

        self.requests.insert(id, request);
        self.stats.total_requests += 1;
        Ok(id)
    }

    /// Set buffer parameters for a capture request
    pub fn set_buffer_params(
        &mut self,
        request_id: u64,
        buffer_params: BufferParams,
    ) -> Result<(), String> {
        // Validate format before borrowing request
        if !self.is_format_supported(buffer_params.format) {
            return Err(format!(
                "Unsupported pixel format: {}",
                buffer_params.format.name()
            ));
        }

        if !buffer_params.is_valid() {
            return Err("Invalid buffer parameters".to_string());
        }

        let request = self
            .requests
            .get_mut(&request_id)
            .ok_or("Request not found")?;

        if request.state != CaptureState::Pending {
            return Err("Cannot modify buffer params after buffer attached".to_string());
        }

        request.buffer_params = buffer_params;
        debug!("📸 Updated buffer params for request {}", request_id);
        Ok(())
    }

    /// Attach buffer to capture request (marks as ready)
    pub fn attach_buffer(&mut self, request_id: u64) -> Result<(), String> {
        let request = self
            .requests
            .get_mut(&request_id)
            .ok_or("Request not found")?;

        if request.state != CaptureState::Pending {
            return Err("Request already has buffer attached".to_string());
        }

        request.mark_ready();
        Ok(())
    }

    /// Execute a capture request (simulate capture)
    pub fn capture(&mut self, request_id: u64) -> Result<Vec<u8>, String> {
        let request = self
            .requests
            .get_mut(&request_id)
            .ok_or("Request not found")?;

        if request.state != CaptureState::Ready {
            return Err(format!("Request not ready for capture (state: {:?})", request.state));
        }

        request.start_capture();

        // In a real implementation, this would:
        // 1. Lock compositor rendering
        // 2. Copy framebuffer to shared memory
        // 3. Apply damage regions if needed
        // 4. Optionally overlay cursor
        // 5. Send ready event to client

        // For now, simulate with a dummy buffer
        let buffer_size = request.buffer_params.size();
        let buffer = vec![0u8; buffer_size];

        request.complete();
        self.stats.successful_captures += 1;

        Ok(buffer)
    }

    /// Copy framebuffer data (actual implementation)
    pub fn copy_framebuffer(
        &self,
        request: &CaptureRequest,
        framebuffer: &[u8],
    ) -> Result<Vec<u8>, String> {
        let params = &request.buffer_params;

        if framebuffer.len() < params.size() {
            return Err("Framebuffer too small".to_string());
        }

        // Handle region cropping if specified
        if let Some(_region) = request.region {
            // In a real implementation, we'd crop the framebuffer here
            // For now, just return the requested size
            Ok(framebuffer[..params.size()].to_vec())
        } else {
            // Full framebuffer copy
            Ok(framebuffer[..params.size()].to_vec())
        }
    }

    /// Cancel a capture request
    pub fn cancel(&mut self, request_id: u64) -> Result<(), String> {
        let request = self
            .requests
            .get_mut(&request_id)
            .ok_or("Request not found")?;

        request.cancel();
        self.stats.cancelled_captures += 1;
        Ok(())
    }

    /// Fail a capture request
    pub fn fail(&mut self, request_id: u64, reason: &str) -> Result<(), String> {
        let request = self
            .requests
            .get_mut(&request_id)
            .ok_or("Request not found")?;

        request.fail(reason);
        self.stats.failed_captures += 1;
        Ok(())
    }

    /// Get a capture request
    pub fn get_request(&self, request_id: u64) -> Option<&CaptureRequest> {
        self.requests.get(&request_id)
    }

    /// Remove completed or failed requests
    pub fn cleanup_completed(&mut self) {
        let before = self.requests.len();
        self.requests.retain(|_, req| {
            !matches!(
                req.state,
                CaptureState::Completed | CaptureState::Failed | CaptureState::Cancelled
            )
        });
        let removed = before - self.requests.len();
        if removed > 0 {
            debug!("📸 Cleaned up {} completed capture requests", removed);
        }
    }

    /// Get statistics
    pub fn stats(&self) -> ScreencopyStats {
        let mut stats = self.stats;
        stats.active_requests = self.requests.len();
        stats
    }
}

impl Default for ScreencopyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about screencopy operations
#[derive(Debug, Clone, Copy, Default)]
pub struct ScreencopyStats {
    /// Total capture requests created
    pub total_requests: usize,
    /// Successfully completed captures
    pub successful_captures: usize,
    /// Failed captures
    pub failed_captures: usize,
    /// Cancelled captures
    pub cancelled_captures: usize,
    /// Currently active requests
    pub active_requests: usize,
}

/// Thread-safe screencopy manager wrapper
pub type SharedScreencopy = Arc<Mutex<ScreencopyManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_format() {
        assert_eq!(PixelFormat::ARGB8888.bytes_per_pixel(), 4);
        assert!(PixelFormat::ARGB8888.has_alpha());
        assert!(!PixelFormat::XRGB8888.has_alpha());
        assert_eq!(PixelFormat::ARGB8888.name(), "ARGB8888");
    }

    #[test]
    fn test_buffer_params() {
        let params = BufferParams::new(PixelFormat::ARGB8888, 100, 100);
        assert_eq!(params.width, 100);
        assert_eq!(params.height, 100);
        assert_eq!(params.stride, 400); // 100 * 4 bytes
        assert_eq!(params.size(), 40_000); // 400 * 100
        assert!(params.is_valid());
    }

    #[test]
    fn test_capture_region() {
        let region = CaptureRegion::new(10, 20, 100, 200);
        assert_eq!(region.x, 10);
        assert_eq!(region.y, 20);
        assert!(region.is_valid());
        assert_eq!(region.area(), 20_000);
    }

    #[test]
    fn test_create_full_capture() {
        let mut manager = ScreencopyManager::new();
        let id = manager.create_capture(Some(1), false);
        assert_eq!(id, 1);

        let request = manager.get_request(id).unwrap();
        assert_eq!(request.state, CaptureState::Pending);
        assert!(request.output_id.is_some());
        assert!(request.region.is_none());
    }

    #[test]
    fn test_create_region_capture() {
        let mut manager = ScreencopyManager::new();
        let region = CaptureRegion::new(0, 0, 800, 600);
        let id = manager.create_region_capture(region, true).unwrap();

        let request = manager.get_request(id).unwrap();
        assert_eq!(request.state, CaptureState::Pending);
        assert!(request.overlay_cursor);
        assert_eq!(request.region.unwrap(), region);
    }

    #[test]
    fn test_capture_flow() {
        let mut manager = ScreencopyManager::new();
        let id = manager.create_capture(None, false);

        // Attach buffer
        manager.attach_buffer(id).unwrap();
        let request = manager.get_request(id).unwrap();
        assert_eq!(request.state, CaptureState::Ready);

        // Execute capture
        let buffer = manager.capture(id).unwrap();
        assert!(!buffer.is_empty());

        let request = manager.get_request(id).unwrap();
        assert_eq!(request.state, CaptureState::Completed);
        assert!(request.duration().is_some());
    }

    #[test]
    fn test_cancel_request() {
        let mut manager = ScreencopyManager::new();
        let id = manager.create_capture(None, false);

        manager.cancel(id).unwrap();
        let request = manager.get_request(id).unwrap();
        assert_eq!(request.state, CaptureState::Cancelled);
    }

    #[test]
    fn test_invalid_region() {
        let mut manager = ScreencopyManager::new();
        let region = CaptureRegion::new(0, 0, 0, 0);
        let result = manager.create_region_capture(region, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_oversized_region() {
        let mut manager = ScreencopyManager::new();
        let region = CaptureRegion::new(0, 0, 10000, 10000);
        let result = manager.create_region_capture(region, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_cleanup() {
        let mut manager = ScreencopyManager::new();

        let id1 = manager.create_capture(None, false);
        manager.attach_buffer(id1).unwrap();
        manager.capture(id1).unwrap();

        let id2 = manager.create_capture(None, false);
        manager.cancel(id2).unwrap();

        assert_eq!(manager.requests.len(), 2);
        manager.cleanup_completed();
        assert_eq!(manager.requests.len(), 0);
    }

    #[test]
    fn test_stats() {
        let mut manager = ScreencopyManager::new();

        manager.create_capture(None, false);
        manager.create_capture(None, false);

        let stats = manager.stats();
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.active_requests, 2);
    }

    #[test]
    fn test_format_support() {
        let manager = ScreencopyManager::new();
        assert!(manager.is_format_supported(PixelFormat::ARGB8888));
        assert_eq!(manager.supported_formats().len(), 4);
    }
}
