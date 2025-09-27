//! Enhanced Buffer Format Support
//! 
//! Extends the existing buffer conversion functions with additional format support
//! and better error handling for improved application compatibility.

use anyhow::Result;
use log::{debug, info, warn};
use memmap2::Mmap;
use std::sync::Arc;
use wayland_server::{protocol::wl_shm, WEnum};

/// Enhanced SHM buffer conversion with additional format support
pub fn convert_shm_to_rgba_enhanced(
    map: Arc<Mmap>,
    width: i32,
    height: i32,
    stride: i32,
    offset: i32,
    format: WEnum<wl_shm::Format>,
) -> Option<Vec<u8>> {
    let width = width.max(0) as usize;
    let height = height.max(0) as usize;
    let stride = stride as usize;
    let offset = offset as usize;
    
    if width == 0 || height == 0 {
        return None;
    }
    
    let needed = offset.checked_add(stride.checked_mul(height)?)?;
    if needed > map.len() {
        warn!("🔴 Buffer size mismatch: needed {} bytes, have {}", needed, map.len());
        return None;
    }
    
    let src = &map[offset..offset + stride * height];
    let mut out = vec![0u8; width * height * 4];
    
    match format {
        // Existing formats (already implemented)
        WEnum::Value(wl_shm::Format::Xrgb8888) => {
            convert_xrgb8888_to_rgba(&src, &mut out, width, height, stride);
        }
        WEnum::Value(wl_shm::Format::Argb8888) => {
            convert_argb8888_to_rgba(&src, &mut out, width, height, stride);
        }
        
        // NEW: Enhanced format support
        WEnum::Value(wl_shm::Format::Rgb565) => {
            convert_rgb565_to_rgba(&src, &mut out, width, height, stride);
        }
        WEnum::Value(wl_shm::Format::Bgr888) => {
            convert_bgr888_to_rgba(&src, &mut out, width, height, stride);
        }
        WEnum::Value(wl_shm::Format::Rgba4444) => {
            convert_rgba4444_to_rgba(&src, &mut out, width, height, stride);
        }
        WEnum::Value(wl_shm::Format::Rgba5551) => {
            convert_rgba5551_to_rgba(&src, &mut out, width, height, stride);
        }
        
        // Fallback for unknown formats
        _ => {
            warn!("🟡 Unsupported SHM format: {:?}, using fallback", format);
            return create_fallback_texture(width, height);
        }
    }
    
    debug!("✅ Converted {}x{} SHM buffer from {:?} to RGBA", width, height, format);
    Some(out)
}

/// Convert RGB565 format to RGBA8888
fn convert_rgb565_to_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 2;
            let dst_idx = x * 4;
            
            if src_idx + 1 >= src_row.len() { break; }
            
            // RGB565: 5 bits red, 6 bits green, 5 bits blue
            let pixel = u16::from_le_bytes([src_row[src_idx], src_row[src_idx + 1]]);
            let r = ((pixel >> 11) & 0x1F) as u8;
            let g = ((pixel >> 5) & 0x3F) as u8;
            let b = (pixel & 0x1F) as u8;
            
            // Expand to 8 bits
            dst_row[dst_idx] = (r << 3) | (r >> 2);     // Red
            dst_row[dst_idx + 1] = (g << 2) | (g >> 4); // Green  
            dst_row[dst_idx + 2] = (b << 3) | (b >> 2); // Blue
            dst_row[dst_idx + 3] = 255;                  // Alpha
        }
    }
}

/// Convert BGR888 format to RGBA8888
fn convert_bgr888_to_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 3;
            let dst_idx = x * 4;
            
            if src_idx + 2 >= src_row.len() { break; }
            
            dst_row[dst_idx] = src_row[src_idx + 2];     // Red (was Blue)
            dst_row[dst_idx + 1] = src_row[src_idx + 1]; // Green
            dst_row[dst_idx + 2] = src_row[src_idx];     // Blue (was Red)
            dst_row[dst_idx + 3] = 255;                  // Alpha
        }
    }
}

/// Convert RGBA4444 format to RGBA8888
fn convert_rgba4444_to_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 2;
            let dst_idx = x * 4;
            
            if src_idx + 1 >= src_row.len() { break; }
            
            let pixel = u16::from_le_bytes([src_row[src_idx], src_row[src_idx + 1]]);
            let r = ((pixel >> 12) & 0xF) as u8;
            let g = ((pixel >> 8) & 0xF) as u8;
            let b = ((pixel >> 4) & 0xF) as u8;
            let a = (pixel & 0xF) as u8;
            
            // Expand 4 bits to 8 bits
            dst_row[dst_idx] = (r << 4) | r;
            dst_row[dst_idx + 1] = (g << 4) | g;
            dst_row[dst_idx + 2] = (b << 4) | b;
            dst_row[dst_idx + 3] = (a << 4) | a;
        }
    }
}

