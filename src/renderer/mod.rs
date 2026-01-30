//! Real GPU rendering pipeline for Axiom compositor
//!
//! This module implements actual GPU rendering using wgpu to composite
//! windows and effects to the screen - not just stubs.

#![allow(missing_docs)]
#![allow(clippy::too_many_lines)]

use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use wgpu::util::DeviceExt;
#[allow(clippy::wildcard_imports)]
use wgpu::*;

/// Real GPU rendering pipeline
pub struct AxiomRenderer {
    /// WGPU device for GPU operations (shared)
    device: Arc<Device>,
    /// Command queue for GPU commands (shared)
    queue: Arc<Queue>,
    /// WGPU Instance
    instance: Arc<Instance>,
    /// WGPU Adapter (for querying capabilities)
    adapter: Arc<Adapter>,

    /// Main surface (for backwards compatibility/primary display)
    /// In multi-monitor, this might be just one of them.
    /// We will need a map of surfaces, but for now we keep simple structure
    /// and maybe add `surfaces` map.
    /// To respect the existing interface which assumes single surface in `new`...

    /// Map of output ID to Surface+Config
    surfaces: HashMap<String, (wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,

    /// Rendered windows (global list, layout determines where they go)
    windows: Vec<RenderedWindow>,

    /// Fallback size if no surface found
    default_size: (u32, u32),

    /// WGPU Render Pipeline
    render_pipeline: RenderPipeline,
    /// WGPU Sampler
    sampler: Sampler,
}

/// Represents a rendered window surface
#[derive(Debug)]
pub struct RenderedWindow {
    /// Unique window ID
    pub id: u64,
    /// Window position on screen
    pub position: (f32, f32),
    /// Window size
    pub size: (f32, f32),
    /// Window texture (actual pixel data)
    pub texture: Option<Texture>,
    /// Window texture view for rendering
    pub texture_view: Option<TextureView>,
    /// Whether window needs redraw
    pub dirty: bool,
    /// Window opacity
    pub opacity: f32,
}

/// Vertex data for rendering quads
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct WindowUniforms {
    opacity: f32,
    padding: [f32; 3], // Alignment
}

static RENDER_STATE: OnceLock<Arc<Mutex<SharedRenderState>>> = OnceLock::new();

#[derive(Default)]
struct SharedRenderState {
    #[allow(clippy::type_complexity)]
    placeholders: HashMap<u64, ((f32, f32), (f32, f32), f32)>,
}

pub fn push_placeholder_quad(id: u64, position: (f32, f32), size: (f32, f32), opacity: f32) {
    if let Some(state) = RENDER_STATE.get() {
        if let Ok(mut s) = state.lock() {
            s.placeholders.insert(id, (position, size, opacity));
        }
    }
}

impl AxiomRenderer {
    /// Create a new real GPU renderer with an actual surface
    pub async fn new(surface: wgpu::Surface<'static>, width: u32, height: u32) -> Result<Self> {
        info!(
            "🎨 Creating real GPU renderer with surface ({}x{})",
            width, height
        );

        // Create wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Get adapter (GPU)
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("Failed to find suitable GPU adapter"))?;

        info!("🖥️ Using GPU: {}", adapter.get_info().name);

