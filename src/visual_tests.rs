//! Visual Testing Infrastructure for Axiom
//!
//! This module provides golden image testing capabilities for visual effects:
//! - Headless rendering to textures
//! - Snapshot capture and comparison
//! - Pixel-perfect and fuzzy matching
//! - Test result visualization

use anyhow::{Context, Result};
use std::path::PathBuf;
use wgpu::{
    BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Device, Extent3d, ImageCopyBuffer,
    ImageDataLayout, Queue, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureView,
};

/// Configuration for visual tests
#[derive(Debug, Clone)]
pub struct VisualTestConfig {
    /// Test name for reference image naming
    pub test_name: String,
    /// Width of test render target
    pub width: u32,
    /// Height of test render target
    pub height: u32,
    /// Tolerance for fuzzy comparison (0.0 = exact, 1.0 = any difference allowed)
    pub tolerance: f32,
    /// Whether to save diff images on failure
    pub save_diffs: bool,
    /// Base directory for golden images
    pub golden_dir: PathBuf,
}

impl Default for VisualTestConfig {
    fn default() -> Self {
        Self {
            test_name: "unnamed_test".to_string(),
            width: 800,
            height: 600,
            tolerance: 0.01, // 1% tolerance by default
            save_diffs: true,
            golden_dir: PathBuf::from("tests/golden_images"),
        }
    }
}

/// Result of a visual test comparison
#[derive(Debug, Clone)]
pub struct ComparisonResult {
    /// Whether the test passed
    pub passed: bool,
    /// Difference metric (0.0 = identical, 1.0 = completely different)
    pub difference: f32,
    /// Number of pixels that differed
    pub different_pixels: usize,
    /// Total number of pixels
    pub total_pixels: usize,
    /// Path to diff image if saved
    pub diff_image_path: Option<PathBuf>,
}

/// Helper for capturing rendered frames as images
pub struct FrameCapture {
    device: std::sync::Arc<Device>,
    queue: std::sync::Arc<Queue>,
    width: u32,
    height: u32,
    format: TextureFormat,
}

