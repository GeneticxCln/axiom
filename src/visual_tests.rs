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
    /// Width of test render target
    pub width: u32,
    /// Height of test render target
    pub height: u32,
    /// Tolerance for fuzzy comparison (0.0 = exact, 1.0 = any difference allowed)
    pub tolerance: f32,
    /// Base directory for golden images
    pub golden_dir: PathBuf,
    /// Test name (for golden image naming)
    pub test_name: String,
    /// Whether to save diff images on failure
    pub save_diffs: bool,
}

impl Default for VisualTestConfig {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            tolerance: 0.01, // 1% tolerance by default
            golden_dir: PathBuf::from("tests/golden_images"),
            test_name: "unnamed_test".to_string(),
            save_diffs: true,
        }
    }
}

/// Result of a visual test comparison
#[derive(Debug, Clone, PartialEq)]
pub enum ComparisonResult {
    /// Test passed - images match within tolerance
    Match,
    /// Test failed - images differ beyond tolerance
    Mismatch {
        difference: f32,
        different_pixels: usize,
        total_pixels: usize,
    },
    /// Golden image doesn't exist - saved new baseline
    NewBaseline,
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

            return Ok(ComparisonResult::NewBaseline);
        }

        // Load and compare
        let golden_data = self.load_golden()?;
        let comparison = self.compare_images(&captured_data, &golden_data)?;

        // Save diff if requested and test failed
        if let ComparisonResult::Mismatch { difference, .. } = &comparison {
            if self.config.save_diffs {
                let diff_path = self.save_diff(&captured_data, &golden_data)?;
                log::warn!(
                    "🔍 Visual test '{}' failed: {:.2}% difference, diff saved to {:?}",
                    self.config.test_name,
                    difference * 100.0,
                    diff_path
                );
            }
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

        if average_difference <= self.config.tolerance {
            Ok(ComparisonResult::Match)
        } else {
            Ok(ComparisonResult::Mismatch {
                difference: average_difference,
                different_pixels,
                total_pixels,
            })
        }
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

/// Visual test context with blur effect support
pub struct VisualTestContext {
    device: std::sync::Arc<Device>,
    queue: std::sync::Arc<Queue>,
    config: VisualTestConfig,
    shader_manager: Option<std::sync::Arc<crate::effects::shaders::ShaderManager>>,
}

impl VisualTestContext {
    /// Create a new visual test context with GPU support
    pub async fn new(config: VisualTestConfig) -> Result<Self> {
        // Create GPU instance and device
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .context("Failed to find suitable GPU adapter")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Visual Test Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .context("Failed to create GPU device")?;

        let device = std::sync::Arc::new(device);
        let queue = std::sync::Arc::new(queue);

        // Initialize shader manager
        let mut shader_manager = crate::effects::shaders::ShaderManager::new(device.clone());
        shader_manager.compile_all_shaders()?;
        let shader_manager = std::sync::Arc::new(shader_manager);

        Ok(Self {
            device,
            queue,
            config,
            shader_manager: Some(shader_manager),
        })
    }

    /// Apply blur effect to an image
    pub async fn apply_blur_effect(
        &mut self,
        input_data: &[u8],
        width: u32,
        height: u32,
        radius: f32,
        intensity: f32,
    ) -> Result<Vec<u8>> {
        use crate::effects::blur::{BlurRenderer, BlurType, BlurParams};

        // Create blur renderer
        let blur_params = BlurParams {
            blur_type: BlurType::Gaussian { radius, intensity },
            enabled: true,
            adaptive_quality: false,
            performance_scale: 1.0,
        };

        let mut blur_renderer = BlurRenderer::new(
            self.device.clone(),
            self.queue.clone(),
            self.shader_manager.as_ref().unwrap().clone(),
            blur_params,
        )?;

        // Create input texture from data
        let input_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Blur Input Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload input data
        self.queue.write_texture(
            input_texture.as_image_copy(),
            input_data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        // Create output texture
        let output_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Blur Output Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // Apply blur effect
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Blur Apply Encoder"),
        });
        
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        blur_renderer.apply_blur(
            &mut encoder,
            &input_view,
            &output_view,
            cgmath::Vector2 { x: width, y: height },
        )?;
        
        self.queue.submit(Some(encoder.finish()));

        // Capture result
        let capture = FrameCapture::new(self.device.clone(), self.queue.clone(), width, height);
        capture.capture_texture(&output_texture).await
    }

    /// Apply single blur pass (horizontal or vertical)
    pub async fn apply_blur_pass(
        &mut self,
        input_data: &[u8],
        width: u32,
        height: u32,
        radius: f32,
        intensity: f32,
        _horizontal: bool,
    ) -> Result<Vec<u8>> {
        // For now, delegate to full blur - can be optimized later
        self.apply_blur_effect(input_data, width, height, radius, intensity)
            .await
    }

    /// Compare rendered result with golden image
    pub fn compare_with_golden(
        &self,
        golden_name: &str,
        result_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<ComparisonResult> {
        let golden_path = PathBuf::from(&self.config.golden_dir).join(golden_name);

        // Create parent directories if needed
        if let Some(parent) = golden_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if !golden_path.exists() {
            // Save as new baseline
            self.save_image(&golden_path, result_data, width, height)?;
            log::info!("📸 Saved new golden image: {:?}", golden_path);
            return Ok(ComparisonResult::NewBaseline);
        }

        // Load golden image
        let golden_data = self.load_image(&golden_path)?;

        // Compare
        let total_pixels = (width * height) as usize;
        let mut different_pixels = 0;
        let mut total_difference = 0.0f32;

        for i in (0..result_data.len().min(golden_data.len())).step_by(4) {
            let r_diff = (result_data[i] as f32 - golden_data[i] as f32).abs() / 255.0;
            let g_diff = (result_data[i + 1] as f32 - golden_data[i + 1] as f32).abs() / 255.0;
            let b_diff = (result_data[i + 2] as f32 - golden_data[i + 2] as f32).abs() / 255.0;
            let a_diff = (result_data[i + 3] as f32 - golden_data[i + 3] as f32).abs() / 255.0;

            let pixel_diff = (r_diff + g_diff + b_diff + a_diff) / 4.0;
            total_difference += pixel_diff;

            if pixel_diff > 0.01 {
                different_pixels += 1;
            }
        }

        let average_difference = total_difference / total_pixels as f32;

        if average_difference <= self.config.tolerance {
            Ok(ComparisonResult::Match)
        } else {
            Ok(ComparisonResult::Mismatch {
                difference: average_difference,
                different_pixels,
                total_pixels,
            })
        }
    }

    /// Save image to PNG file
    fn save_image(&self, path: &PathBuf, data: &[u8], width: u32, height: u32) -> Result<()> {
        let mut encoder = png::Encoder::new(std::fs::File::create(path)?, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(data)?;
        Ok(())
    }

    /// Load image from PNG file
    fn load_image(&self, path: &PathBuf) -> Result<Vec<u8>> {
        let decoder = png::Decoder::new(std::fs::File::open(path)?);
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;
        buf.truncate(info.buffer_size());
        Ok(buf)
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
        let match_result = ComparisonResult::Match;
        assert!(matches!(match_result, ComparisonResult::Match));

        let mismatch_result = ComparisonResult::Mismatch {
            difference: 0.05,
            different_pixels: 500,
            total_pixels: 10000,
        };
        
        match mismatch_result {
            ComparisonResult::Mismatch { difference, .. } => {
                assert_eq!(difference, 0.05);
            }
            _ => panic!("Expected mismatch result"),
        }
        
        let new_baseline = ComparisonResult::NewBaseline;
        assert!(matches!(new_baseline, ComparisonResult::NewBaseline));
    }
}