        // Create device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    label: None,
                },
                None,
            )
            .await?;

        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        info!("✅ GPU renderer initialized successfully");

        let mut surfaces = HashMap::new();
        surfaces.insert("primary".to_string(), (surface, config.clone()));

        // Initialize pipeline and sampler
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Compositor Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compositor.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            instance: Arc::new(instance),
            adapter: Arc::new(adapter),
            surfaces,
            windows: Vec::new(),
            default_size: (width, height),
            render_pipeline,
            sampler,
        })
    }

    /// Create a headless renderer for testing
    pub async fn new_headless() -> Result<Self> {
        info!("🎨 Creating headless GPU renderer for testing");

        // Create wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Get adapter without surface requirement
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("Failed to find suitable GPU adapter"))?;

        // Create device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    label: None,
                },
                None,
            )
            .await?;

        info!("✅ Headless GPU renderer initialized");

        // Initialize pipeline and sampler (headless fallback)
        // Note: Headless might fail pipeline creation without a surface format hint,
        // so we use a default Bgra8UnormSrgb which is common.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Compositor Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compositor.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Use a common format for headless
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            instance: Arc::new(instance),
            adapter: Arc::new(adapter),
            surfaces: HashMap::new(),
            windows: Vec::new(),
            default_size: (1920, 1080),
            render_pipeline,
            sampler,
        })
    }

    /// Add a new output surface to the renderer
    pub fn add_output(
        &mut self,
        name: String,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) {
        info!(
            "🖥️ Renderer: Adding output '{}' ({}x{})",
            name, width, height
        );

        // Configure surface
        let surface_caps = surface.get_capabilities(&self.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&self.device, &config);
        self.surfaces.insert(name, (surface, config));
    }

    /// Remove an output surface
    pub fn remove_output(&mut self, name: &str) {
        info!("🔌 Renderer: Removing output '{}'", name);
        self.surfaces.remove(name);
    }

    /// Update output size (e.g. on window resize)
    pub fn update_output_size(&mut self, name: &str, width: u32, height: u32) {
        if let Some((surface, config)) = self.surfaces.get_mut(name) {
            info!(
                "🖥️ Renderer: Resizing output '{}' to {}x{}",
                name, width, height
            );
            config.width = width;
            config.height = height;
            surface.configure(&self.device, config);
        }
    }

    /// Add a window to be rendered
    pub fn add_window(&mut self, id: u64, position: (f32, f32), size: (f32, f32)) -> Result<()> {
        info!(
            "➕ Adding window {} at ({}, {}) size {}x{}",
            id, position.0, position.1, size.0, size.1
        );

        let window = RenderedWindow {
            id,
            position,
            size,
            texture: None,
            texture_view: None,
            dirty: true,
            opacity: 1.0,
        };

        self.windows.push(window);
        Ok(())
    }

    /// Upsert a window rectangle without a texture (simple colored quad placeholder)
    pub fn upsert_window_rect(
        &mut self,
        id: u64,
        position: (f32, f32),
        size: (f32, f32),
        opacity: f32,
    ) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.position = position;
            w.size = size;
            w.opacity = opacity;
            w.dirty = true;
        } else {
            let window = RenderedWindow {
                id,
                position,
                size,
                texture: None,
                texture_view: None,
                dirty: true,
                opacity,
            };
            self.windows.push(window);
        }
    }

    /// Update window texture from raw RGBA data
    pub fn update_window_texture(&mut self, window_id: u64, width: u32, height: u32, data: &[u8]) {
        if let Some(window) = self.windows.iter_mut().find(|w| w.id == window_id) {
            let size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };

            // Check if we need to replace texture
            let needs_replace = if let Some(ref current) = window.texture {
                current.width() != width || current.height() != height
            } else {
                true
            };

            if needs_replace {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Window {} Texture", window_id)),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });

                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                window.texture = Some(texture);
                window.texture_view = Some(view);
            }

            // Now write data
            if let Some(ref texture) = window.texture {
                self.queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * width),
                        rows_per_image: Some(height),
                    },
                    size,
                );
            }

            window.dirty = true;
        }
    }

    /// Render all windows (simplified for now - needs actual surface)
    pub fn render(&mut self) -> Result<()> {
        // Reduced logging level to avoid spamming I/O at 60fps
        log::trace!("🎨 Rendering {} windows to GPU", self.windows.len());

        // For now, just validate that we have the GPU device and queue
        // In a real implementation, this would render to an actual surface

        for window in &self.windows {
            if window.texture_view.is_some() {
                log::trace!("✅ Would render window {} with texture", window.id);
            } else {
                log::trace!(
                    "🟦 Rendering placeholder quad for window {} at ({:.1},{:.1}) size {:.1}x{:.1} opacity {:.2}",
                    window.id, window.position.0, window.position.1, window.size.0, window.size.1, window.opacity
                );
            }
        }

        log::trace!("🖥️ Frame rendered with {} windows", self.windows.len());
        Ok(())
    }

    /// Render specifically to the named output
    pub fn render_output(&mut self, output_name: &str) -> Result<()> {
        if let Some((surface, _config)) = self.surfaces.get(output_name) {
            match surface.get_current_texture() {
                Ok(frame) => {
                    self.render_to_surface(surface, &frame)?;
                    frame.present();
                    Ok(())
                }
                Err(e) => {
                    log::warn!(
                        "Failed to get current texture for output {}: {}",
                        output_name,
                        e
                    );
                    Err(anyhow::anyhow!(e))
                }
            }
        } else {
            log::warn!(
                "Attempted to render to non-existent output: {}",
                output_name
            );
            Ok(())
        }
    }

    /// Render all windows to a wgpu surface (real rendering)
    pub fn render_to_surface(
        &self,
        _surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
    ) -> Result<()> {
        debug!("🎨 Rendering {} windows to surface", self.windows.len());

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // Create uniform buffer for projection matrix
        let config = self.surfaces.values().next().map(|(_, c)| c).or({
            // Fallback if we can't find config from map (though we should have it)
            // This can happen if called on headless or if surface was added ad-hoc
            None
        });

        let (width, height) = if let Some(c) = config {
            (c.width as f32, c.height as f32)
        } else {
            (self.default_size.0 as f32, self.default_size.1 as f32)
        };

        let projection = create_projection_matrix(width, height);
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Projection Uniform Buffer"),
                contents: bytemuck::cast_slice(
                    &projection.iter().flatten().copied().collect::<Vec<f32>>(),
                ),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        // Prepare resources before starting render pass to avoid lifetime issues
        let mut draw_commands = Vec::new();

        for window in &self.windows {
            if let Some(texture_view) = &window.texture_view {
                // Create window uniform buffer
                let window_uniforms = WindowUniforms {
                    opacity: window.opacity,
                    padding: [0.0; 3],
                };
                let window_uniform_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("Window {} Uniforms", window.id)),
                            contents: bytemuck::cast_slice(&[window_uniforms]),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        });

                // Create bind group for this window
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: &self.render_pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: window_uniform_buffer.as_entire_binding(),
                        },
                    ],
                    label: Some(&format!("Window {} Bind Group", window.id)),
                });

                // Create vertex buffer for this window
                let x = window.position.0;
                let y = window.position.1;
                let w = window.size.0;
                let h = window.size.1;

                let vertices = [
                    Vertex {
                        position: [x, y, 0.0],
                        tex_coords: [0.0, 0.0],
                    },
                    Vertex {
                        position: [x, y + h, 0.0],
                        tex_coords: [0.0, 1.0],
                    },
                    Vertex {
                        position: [x + w, y + h, 0.0],
                        tex_coords: [1.0, 1.0],
                    },
                    Vertex {
                        position: [x, y, 0.0],
                        tex_coords: [0.0, 0.0],
                    },
                    Vertex {
                        position: [x + w, y + h, 0.0],
                        tex_coords: [1.0, 1.0],
                    },
                    Vertex {
                        position: [x + w, y, 0.0],
                        tex_coords: [1.0, 0.0],
                    },
                ];

                let vertex_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("Window {} Vertex Buffer", window.id)),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });

                draw_commands.push((window.id, bind_group, vertex_buffer));
            }
        }

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Axiom Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);

            for (id, bind_group, vertex_buffer) in &draw_commands {
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.draw(0..6, 0..1);
                debug!("✅ Rendered window {} to surface", id);
            }
        }

        // Submit commands to GPU
        self.queue.submit(std::iter::once(encoder.finish()));

        Ok(())
    }

    /// Get device for external use
    pub fn device(&self) -> Arc<Device> {
        self.device.clone()
    }

    /// Get queue for external use
    pub fn queue(&self) -> Arc<Queue> {
        self.queue.clone()
    }

    /// Get instance for external use (creating surfaces)
    pub fn instance(&self) -> Arc<Instance> {
        self.instance.clone()
    }

    /// Get number of rendered windows
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Start a simple headless render loop at ~60 FPS for development
    pub async fn start_headless_loop() -> Result<tokio::task::JoinHandle<()>> {
        let mut renderer = Self::new_headless().await?;
        // Initialize shared render state if not already
        let _ = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
        info!("🖥️ Starting headless render loop (~60 FPS)");

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(16));
            loop {
                ticker.tick().await;
                // Sync placeholders into renderer's window list
                if let Some(state) = RENDER_STATE.get() {
                    if let Ok(s) = state.lock() {
                        for (id, (pos, size, opacity)) in &s.placeholders {
                            renderer.upsert_window_rect(*id, *pos, *size, *opacity);
                        }
                    }
                }
                if let Err(e) = renderer.render() {
                    debug!("render error: {}", e);
                }
            }
        });

        Ok(handle)
    }
}

