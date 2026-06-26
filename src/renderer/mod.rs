//! GPU rendering pipeline for Axiom compositor
//!
//! This module implements actual GPU rendering using wgpu to composite
//! windows and effects to the screen.
//!
//! ## Architecture
//! - [`AxiomRenderer`]: Main renderer managing WGPU device, surfaces, and pipelines
//! - Window textures uploaded from Wayland SHM buffers
//! - Shadow/blur post-processing via effects engine integration
//! - Headless render target for off-screen GPU compositing

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

    /// Headless output texture + view for off-screen shadow/blur passes.
    /// Created lazily and resized when dimensions change. Used by render()
    /// for GPU effects compositing when no surface is attached.
    headless_target: Option<(Texture, TextureView)>,

    /// WGPU Render Pipeline
    render_pipeline: RenderPipeline,
    /// WGPU Sampler
    sampler: Sampler,

    /// Cached projection uniform buffer — reused across frames to avoid
    /// per-frame GPU allocation churn. Recreated only when output
    /// dimensions change.
    cached_projection_buffer: Option<Buffer>,
    cached_projection_dims: (u32, u32),
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

    /// Cached uniform buffer — reused across frames to avoid per-frame GPU
    /// allocation. Recreated only when opacity changes.
    cached_uniform_buffer: Option<Buffer>,
    cached_opacity: f32,

    /// Cached vertex buffer — reused across frames. Recreated only when
    /// position or size changes.
    cached_vertex_buffer: Option<Buffer>,
    cached_position: (f32, f32),
    cached_size: (f32, f32),
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