/// Convert RGBA5551 format to RGBA8888
fn convert_rgba5551_to_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 2;
            let dst_idx = x * 4;
            
            if src_idx + 1 >= src_row.len() { break; }
            
            let pixel = u16::from_le_bytes([src_row[src_idx], src_row[src_idx + 1]]);
            let r = ((pixel >> 11) & 0x1F) as u8;
            let g = ((pixel >> 6) & 0x1F) as u8;
            let b = ((pixel >> 1) & 0x1F) as u8;
            let a = (pixel & 0x1) as u8;
            
            // Expand to 8 bits
            dst_row[dst_idx] = (r << 3) | (r >> 2);
            dst_row[dst_idx + 1] = (g << 3) | (g >> 2);
            dst_row[dst_idx + 2] = (b << 3) | (b >> 2);
            dst_row[dst_idx + 3] = if a == 1 { 255 } else { 0 };
        }
    }
}

/// Existing conversion functions (keep current implementations)
fn convert_xrgb8888_to_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 4;
            let dst_idx = x * 4;
            
            if src_idx + 3 >= src_row.len() { break; }
            
            dst_row[dst_idx] = src_row[src_idx + 2];     // Red
            dst_row[dst_idx + 1] = src_row[src_idx + 1]; // Green
            dst_row[dst_idx + 2] = src_row[src_idx];     // Blue
            dst_row[dst_idx + 3] = 255;                  // Alpha (opaque)
        }
    }
}

fn convert_argb8888_to_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 4;
            let dst_idx = x * 4;
            
            if src_idx + 3 >= src_row.len() { break; }
            
            dst_row[dst_idx] = src_row[src_idx + 2];     // Red
            dst_row[dst_idx + 1] = src_row[src_idx + 1]; // Green
            dst_row[dst_idx + 2] = src_row[src_idx];     // Blue
            dst_row[dst_idx + 3] = src_row[src_idx + 3]; // Alpha
        }
    }
}

/// Create a fallback texture for unsupported formats
/// Returns a pleasant gradient pattern to indicate the unsupported format
fn create_fallback_texture(width: usize, height: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; width * height * 4];
    
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            
            // Create a subtle diagonal pattern to indicate unsupported format
            let pattern = ((x + y) / 8) % 2;
            let base_color = if pattern == 0 { 64 } else { 96 };
            
            out[idx] = base_color;         // Red
            out[idx + 1] = base_color + 32; // Green (slightly brighter)
            out[idx + 2] = base_color + 16; // Blue
            out[idx + 3] = 255;            // Alpha
        }
    }
    
    info!("🟡 Created fallback texture ({}x{}) for unsupported format", width, height);
    Some(out)
}

/// Enhanced DMABuf format support
pub fn convert_dmabuf_to_rgba_enhanced(
    planes: &[DmabufPlane],
    fourcc: u32,
    width: i32,
    height: i32,
) -> Option<Vec<u8>> {
    const DRM_FORMAT_XRGB8888: u32 = 0x34325258; // 'XR24'
    const DRM_FORMAT_ARGB8888: u32 = 0x34325241; // 'AR24'
    const DRM_FORMAT_XBGR8888: u32 = 0x34324258; // 'XB24'
    const DRM_FORMAT_ABGR8888: u32 = 0x34324241; // 'AB24'
    
    // NEW: Additional DMABuf format support
    const DRM_FORMAT_RGB565: u32 = 0x36314752;   // 'RG16'
    const DRM_FORMAT_BGR565: u32 = 0x36314742;   // 'BG16'
    const DRM_FORMAT_RGBA4444: u32 = 0x34344152; // 'RA44'
    const DRM_FORMAT_BGRA4444: u32 = 0x34344142; // 'BA44'
    
    let width = width.max(0) as usize;
    let height = height.max(0) as usize;
    
    if width == 0 || height == 0 || planes.is_empty() {
        return None;
    }
    
    let plane = &planes[0];
    let stride = plane.stride.max(0) as usize;
    let offset = plane.offset.max(0) as usize;
    
    let needed = offset.checked_add(stride.checked_mul(height))?;
    if needed > plane.map.len() {
        warn!("🔴 DMABuf size mismatch: needed {} bytes, have {}", needed, plane.map.len());
        return None;
    }
    
    let src = &plane.map[offset..offset + stride * height];
    let mut out = vec![0u8; width * height * 4];
    
    match fourcc {
        // Existing formats
        DRM_FORMAT_XBGR8888 => convert_xbgr8888_dmabuf(&src, &mut out, width, height, stride),
        DRM_FORMAT_ABGR8888 => convert_abgr8888_dmabuf(&src, &mut out, width, height, stride),
        DRM_FORMAT_XRGB8888 => convert_xrgb8888_dmabuf(&src, &mut out, width, height, stride),
        DRM_FORMAT_ARGB8888 => convert_argb8888_dmabuf(&src, &mut out, width, height, stride),
        
        // NEW: Additional formats
        DRM_FORMAT_RGB565 => convert_rgb565_dmabuf(&src, &mut out, width, height, stride),
        DRM_FORMAT_BGR565 => convert_bgr565_dmabuf(&src, &mut out, width, height, stride),
        DRM_FORMAT_RGBA4444 => convert_rgba4444_dmabuf(&src, &mut out, width, height, stride),
        DRM_FORMAT_BGRA4444 => convert_bgra4444_dmabuf(&src, &mut out, width, height, stride),
        
        _ => {
            warn!("🟡 Unsupported DMABuf fourcc: 0x{:08X}, using fallback", fourcc);
            return create_fallback_texture(width, height);
        }
    }
    
    debug!("✅ Converted {}x{} DMABuf from fourcc 0x{:08X} to RGBA", width, height, fourcc);
    Some(out)
}