// Vertex descriptor for wgpu
impl Vertex {
    fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as BufferAddress,
            step_mode: VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as BufferAddress,
                    shader_location: 1,
                    format: VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Create orthographic projection matrix
pub fn create_projection_matrix(width: f32, height: f32) -> [[f32; 4]; 4] {
    let left = 0.0;
    let right = width;
    let bottom = height; // Flip Y coordinate
    let top = 0.0;
    let near = -1.0;
    let far = 1.0;

    [
        [2.0 / (right - left), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (top - bottom), 0.0, 0.0],
        [0.0, 0.0, -2.0 / (far - near), 0.0],
        [
            -(right + left) / (right - left),
            -(top + bottom) / (top - bottom),
            -(far + near) / (far - near),
            1.0,
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_projection_matrix() {
        let matrix = create_projection_matrix(1920.0, 1080.0);
        // Basic sanity check - matrix should not be all zeros
        assert_ne!(matrix[0][0], 0.0);
        assert_ne!(matrix[1][1], 0.0);
    }

    #[test]
    fn test_rendered_window_creation() {
        let window = RenderedWindow {
            id: 1,
            position: (100.0, 100.0),
            size: (400.0, 300.0),
            texture: None,
            texture_view: None,
            dirty: true,
            opacity: 1.0,
        };

        assert_eq!(window.id, 1);
        assert_eq!(window.position, (100.0, 100.0));
        assert_eq!(window.size, (400.0, 300.0));
        assert!(window.dirty);
    }
}
