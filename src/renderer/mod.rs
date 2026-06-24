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

// Type aliases for renderer effect queues reduce type complexity.
type ShadowQueue = HashMap<u64, ((f32, f32), (f32, f32), crate::effects::ShadowParams)>;
type BlurQueue = HashMap<u64, ((f32, f32), (f32, f32), crate::effects::BlurParams)>;
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

    /// Optional effects engine for GPU post-processing (shadows, blur).
    /// When set, shadow passes are auto-applied during surface rendering.
    effects_engine: Option<Arc<parking_lot::RwLock<crate::effects::EffectsEngine>>>,

    /// Map of output ID to Surface+Config
    surfaces: HashMap<String, (wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,

    /// Rendered windows (global list, layout determines where they go)
    windows: Vec<RenderedWindow>,

    /// Per-frame shadow queue: window_id -> (position, size, shadow_params)
    /// Populated by render_frame() each tick, consumed by surface rendering passes.
    window_shadows: ShadowQueue,

    /// Per-frame blur queue: window_id -> (position, size, blur_params)
    /// Populated by render_frame() each tick, consumed by surface rendering passes.
    window_blurs: BlurQueue,

    /// Fallback size if no surface found
    default_size: (u32, u32),

    /// Headless output texture + view for off-screen shadow/blur passes.
    /// Created lazily and resized when dimensions change. Used by render()
    /// for GPU effects compositing when no surface is attached.
    headless_target: Option<(Texture, TextureView)>,

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
            effects_engine: None,
            surfaces,
            windows: Vec::new(),
            window_shadows: HashMap::with_capacity(64),
            window_blurs: HashMap::with_capacity(64),
            default_size: (width, height),
            headless_target: None,
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
            effects_engine: None,
            surfaces: HashMap::new(),
            windows: Vec::new(),
            window_shadows: HashMap::with_capacity(64),
            window_blurs: HashMap::with_capacity(64),
            default_size: (1920, 1080),
            headless_target: None,
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

    /// Render all windows and apply post-processing effects (shadows, blur).
    ///
    /// Processes the per-frame shadow and blur queues that were populated by
    /// render_frame(). Uses the headless target texture for GPU compositing
    /// when no surface is attached. The queues are consumed on each call.
    pub fn render(&mut self) -> Result<()> {
        use cgmath::Vector2;

        let window_count = self.windows.len();
        let shadow_count = self.window_shadows.len();
        let blur_count = self.window_blurs.len();

        if shadow_count > 0 || blur_count > 0 {
            debug!(
                "🎨 Rendering {} windows + {} shadows + {} blurs to GPU",
                window_count, shadow_count, blur_count
            );
        } else {
            log::trace!("🎨 Rendering {} windows to GPU", window_count);
        }

        let has_work = shadow_count > 0 || blur_count > 0;
        if !has_work {
            return Ok(());
        }

        // Create or reuse headless target for off-screen effects compositing.
        // Shadows and blurs are rendered into this texture; the result can
        // later be sampled by the GL pass for full-screen compositing.
        let (w, h) = self.default_size;
        let target_view =
            ensure_headless_target(&self.device, &mut self.headless_target, w, h);

        let Some(ref effects_engine) = self.effects_engine else {
            // No effects engine wired yet — clear queues and bail
            self.window_shadows.clear();
            self.window_blurs.clear();
            return Ok(());
        };

        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("Headless Effects Encoder"),
            },
        );

        // Dispatch shadow passes from the per-frame queue
        if shadow_count > 0 {
            let shadow_data: Vec<_> = self
                .window_shadows
                .values()
                .map(|((px, py), (sx, sy), params)| {
                    (
                        Vector2::new(*px, *py),
                        Vector2::new(*sx, *sy),
                        params.clone(),
                    )
                })
                .collect();

            if let Err(e) =
                effects_engine.write().render_shadows(&mut encoder, target_view, &shadow_data)
            {
                log::warn!("⚠️ Headless shadow render failed: {}", e);
            }
        }

        // Dispatch blur passes from the per-frame queue
        if blur_count > 0 {
            let tex_size = cgmath::Vector2::new(w, h);
            if let Err(e) =
                effects_engine.write().render_blurs(&mut encoder, target_view, target_view, tex_size)
            {
                log::warn!("⚠️ Headless blur render failed: {}", e);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Consume per-frame effect queues
        self.window_shadows.clear();
        self.window_blurs.clear();

        log::trace!("🖥️ Headless frame rendered with {} windows + {} shadows + {} blurs", window_count, shadow_count, blur_count);
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

    /// Render to a surface with optional shadow post-processing.
    /// After compositing windows, invokes the provided shadow callback
    /// with a fresh encoder so shadows are drawn on top.
    pub fn render_to_surface_with_shadows(
        &self,
        surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
        on_shadows: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView) -> Result<()>,
    ) -> Result<()> {
        // Composite windows first
        self.render_to_surface(surface, surface_texture)?;

        // Run shadow pass as a separate draw batch
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut shadow_encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Shadow Encoder"),
            });
        on_shadows(&mut shadow_encoder, &view)?;
        self.queue
            .submit(std::iter::once(shadow_encoder.finish()));
        Ok(())
    }

    /// Render to a surface and auto-dispatch shadows + blurs from internal queues.
    /// This is the preferred surface rendering path — it composites windows,
    /// then applies GPU shadow and blur post-processing from the per-frame
    /// queues populated by render_frame(), all in a single encoder.
    pub fn render_to_surface_auto(
        &mut self,
        surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
    ) -> Result<()> {
        use cgmath::Vector2;

        // Composite windows first
        self.render_to_surface(surface, surface_texture)?;

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let has_shadows = !self.window_shadows.is_empty();
        let has_blurs = !self.window_blurs.is_empty();

        if has_shadows || has_blurs {
            let mut encoder = self.device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor {
                    label: Some("Effects Post-Process Encoder"),
                },
            );

            // Dispatch shadow passes from internal queue
            if has_shadows {
                if let Some(ref effects_engine) = self.effects_engine {
                    let shadow_data: Vec<_> = self
                        .window_shadows
                        .values()
                        .map(|((px, py), (sx, sy), params)| {
                            (
                                Vector2::new(*px, *py),
                                Vector2::new(*sx, *sy),
                                params.clone(),
                            )
                        })
                        .collect();

                    if !shadow_data.is_empty() {
                        if let Err(e) = effects_engine.write().render_shadows(
                            &mut encoder,
                            &view,
                            &shadow_data,
                        ) {
                            log::warn!("⚠️ Surface shadow render failed: {}", e);
                        }
                    }
                }
            }

            // Dispatch blur passes from internal queue via GPU BlurRenderer
            if has_blurs {
                if let Some(ref effects_engine) = self.effects_engine {
                    let tex_size = cgmath::Vector2::new(
                        surface_texture.texture.width(),
                        surface_texture.texture.height(),
                    );
                    if let Err(e) = effects_engine.write().render_blurs(
                        &mut encoder,
                        &view,
                        &view,
                        tex_size,
                    ) {
                        log::warn!("⚠️ Surface blur render failed: {}", e);
                    }
                }
            }

            self.queue.submit(std::iter::once(encoder.finish()));
        }

        // Clear per-frame effect queues after consumption
        self.window_shadows.clear();
        self.window_blurs.clear();

        Ok(())
    }

    /// Wire an effects engine for future shadow/blur post-processing passes.
    /// Once set, surface renders can apply GPU effects automatically.
    pub fn set_effects_engine(
        &mut self,
        engine: Arc<parking_lot::RwLock<crate::effects::EffectsEngine>>,
    ) {
        self.effects_engine = Some(engine);
    }

    /// Queue shadow rendering for a window. Called each frame from render_frame()
    /// with data pulled from the effects engine. Shadows are consumed by the
    /// surface rendering pass (render_to_surface_with_shadows).
    pub fn queue_shadow(
        &mut self,
        id: u64,
        position: (f32, f32),
        size: (f32, f32),
        params: crate::effects::ShadowParams,
    ) {
        self.window_shadows.insert(id, (position, size, params));
    }

    /// Clear the per-frame shadow queue (should be called at the start of each frame).
    pub fn clear_shadows(&mut self) {
        self.window_shadows.clear();
    }

    /// Queue blur rendering for a window. Called each frame from render_frame()
    /// with data pulled from the effects engine.
    pub fn queue_blur(
        &mut self,
        id: u64,
        position: (f32, f32),
        size: (f32, f32),
        params: crate::effects::BlurParams,
    ) {
        self.window_blurs.insert(id, (position, size, params));
    }

    /// Clear the per-frame blur queue (should be called at the start of each frame).
    pub fn clear_blurs(&mut self) {
        self.window_blurs.clear();
    }

    /// Get the current blur queue for surface rendering.
    #[allow(dead_code)]
    pub fn blur_queue(&self) -> &BlurQueue {
        &self.window_blurs
    }

    /// Get the current shadow queue for surface rendering.
    /// Consumed by the backend when calling render_to_surface_with_shadows.
    #[allow(dead_code)]
    pub fn shadow_queue(&self) -> &ShadowQueue {
        &self.window_shadows
    }

    /// Get number of rendered windows
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Remove a window and its associated state (texture, queued shadow/blur).
    /// GPU textures owned by this renderer are dropped along with the
    /// `RenderedWindow` entry, fixing a long-running compositor that would
    /// otherwise accumulate stale GPU resources across window lifecycle.
    pub fn remove_window(&mut self, id: u64) {
        let before = self.windows.len();
        self.windows.retain(|w| w.id != id);
        let removed = self.windows.len() != before;
        if removed {
            self.window_shadows.remove(&id);
            self.window_blurs.remove(&id);
            log::trace!("🗑️ Renderer: removed window {}", id);
        }
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

/// Get or create a headless output texture for off-screen shadow/blur rendering.
/// Free function to support disjoint borrows — the returned reference only
/// borrows `headless_target`, not the entire struct.
fn ensure_headless_target<'a>(
    device: &Device,
    headless_target: &'a mut Option<(Texture, TextureView)>,
    width: u32,
    height: u32,
) -> &'a TextureView {
    let recreate = headless_target
        .as_ref()
        .is_none_or(|(tex, _)| tex.width() != width || tex.height() != height);

    if recreate {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Headless Render Target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        *headless_target = Some((texture, view));
    }

    &headless_target.as_ref().unwrap().1
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