/// Push a deferred placeholder quad for window `id` to the global render
/// state. Consumed by the headless render loop (see
/// [`AxiomRenderer::start_headless_loop`]). Used by demos and tests that
/// want a visible rectangle without going through the Wayland buffer
/// upload path.
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
            headless_target: None,
            render_pipeline,
            sampler,
            cached_projection_buffer: None,
            cached_projection_dims: (0, 0),
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
            headless_target: None,
            render_pipeline,
            sampler,
            cached_projection_buffer: None,
            cached_projection_dims: (0, 0),
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
    pub fn add_window(&mut self, id: u64, position: (f32, f32), size: (f32, f32)) {
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
            cached_uniform_buffer: None,
            cached_opacity: f32::NAN,
            cached_vertex_buffer: None,
            cached_position: (f32::NAN, f32::NAN),
            cached_size: (f32::NAN, f32::NAN),
        };

        self.windows.push(window);
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
                cached_uniform_buffer: None,
                cached_opacity: f32::NAN,
                cached_vertex_buffer: None,
                cached_position: (f32::NAN, f32::NAN),
                cached_size: (f32::NAN, f32::NAN),
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
    pub fn render(&mut self) {
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
            return;
        }

        // Delegate to composite_effects_on_buffer — the old headless-only path
        // is superseded by the GL-framebuffer bridging in backend::render().
        // Clear any stale queues without doing extra GPU work.
        self.window_shadows.clear();
        self.window_blurs.clear();
    }

    /// Composite shadow/blur effects onto an existing RGBA framebuffer.
    ///
    /// Uploads `input_rgba` to a WGPU texture, runs any queued shadow and blur
    /// passes from the internal queues, then reads back the result. If no
    /// effects are queued or no effects engine is wired, returns the input
    /// unchanged (clipped to `width * height * 4` bytes).
    ///
    /// The internal shadow and blur queues are consumed by this call.
    pub fn composite_effects_on_buffer(
        &mut self,
        input_rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        use cgmath::Vector2;

        let expected_len = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        let has_effects = !self.window_shadows.is_empty() || !self.window_blurs.is_empty();

        if !has_effects || self.effects_engine.is_none() {
            self.window_shadows.clear();
            self.window_blurs.clear();
            return Ok(input_rgba
                .get(..expected_len)
                .unwrap_or(input_rgba)
                .to_vec());
        }

        // Ensure headless target exists and is correctly sized
        let (headless_tex, target_view) =
            ensure_headless_target(&self.device, &mut self.headless_target, width, height);

        // Upload the GL framebuffer contents into the headless WGPU texture
        let upload_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let bytes_per_row = std::num::NonZeroU32::new(4 * width);
        if let Some(bpr) = bytes_per_row {
            self.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: headless_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                input_rgba.get(..expected_len).unwrap_or(input_rgba),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr.get()),
                    rows_per_image: Some(height),
                },
                upload_size,
            );
        }

        // Apply effects in a single encoder
        let effects_engine = match self.effects_engine.as_ref() {
            Some(e) => e,
            None => {
                log::warn!("composite_effects_on_buffer called but effects_engine is not set; skipping GPU effects");
                return Ok(vec![]);
            }
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("GL-Bridged Effects Encoder"),
            });

        // Dispatch shadows
        if !self.window_shadows.is_empty() {
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
                effects_engine
                    .write()
                    .render_shadows(&mut encoder, target_view, &shadow_data)
            {
                log::warn!("⚠️ Shadow pass on GL bridge failed: {}", e);
            }
        }

        // Dispatch blurs
        if !self.window_blurs.is_empty() {
            let tex_size = Vector2::new(width, height);
            if let Err(e) = effects_engine.write().render_blurs(
                &mut encoder,
                target_view,
                target_view,
                tex_size,
            ) {
                log::warn!("⚠️ Blur pass on GL bridge failed: {}", e);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        // Read back the composited result via a staging buffer
        let buffer_size = (width as u64)
            .saturating_mul(height as u64)
            .saturating_mul(4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Effects Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut copy_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Effects Copy Encoder"),
                });
        copy_encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: headless_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: bytes_per_row.map(std::num::NonZeroU32::get),
                    rows_per_image: Some(height),
                },
            },
            upload_size,
        );
        self.queue.submit(std::iter::once(copy_encoder.finish()));

        // Poll for completion and read back
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| anyhow::anyhow!("GPU readback channel closed"))??;

        let mapped = slice.get_mapped_range();
        let result = mapped.to_vec();
        drop(mapped);
        staging.unmap();

        // Consume per-frame effect queues
        self.window_shadows.clear();
        self.window_blurs.clear();

        debug!(
            "🖥️ GL-bridged effects composite: {}x{} ({} bytes)",
            width,
            height,
            result.len()
        );
        Ok(result)
    }

    /// Render specifically to the named output.
    ///
    /// This used to hold an immutable borrow of `self.surfaces` across a
    /// `&mut self` call to `render_to_surface`, which tripped `E0502`.
    /// The fix is to take ownership of the `(surface, config)` entry via
    /// `remove_entry`, drop the immutable borrow, then re-insert on the
    /// way out. The `Surface` itself isn't `Clone` in wgpu 0.19, so this
    /// swap pattern is the simplest correct fix.
    pub fn render_output(&mut self, output_name: &str) -> Result<()> {
        if let Some((key, (surface, config))) = self.surfaces.remove_entry(output_name) {
            let config_clone = config.clone();
            let result = match surface.get_current_texture() {
                Ok(frame) => {
                    let render_result = self.render_to_surface(&surface, &config_clone, &frame);
                    if render_result.is_ok() {
                        frame.present();
                    }
                    render_result
                }
                Err(e) => {
                    log::warn!("Failed to get current texture for output {}: {}", key, e);
                    Err(anyhow::anyhow!(e))
                }
            };
            // Always re-insert so the surface survives the call regardless
            // of whether rendering succeeded.
            self.surfaces.insert(key, (surface, config));
            result
        } else {
            log::warn!(
                "Attempted to render to non-existent output: {}",
                output_name
            );
            Ok(())
        }
    }

    /// Render all windows to a wgpu surface (real rendering).
    /// Uses the provided `config` for projection dimensions — no longer
    /// picks an arbitrary first surface from the map.
    pub fn render_to_surface(
        &mut self,
        _surface: &wgpu::Surface<'_>,
        config: &wgpu::SurfaceConfiguration,
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

        let width = config.width as f32;
        let height = config.height as f32;

        // Reuse cached projection buffer when dimensions haven't changed.
        // This avoids the dominant per-frame GPU allocation in this function.
        let dims = (config.width, config.height);
        if self.cached_projection_dims != dims {
            let projection = create_projection_matrix(width, height);
            let flat: Vec<f32> = projection.iter().flatten().copied().collect();
            let contents = bytemuck::cast_slice(&flat);

            if let Some(ref buf) = self.cached_projection_buffer {
                // Existing buffer is wrong size — rebuild via a fresh one
                // (write_buffer can't resize). Drop implicitly and replace.
                let _ = buf;
                self.cached_projection_buffer = Some(self.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Projection Uniform Buffer"),
                        contents,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    },
                ));
            } else {
                self.cached_projection_buffer = Some(self.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Projection Uniform Buffer"),
                        contents,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    },
                ));
            }
            self.cached_projection_dims = dims;
        }

        let uniform_buffer = self
            .cached_projection_buffer
            .as_ref()
            .expect("projection buffer initialized");

        // Prepare resources before starting render pass to avoid lifetime issues
        let mut draw_commands = Vec::new();

        for window in &mut self.windows {
            if let Some(texture_view) = &window.texture_view {
                // Recreate uniform buffer only when opacity changes
                if (window.cached_opacity - window.opacity).abs() > f32::EPSILON
                    || window.cached_uniform_buffer.is_none()
                {
                    let window_uniforms = WindowUniforms {
                        opacity: window.opacity,
                        padding: [0.0; 3],
                    };
                    window.cached_uniform_buffer =
                        Some(self.device.create_buffer_init(
                            &wgpu::util::BufferInitDescriptor {
                                label: Some(&format!("Window {} Uniforms", window.id)),
                                contents: bytemuck::cast_slice(&[window_uniforms]),
                                usage: wgpu::BufferUsages::UNIFORM
                                    | wgpu::BufferUsages::COPY_DST,
                            },
                        ));
                    window.cached_opacity = window.opacity;
                }

                let window_uniform_buffer = window
                    .cached_uniform_buffer
                    .as_ref()
                    .expect("uniform buffer initialized");

                // Create bind group for this window (cheap; references cached buffers)
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

                // Recreate vertex buffer only when position or size changes
                let pos_changed = (window.cached_position.0 - window.position.0).abs()
                    > f32::EPSILON
                    || (window.cached_position.1 - window.position.1).abs() > f32::EPSILON;
                let size_changed = (window.cached_size.0 - window.size.0).abs() > f32::EPSILON
                    || (window.cached_size.1 - window.size.1).abs() > f32::EPSILON;

                if pos_changed || size_changed || window.cached_vertex_buffer.is_none() {
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

                    window.cached_vertex_buffer =
                        Some(self.device.create_buffer_init(
                            &wgpu::util::BufferInitDescriptor {
                                label: Some(&format!("Window {} Vertex Buffer", window.id)),
                                contents: bytemuck::cast_slice(&vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            },
                        ));
                    window.cached_position = window.position;
                    window.cached_size = window.size;
                }

                let vertex_buffer = window
                    .cached_vertex_buffer
                    .as_ref()
                    .expect("vertex buffer initialized");

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
        &mut self,
        surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
        on_shadows: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView) -> Result<()>,
    ) -> Result<()> {
        // Composite windows first — use first surface's config for
        // projection (single-output path; multi-output callers should
        // use the dedicated render_output path).
        let config_clone = self
            .surfaces
            .values()
            .next()
            .map(|(_, c)| c.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("render_to_surface_with_shadows: no surface config available")
            })?;
        self.render_to_surface(surface, &config_clone, surface_texture)?;

        // Run shadow pass as a separate draw batch
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut shadow_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Shadow Encoder"),
                });
        on_shadows(&mut shadow_encoder, &view)?;
        self.queue.submit(std::iter::once(shadow_encoder.finish()));
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

        // Composite windows first — use first surface's config for
        // projection (single-output path; multi-output callers should
        // use the dedicated render_output path).
        let config_clone = self
            .surfaces
            .values()
            .next()
            .map(|(_, c)| c.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("render_to_surface_auto: no surface config available")
            })?;
        self.render_to_surface(surface, &config_clone, surface_texture)?;

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let has_shadows = !self.window_shadows.is_empty();
        let has_blurs = !self.window_blurs.is_empty();

        if has_shadows || has_blurs {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Effects Post-Process Encoder"),
                });

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
                        if let Err(e) =
                            effects_engine
                                .write()
                                .render_shadows(&mut encoder, &view, &shadow_data)
                        {
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
                    if let Err(e) =
                        effects_engine
                            .write()
                            .render_blurs(&mut encoder, &view, &view, tex_size)
                    {
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
                renderer.render();
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
/// Free function to support disjoint borrows — the returned references only
/// borrow `headless_target`, not the entire struct.
fn ensure_headless_target<'a>(
    device: &Device,
    headless_target: &'a mut Option<(Texture, TextureView)>,
    width: u32,
    height: u32,
) -> (&'a Texture, &'a TextureView) {
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        *headless_target = Some((texture, view));
    }

    let (ref tex, ref view) = headless_target
        .as_ref()
        .expect("headless_target must be initialized before get_or_create_headless_target returns");
    (tex, view)
}

/// Build a 4×4 column-major orthographic projection matrix for a surface
/// of `(width, height)` in compositor logical pixels. The matrix maps
/// screen-space coordinates `(0, 0)` to the top-left and `(width, height)`
/// to the bottom-right, with Y flipped (typical UI orientation).
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
            cached_uniform_buffer: None,
            cached_opacity: f32::NAN,
            cached_vertex_buffer: None,
            cached_position: (f32::NAN, f32::NAN),
            cached_size: (f32::NAN, f32::NAN),
        };

        assert_eq!(window.id, 1);
        assert_eq!(window.position, (100.0, 100.0));
        assert_eq!(window.size, (400.0, 300.0));
        assert!(window.dirty);
    }
}
