//! GPU-based Blur Effects Implementation
//!
//! This module implements various blur effects using GPU shaders:
//! - Gaussian blur (dual-pass for performance)
//! - Background blur (behind windows)
//! - Window content blur
//! - Bokeh blur for special effects

use anyhow::Result;
use cgmath::Vector2;
use log::{debug, info};
use std::sync::Arc;
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BlendState, Buffer, BufferDescriptor,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, Device, FragmentState,
    MultisampleState, PrimitiveState, Queue, RenderPipeline, RenderPipelineDescriptor,
    Texture, TextureFormat, TextureUsages, TextureView, VertexState,
};

use super::shaders::{ShaderManager, ShaderType};

/// Different types of blur effects
#[derive(Debug, Clone)]
pub enum BlurType {
    /// Standard Gaussian blur
    Gaussian { radius: f32, intensity: f32 },
    /// Background blur (behind transparent windows)
    Background { radius: f32, intensity: f32 },
    /// Window content blur
    Window { radius: f32, intensity: f32 },
    /// Bokeh blur with circular highlights
    Bokeh {
        radius: f32,
        intensity: f32,
        highlight_threshold: f32,
    },
}

/// Blur effect parameters
#[derive(Debug, Clone)]
pub struct BlurParams {
    pub blur_type: BlurType,
    pub enabled: bool,
    pub adaptive_quality: bool,
    pub performance_scale: f32, // 0.5 to 1.0
}

/// GPU-based blur renderer
pub struct BlurRenderer {
    device: Arc<Device>,
    queue: Arc<Queue>,
    shader_manager: Arc<ShaderManager>,

    // Render pipelines for different blur passes
    horizontal_blur_pipeline: Option<RenderPipeline>,
    vertical_blur_pipeline: Option<RenderPipeline>,

    // Uniform buffers
    blur_params_buffer: Buffer,

    // Intermediate textures for dual-pass blur
    intermediate_texture: Option<Texture>,
    intermediate_texture_view: Option<TextureView>,

    // Current blur parameters
    current_params: BlurParams,

    // Performance tracking
    last_blur_time: std::time::Duration,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    radius: f32,
    intensity: f32,
    direction: [f32; 2],
    texture_size: [f32; 2],
}