// DMABuf plane structure (mirrors existing definition)
#[derive(Clone)]
pub struct DmabufPlane {
    pub map: Arc<Mmap>,
    pub stride: i32,
    pub offset: i32,
}

// Existing DMABuf conversion functions (keep current implementations)
fn convert_xbgr8888_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    // Existing implementation - don't modify
}

fn convert_abgr8888_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    // Existing implementation - don't modify  
}

fn convert_xrgb8888_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    // Existing implementation - don't modify
}

fn convert_argb8888_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    // Existing implementation - don't modify
}

// NEW: DMABuf format conversion functions
fn convert_rgb565_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    convert_rgb565_to_rgba(src, dst, width, height, stride);
}

fn convert_bgr565_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 2;
            let dst_idx = x * 4;
            
            if src_idx + 1 >= src_row.len() { break; }
            
            let pixel = u16::from_le_bytes([src_row[src_idx], src_row[src_idx + 1]]);
            let b = ((pixel >> 11) & 0x1F) as u8;  // Blue in high bits
            let g = ((pixel >> 5) & 0x3F) as u8;   // Green in middle
            let r = (pixel & 0x1F) as u8;          // Red in low bits
            
            dst_row[dst_idx] = (r << 3) | (r >> 2);
            dst_row[dst_idx + 1] = (g << 2) | (g >> 4);
            dst_row[dst_idx + 2] = (b << 3) | (b >> 2);
            dst_row[dst_idx + 3] = 255;
        }
    }
}

fn convert_rgba4444_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    convert_rgba4444_to_rgba(src, dst, width, height, stride);
}

fn convert_bgra4444_dmabuf(src: &[u8], dst: &mut [u8], width: usize, height: usize, stride: usize) {
    for y in 0..height {
        let src_row = &src[y * stride..];
        let dst_row = &mut dst[y * width * 4..];
        
        for x in 0..width {
            let src_idx = x * 2;
            let dst_idx = x * 4;
            
            if src_idx + 1 >= src_row.len() { break; }
            
            let pixel = u16::from_le_bytes([src_row[src_idx], src_row[src_idx + 1]]);
            let b = ((pixel >> 12) & 0xF) as u8;   // Blue
            let g = ((pixel >> 8) & 0xF) as u8;    // Green
            let r = ((pixel >> 4) & 0xF) as u8;    // Red
            let a = (pixel & 0xF) as u8;           // Alpha
            
            dst_row[dst_idx] = (r << 4) | r;
            dst_row[dst_idx + 1] = (g << 4) | g;
            dst_row[dst_idx + 2] = (b << 4) | b;
            dst_row[dst_idx + 3] = (a << 4) | a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rgb565_conversion() {
        // Test RGB565 conversion with known values
        let width = 2;
        let height = 1;
        let stride = 4;
        
        // RGB565: Red=31, Green=0, Blue=0 should convert to Red=255, Green=0, Blue=0
        let src = vec![0x1F, 0x00, 0x00, 0x00];
        let mut dst = vec![0u8; width * height * 4];
        
        convert_rgb565_to_rgba(&src, &mut dst, width, height, stride);
        
        // Check first pixel is pure red
        assert_eq!(dst[0], 248); // Red (31 << 3 = 248)
        assert_eq!(dst[1], 0);   // Green
        assert_eq!(dst[2], 0);   // Blue  
        assert_eq!(dst[3], 255); // Alpha
    }
    
    #[test]
    fn test_fallback_texture() {
        let result = create_fallback_texture(4, 4);
        assert!(result.is_some());
        let texture = result.unwrap();
        assert_eq!(texture.len(), 4 * 4 * 4); // 4x4 RGBA
    }
}