impl FrameCapture {
    /// Create a new frame capture helper
    pub fn new(
        device: std::sync::Arc<Device>,
        queue: std::sync::Arc<Queue>,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            device,
            queue,
            width,
            height,
            format: TextureFormat::Rgba8UnormSrgb,
        }
    }

    /// Capture the contents of a texture to a buffer
    pub async fn capture_texture(&self, texture: &Texture) -> Result<Vec<u8>> {
        let bytes_per_pixel = 4; // RGBA8
        let unpadded_bytes_per_row = self.width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) / align * align;
        let buffer_size = (padded_bytes_per_row * self.height) as u64;

        // Create buffer to copy texture data to
        let buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("Frame Capture Buffer"),
            size: buffer_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Copy texture to buffer
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Frame Capture Encoder"),
            });

        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            ImageCopyBuffer {
                buffer: &buffer,
                layout: ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        // Read buffer data
        let buffer_slice = buffer.slice(..);
        let (tx, rx) = tokio::sync::oneshot::channel();

        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });

        self.device.poll(wgpu::Maintain::Wait);

        rx.await
            .context("Failed to receive buffer mapping result")?
            .context("Failed to map buffer")?;

        let data = buffer_slice.get_mapped_range();

        // Remove padding if necessary
        let mut result = Vec::with_capacity((self.width * self.height * bytes_per_pixel) as usize);

        if padded_bytes_per_row == unpadded_bytes_per_row {
            // No padding, direct copy
            result.extend_from_slice(&data);
        } else {
            // Remove padding from each row
            for row in 0..self.height {
                let start = (row * padded_bytes_per_row) as usize;
                let end = start + unpadded_bytes_per_row as usize;
                result.extend_from_slice(&data[start..end]);
            }
        }

        drop(data);
        buffer.unmap();

        Ok(result)
    }

    /// Create a test texture for rendering
    pub fn create_test_texture(&self) -> Texture {
        self.device.create_texture(&TextureDescriptor {
            label: Some("Visual Test Texture"),
            size: Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
}

/// Visual test runner with golden image comparison
pub struct VisualTestRunner {
    capture: FrameCapture,
    config: VisualTestConfig,
}

impl VisualTestRunner {
    /// Create a new visual test runner
    pub fn new(
        device: std::sync::Arc<Device>,
        queue: std::sync::Arc<Queue>,
        config: VisualTestConfig,
    ) -> Self {
        let capture = FrameCapture::new(device, queue, config.width, config.height);
        Self { capture, config }
    }

    /// Run a visual test by comparing captured frame to golden image
    pub async fn run_test<F>(&self, render_fn: F) -> Result<ComparisonResult>
    where
        F: FnOnce(&TextureView) -> Result<()>,
    {
        // Create test texture and render to it
        let test_texture = self.capture.create_test_texture();
        let test_view = test_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Execute render function
        render_fn(&test_view)?;

        // Capture rendered frame
        let captured_data = self.capture.capture_texture(&test_texture).await?;

        // Load golden image if it exists
        let golden_path = self.get_golden_path();

        if !golden_path.exists() {
            // No golden image exists, save current render as baseline
            log::info!(
                "📸 No golden image found for '{}', saving baseline",
                self.config.test_name
            );
            self.save_golden(&captured_data)?;

            return Ok(ComparisonResult {
                passed: true,
                difference: 0.0,
                different_pixels: 0,
                total_pixels: (self.config.width * self.config.height) as usize,
                diff_image_path: None,
            });
        }

        // Load and compare
        let golden_data = self.load_golden()?;
        let comparison = self.compare_images(&captured_data, &golden_data)?;

        // Save diff if requested and test failed
        if !comparison.passed && self.config.save_diffs {
            let diff_path = self.save_diff(&captured_data, &golden_data)?;
            log::warn!(
                "🔍 Visual test '{}' failed: {:.2}% difference, diff saved to {:?}",
                self.config.test_name,
                comparison.difference * 100.0,
                diff_path
            );
        }

        Ok(comparison)
    }

    /// Compare two image buffers
    fn compare_images(&self, captured: &[u8], golden: &[u8]) -> Result<ComparisonResult> {
        if captured.len() != golden.len() {
            anyhow::bail!(
                "Image size mismatch: captured {} bytes, golden {} bytes",
                captured.len(),
                golden.len()
            );
        }

        let total_pixels = (self.config.width * self.config.height) as usize;
        let mut different_pixels = 0;
        let mut total_difference = 0.0f32;

        // Compare pixel by pixel (RGBA format)
        for i in (0..captured.len()).step_by(4) {
            let r_diff = (captured[i] as f32 - golden[i] as f32).abs() / 255.0;
            let g_diff = (captured[i + 1] as f32 - golden[i + 1] as f32).abs() / 255.0;
            let b_diff = (captured[i + 2] as f32 - golden[i + 2] as f32).abs() / 255.0;
            let a_diff = (captured[i + 3] as f32 - golden[i + 3] as f32).abs() / 255.0;

            let pixel_diff = (r_diff + g_diff + b_diff + a_diff) / 4.0;
            total_difference += pixel_diff;

            if pixel_diff > 0.01 {
                // More than 1% difference per channel
                different_pixels += 1;
            }
        }

        let average_difference = total_difference / total_pixels as f32;
        let passed = average_difference <= self.config.tolerance;

        Ok(ComparisonResult {
            passed,
            difference: average_difference,
            different_pixels,
            total_pixels,
            diff_image_path: None,
        })
    }

    /// Get the path for the golden image
    fn get_golden_path(&self) -> PathBuf {
        self.config
            .golden_dir
            .join(format!("{}.png", self.config.test_name))
    }

    /// Save captured data as golden image
    fn save_golden(&self, data: &[u8]) -> Result<()> {
        std::fs::create_dir_all(&self.config.golden_dir)?;
        let path = self.get_golden_path();
        
        // Create parent directories if needed (for hierarchical test names)
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Use tiny-skia or image crate to save PNG
        // For now, save raw RGBA data
        let mut encoder =
            png::Encoder::new(std::fs::File::create(&path)?, self.config.width, self.config.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder.write_header()?;
        writer.write_image_data(data)?;

        log::info!("✅ Saved golden image: {:?}", path);
        Ok(())
    }

    /// Load golden image data
    fn load_golden(&self) -> Result<Vec<u8>> {
        let path = self.get_golden_path();
        let decoder = png::Decoder::new(std::fs::File::open(&path)?);
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;

        // Ensure we have RGBA data
        if info.color_type != png::ColorType::Rgba {
            anyhow::bail!("Golden image must be RGBA format");
        }

        buf.truncate(info.buffer_size());
        Ok(buf)
    }

    /// Save diff image showing differences
    fn save_diff(&self, captured: &[u8], golden: &[u8]) -> Result<PathBuf> {
        let diff_dir = self.config.golden_dir.join("diffs");
        std::fs::create_dir_all(&diff_dir)?;

        let diff_path = diff_dir.join(format!("{}_diff.png", self.config.test_name));

        // Create diff visualization (highlight differences in red)
        let mut diff_data = Vec::with_capacity(captured.len());

        for i in (0..captured.len()).step_by(4) {
            let r_diff = (captured[i] as i16 - golden[i] as i16).abs() as u8;
            let g_diff = (captured[i + 1] as i16 - golden[i + 1] as i16).abs() as u8;
            let b_diff = (captured[i + 2] as i16 - golden[i + 2] as i16).abs() as u8;

            // Highlight differences in red
            let has_diff = r_diff > 2 || g_diff > 2 || b_diff > 2;

            if has_diff {
                diff_data.extend_from_slice(&[255, 0, 0, 255]); // Red for differences
            } else {
                diff_data.extend_from_slice(&captured[i..i + 4]); // Original if same
            }
        }

        let mut encoder = png::Encoder::new(
            std::fs::File::create(&diff_path)?,
            self.config.width,
            self.config.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder.write_header()?;
        writer.write_image_data(&diff_data)?;

        Ok(diff_path)
    }

    /// Update golden image with current render (use with caution!)
    pub async fn update_golden<F>(&self, render_fn: F) -> Result<()>
    where
        F: FnOnce(&TextureView) -> Result<()>,
    {
        let test_texture = self.capture.create_test_texture();
        let test_view = test_texture.create_view(&wgpu::TextureViewDescriptor::default());

        render_fn(&test_view)?;

        let captured_data = self.capture.capture_texture(&test_texture).await?;
        self.save_golden(&captured_data)?;

        log::info!("🔄 Updated golden image for '{}'", self.config.test_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that we can create a visual test runner
    #[test]
    fn test_create_runner() {
        let config = VisualTestConfig {
            test_name: "test_runner".to_string(),
            width: 100,
            height: 100,
            ..Default::default()
        };

        // This test just verifies the structure compiles
        assert_eq!(config.width, 100);
        assert_eq!(config.height, 100);
    }

    /// Test comparison result structure
    #[test]
    fn test_comparison_result() {
        let result = ComparisonResult {
            passed: true,
            difference: 0.0,
            different_pixels: 0,
            total_pixels: 10000,
            diff_image_path: None,
        };

        assert!(result.passed);
        assert_eq!(result.difference, 0.0);
    }
}