impl BlurRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        shader_manager: Arc<ShaderManager>,
        initial_params: BlurParams,
    ) -> Result<Self> {
        info!("🌊 Initializing GPU Blur Renderer...");

        // Create uniform buffer for blur parameters
        let blur_params_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("Blur Parameters Buffer"),
            size: std::mem::size_of::<BlurUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut renderer = Self {
            device,
            queue,
            shader_manager,
            horizontal_blur_pipeline: None,
            vertical_blur_pipeline: None,
            blur_params_buffer,
            intermediate_texture: None,
            intermediate_texture_view: None,
            current_params: initial_params,
            last_blur_time: std::time::Duration::from_millis(0),
        };

        // Initialize render pipelines
        renderer.create_blur_pipelines()?;

        info!("✅ Blur Renderer initialized successfully");
        Ok(renderer)
    }

    /// Create render pipelines for blur effects
    fn create_blur_pipelines(&mut self) -> Result<()> {
        debug!("🔧 Creating blur render pipelines...");

        // Get compiled shaders
        let horizontal_shader = self
            .shader_manager
            .get_shader(&ShaderType::BlurHorizontal)
            .ok_or_else(|| anyhow::anyhow!("Horizontal blur shader not found"))?;
        let vertical_shader = self
            .shader_manager
            .get_shader(&ShaderType::BlurVertical)
            .ok_or_else(|| anyhow::anyhow!("Vertical blur shader not found"))?;

        // Create bind group layout for blur uniforms and textures
        let bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Blur Bind Group Layout"),
                    entries: &[
                        // Blur uniforms
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // Input texture
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        // Texture sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blur Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        // Horizontal blur pipeline
        self.horizontal_blur_pipeline = Some(self.device.create_render_pipeline(
            &RenderPipelineDescriptor {
                label: Some("Horizontal Blur Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: horizontal_shader,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                fragment: Some(FragmentState {
                    module: horizontal_shader,
                    entry_point: "fs_main",
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Bgra8UnormSrgb,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview: None,
            },
        ));

        // Vertical blur pipeline
        self.vertical_blur_pipeline = Some(self.device.create_render_pipeline(
            &RenderPipelineDescriptor {
                label: Some("Vertical Blur Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: vertical_shader,
                    entry_point: "vs_main",
                    buffers: &[],
                },
                fragment: Some(FragmentState {
                    module: vertical_shader,
                    entry_point: "fs_main",
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Bgra8UnormSrgb,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview: None,
            },
        ));

        debug!("✅ Blur pipelines created successfully");
        Ok(())
    }

    /// Apply blur effect to a texture
    pub fn apply_blur(
        &mut self,
        encoder: &mut CommandEncoder,
        input_texture: &TextureView,
        output_texture: &TextureView,
        texture_size: Vector2<u32>,
    ) -> Result<()> {
        let start_time = std::time::Instant::now();

        // Get blur parameters based on current settings
        let (radius, intensity) = match &self.current_params.blur_type {
            BlurType::Gaussian { radius, intensity } => (*radius, *intensity),
            BlurType::Background { radius, intensity } => (*radius * 0.8, *intensity), // Slightly less intense
            BlurType::Window { radius, intensity } => (*radius, *intensity),
            BlurType::Bokeh {
                radius, intensity, ..
            } => (*radius * 1.2, *intensity), // More intense for bokeh
        };

        // Apply performance scaling
        let effective_radius = radius * self.current_params.performance_scale;
        let effective_intensity = intensity * self.current_params.performance_scale;

        // Ensure we have intermediate texture for dual-pass blur
        self.ensure_intermediate_texture(texture_size)?;

        // First pass: Horizontal blur (input -> intermediate)
        self.apply_horizontal_blur(
            encoder,
            input_texture,
            self.intermediate_texture_view.as_ref().unwrap(),
            texture_size,
            effective_radius,
            effective_intensity,
        )?;

        // Second pass: Vertical blur (intermediate -> output)
        self.apply_vertical_blur(
            encoder,
            self.intermediate_texture_view.as_ref().unwrap(),
            output_texture,
            texture_size,
            effective_radius,
            effective_intensity,
        )?;

        self.last_blur_time = start_time.elapsed();

        debug!(
            "🌊 Applied blur effect: radius={:.1}, intensity={:.1}, time={:.2}ms",
            effective_radius,
            effective_intensity,
            self.last_blur_time.as_secs_f64() * 1000.0
        );

        Ok(())
    }

    /// Apply horizontal blur pass
    fn apply_horizontal_blur(
        &self,
        encoder: &mut CommandEncoder,
        input_texture: &TextureView,
        output_texture: &TextureView,
        texture_size: Vector2<u32>,
        radius: f32,
        intensity: f32,
    ) -> Result<()> {
        // Update uniform buffer with horizontal direction
        let uniforms = BlurUniforms {
            radius,
            intensity,
            direction: [1.0, 0.0], // Horizontal
            texture_size: [texture_size.x as f32, texture_size.y as f32],
        };

        self.queue.write_buffer(
            &self.blur_params_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );

        // Create bind group for this pass
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("Horizontal Blur Bind Group"),
            layout: &self
                .horizontal_blur_pipeline
                .as_ref()
                .unwrap()
                .get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.blur_params_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(input_texture),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.create_blur_sampler()),
                },
            ],
        });

        // Render pass
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizontal Blur Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_texture,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(self.horizontal_blur_pipeline.as_ref().unwrap());
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1); // Full-screen triangle

        Ok(())
    }

    /// Apply vertical blur pass
    fn apply_vertical_blur(
        &self,
        encoder: &mut CommandEncoder,
        input_texture: &TextureView,
        output_texture: &TextureView,
        texture_size: Vector2<u32>,
        radius: f32,
        intensity: f32,
    ) -> Result<()> {
        // Update uniform buffer with vertical direction
        let uniforms = BlurUniforms {
            radius,
            intensity,
            direction: [0.0, 1.0], // Vertical
            texture_size: [texture_size.x as f32, texture_size.y as f32],
        };

        self.queue.write_buffer(
            &self.blur_params_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );

        // Create bind group for this pass
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("Vertical Blur Bind Group"),
            layout: &self
                .vertical_blur_pipeline
                .as_ref()
                .unwrap()
                .get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.blur_params_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(input_texture),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.create_blur_sampler()),
                },
            ],
        });

        // Render pass
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Vertical Blur Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_texture,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(self.vertical_blur_pipeline.as_ref().unwrap());
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1); // Full-screen triangle

        Ok(())
    }

    /// Ensure intermediate texture exists and is the correct size
    fn ensure_intermediate_texture(&mut self, size: Vector2<u32>) -> Result<()> {
        // Check if we need to create or recreate the intermediate texture
        let needs_recreation = self
            .intermediate_texture
            .as_ref()
            .map(|texture| texture.width() != size.x || texture.height() != size.y)
            .unwrap_or(true);

        if needs_recreation {
            debug!(
                "🔄 Creating intermediate blur texture: {}x{}",
                size.x, size.y
            );

            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Blur Intermediate Texture"),
                size: wgpu::Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::Bgra8UnormSrgb,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            self.intermediate_texture = Some(texture);
            self.intermediate_texture_view = Some(texture_view);
        }

        Ok(())
    }

    /// Create sampler for blur effects
    fn create_blur_sampler(&self) -> wgpu::Sampler {
        self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Blur Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        })
    }

    /// Update blur parameters
    pub fn update_blur_params(&mut self, new_params: BlurParams) {
        if !self.params_equal(&self.current_params, &new_params) {
            debug!("🔄 Updating blur parameters: {:?}", new_params.blur_type);
            self.current_params = new_params;
        }
    }

    /// Check if blur parameters are equal (for change detection)
    fn params_equal(&self, a: &BlurParams, b: &BlurParams) -> bool {
        a.enabled == b.enabled
            && a.adaptive_quality == b.adaptive_quality
            && (a.performance_scale - b.performance_scale).abs() < 0.01
            && match (&a.blur_type, &b.blur_type) {
                (
                    BlurType::Gaussian {
                        radius: r1,
                        intensity: i1,
                    },
                    BlurType::Gaussian {
                        radius: r2,
                        intensity: i2,
                    },
                ) => (r1 - r2).abs() < 0.1 && (i1 - i2).abs() < 0.01,
                (
                    BlurType::Background {
                        radius: r1,
                        intensity: i1,
                    },
                    BlurType::Background {
                        radius: r2,
                        intensity: i2,
                    },
                ) => (r1 - r2).abs() < 0.1 && (i1 - i2).abs() < 0.01,
                _ => false,
            }
    }

    /// Get performance statistics
    pub fn get_performance_stats(&self) -> (std::time::Duration, f32) {
        (self.last_blur_time, self.current_params.performance_scale)
    }

    /// Enable or disable adaptive quality based on performance
    pub fn set_adaptive_quality(&mut self, enabled: bool) {
        self.current_params.adaptive_quality = enabled;
        if enabled {
            info!("🎛️ Adaptive blur quality enabled");
        } else {
            info!("🎛️ Adaptive blur quality disabled");
        }
    }

    /// Update performance scale (called by effects engine based on frame time)
    pub fn update_performance_scale(&mut self, scale: f32) {
        let new_scale = scale.clamp(0.3, 1.0);
        if (self.current_params.performance_scale - new_scale).abs() > 0.05 {
            debug!(
                "⚡ Updated blur performance scale: {:.2} -> {:.2}",
                self.current_params.performance_scale, new_scale
            );
            self.current_params.performance_scale = new_scale;
        }
    }
}
