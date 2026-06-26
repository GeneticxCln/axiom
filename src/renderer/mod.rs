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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// Type aliases for renderer effect queues reduce type complexity.
type ShadowQueue = HashMap<u64, ((f32, f32), (f32, f32), crate::effects::ShadowParams)>;
type BlurQueue = HashMap<u64, ((f32, f32), (f32, f32), crate::effects::BlurParams)>;
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
    pub windows: Vec<RenderedWindow>,

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

    /// Per-frame effect queue sizes tracked atomically so the backend can
    /// observe "is work pending?" with relaxed loads without taking the
    /// renderer's RwLock. Each `queue_*`/`clear_*` mutation mirrors into
    /// the corresponding counter. Drained (set to 0) on `clear_*` and on
    /// any successful composite in `composite_effects_on_buffer`.
    pending_shadows: AtomicUsize,
    pending_blurs: AtomicUsize,

    /// Border width in pixels for window decoration
    border_width: f32,
    /// Cached border width used in the last per-window uniform buffer writes.
    /// Compared with `self.border_width` to detect when all window uniform
    /// buffers need invalidation (border width is baked into each uniform).
    cached_border_width: f32,
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
    /// allocation. Recreated when opacity, size, or border width changes.
    pub cached_uniform_buffer: Option<Buffer>,
    pub cached_opacity: f32,
    /// Cached (width, height) used in the last uniform buffer write.
    /// Compared with current `self.size` to detect resize-driven
    /// invalidation independent of opacity changes.
    pub cached_uniform_size: (f32, f32),

    /// Cached vertex buffer — reused across frames. Recreated only when
    /// position or size changes.
    pub cached_vertex_buffer: Option<Buffer>,
    pub cached_position: (f32, f32),
    pub cached_size: (f32, f32),
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
    border_width: f32,
    window_width: f32,
    window_height: f32,
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
            border_width: 2.0,
            cached_border_width: 2.0,
            pending_shadows: AtomicUsize::new(0),
            pending_blurs: AtomicUsize::new(0),
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
            border_width: 2.0,
            cached_border_width: 2.0,
            pending_shadows: AtomicUsize::new(0),
            pending_blurs: AtomicUsize::new(0),
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
            cached_uniform_size: (f32::NAN, f32::NAN),
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
                cached_uniform_size: (f32::NAN, f32::NAN),
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
        self.pending_shadows.store(0, Ordering::Relaxed);
        self.pending_blurs.store(0, Ordering::Relaxed);
    }

    /// Non-locking observation of whether any post-processing effect has
    /// been queued for the current frame. Used by the backend to decide
    /// whether to perform the (expensive) GL→WGPU ping-pong or submit the
    /// GL framebuffer directly.
    ///
    /// Returns `false` when there is nothing to do. The atomic loads are
    /// ordered Relaxed because callers only need eventual consistency —
    /// at worst, a freshly-queued effect will be observed on the next
    /// render, and a stale count simply gates one extra round-trip.
    pub fn has_pending_post_process(&self) -> bool {
        self.pending_shadows.load(Ordering::Relaxed) > 0
            || self.pending_blurs.load(Ordering::Relaxed) > 0
    }

    /// Run the GPU effects composite, but only when there is work to do.
    ///
    /// This is the only entry point that performs the GL->WGPU->GL round
    /// trip. When `has_pending_post_process()` is `false` (or no
    /// `effects_engine` is wired), the per-frame queues are drained and
    /// `Ok(None)` is returned -- the caller should submit the GL
    /// framebuffer unchanged. When work IS queued and the effects engine
    /// is available, the full round-trip runs and a processed RGBA buffer
    /// is returned for the caller to upload back into GL.
    ///
    /// Internally delegates to the (now-deprecated) round-trip helper for
    /// the warm path; the `#[allow(deprecated)]` keeps that single call
    /// quiet without polluting the public recommendation channel.
    #[allow(deprecated)]
    pub fn drain_post_process(
        &mut self,
        width: u32,
        height: u32,
        gl_pixels: Vec<u8>,
    ) -> Result<Option<Vec<u8>>> {
        let has_effects =
            !self.window_shadows.is_empty() || !self.window_blurs.is_empty();
        if !has_effects || self.effects_engine.is_none() {
            // Drop the readback bytes immediately — they would be
            // discarded downstream (replaced by the original GL
            // framebuffer) and not paying for them in the cold path is
            // the entire point of this optimisation.
            self.window_shadows.clear();
            self.window_blurs.clear();
            self.pending_shadows.store(0, Ordering::Relaxed);
            self.pending_blurs.store(0, Ordering::Relaxed);
            return Ok(None);
        }
        let processed = self.composite_effects_on_buffer(&gl_pixels, width, height)?;
        Ok(Some(processed))
    }

    /// Composite shadow/blur effects onto an existing RGBA framebuffer.
    ///
    /// Uploads `input_rgba` to a WGPU texture, runs any queued shadow and blur
    /// passes from the internal queues, then reads back the result. If no
    /// effects are queued or no effects engine is wired, returns the input
    /// unchanged (clipped to `width * height * 4` bytes).
    ///
    /// ***Prefer [`drain_post_process`] from backend code:*** this method
    /// forces a full GL→WGPU→GL ping-pong even when nothing is queued. The
    /// drain entry point skips both the CPU readback allocation *and* the
    /// upload/composite when [`has_pending_post_process`] is `false`.
    ///
    /// The internal shadow and blur queues are consumed by this call.
    ///
    /// [`drain_post_process`]: AxiomRenderer::drain_post_process
    /// [`has_pending_post_process`]: AxiomRenderer::has_pending_post_process
    #[deprecated(
        since = "0.1.0",
        note = "use drain_post_process instead -- it skips the GL->CPU readback allocation when no shadows/blurs are queued"
    )]
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
            self.pending_shadows.store(0, Ordering::Relaxed);
            self.pending_blurs.store(0, Ordering::Relaxed);
            return Ok(input_rgba
                .get(..expected_len)
                .unwrap_or(input_rgba)
                .to_vec());
        }

        // Ensure headless target exists and is correctly sized
        let (headless_tex, target_view) =
            ensure_headless_target(&self.device, &mut self.headless_target, width, height)?;

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
        self.pending_shadows.store(0, Ordering::Relaxed);
        self.pending_blurs.store(0, Ordering::Relaxed);

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

    /// Recreate per-window uniform and vertex buffers when their
    /// source data (opacity, size, border width, position) has changed.
    ///
    /// This method is extracted from [`render_to_surface`] so it can be
    /// exercised in integration tests without requiring a real
    /// `wgpu::Surface`. After calling it, each `RenderedWindow` that has a
    /// texture will have `cached_uniform_buffer` and
    /// `cached_vertex_buffer` populated with up-to-date GPU buffers.
    ///
    /// Idempotent — windows whose state hasn't changed are left untouched
    /// and the return value only reflects windows that actually needed
    /// recreation this pass.
    pub fn prepare_window_resources(&mut self) {
        self.cached_border_width = self.border_width;

        for window in &mut self.windows {
            if window.texture_view.is_none() {
                continue;
            }

            // Uniform buffer invalidation: opacity, size, or border width
            let opacity_changed =
                (window.cached_opacity - window.opacity).abs() > f32::EPSILON;
            let size_changed = (window.cached_uniform_size.0 - window.size.0).abs()
                > f32::EPSILON
                || (window.cached_uniform_size.1 - window.size.1).abs() > f32::EPSILON;
            let border_changed =
                (self.cached_border_width - self.border_width).abs() > f32::EPSILON;

            if opacity_changed
                || size_changed
                || border_changed
                || window.cached_uniform_buffer.is_none()
            {
                let window_uniforms = WindowUniforms {
                    opacity: window.opacity,
                    border_width: self.border_width,
                    window_width: window.size.0,
                    window_height: window.size.1,
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
                window.cached_uniform_size = window.size;
            }

            // Vertex buffer invalidation: position or size
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

                if pos_changed || size_changed || window.cached_vertex_buffer.is_none() {
                    let x = window.position.0;
                    let y = window.position.1;
                    let w = window.size.0;
                    let h = window.size.1;

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
        debug!("\u{1f3a8} Rendering {} windows to surface", self.windows.len());

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

            // Recreate when dimensions change (write_buffer can't resize).
            // The old buffer is dropped when `Some` is replaced.
            self.cached_projection_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Projection Uniform Buffer"),
                    contents,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ));
            self.cached_projection_dims = dims;
        }

        // Delegate per-window uniform/vertex buffer cache management to
        // the extracted helper so it can be tested in isolation.
        // Called before borrowing cached_projection_buffer to satisfy
        // NLL — prepare_window_resources needs &mut self, but
        // uniform_buffer below needs a shared borrow that lives through
        // the bind-group loop.
        self.prepare_window_resources();

        let uniform_buffer = self
            .cached_projection_buffer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("projection buffer not initialized"))?;

        // Prepare resources before starting render pass to avoid lifetime issues
        let mut draw_commands = Vec::new();

        for window in &self.windows {
            if let Some(texture_view) = &window.texture_view {
                let window_uniform_buffer = window
                    .cached_uniform_buffer
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("uniform buffer for window {} not initialized", window.id))?;

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

                let vertex_buffer = window
                    .cached_vertex_buffer
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("vertex buffer for window {} not initialized", window.id))?;

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

    /// Render a clear-color fill to a headless target and read it back.
    ///
    /// Creates a temporary `RENDER_ATTACHMENT | COPY_SRC` texture at the
    /// given size, fills it with `(r, g, b, a)` via a render pass, copies
    /// the result to a staging buffer, and maps it back to the CPU. The
    /// returned `Vec<u8>` contains `width × height × 4` RGBA bytes.
    ///
    /// This exercises the full headless GPU pipeline — texture creation,
    /// command encoding, render pass, copy, and async buffer mapping —
    /// without requiring a surface. Useful for integration tests.
    pub fn render_headless_clear_readback(
        &self,
        width: u32,
        height: u32,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    ) -> Result<Vec<u8>> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        // Render-target texture
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Headless Clear Target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // Render pass: clear to the requested color
        let mut encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Headless Clear Encoder"),
                });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Headless Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r as f64 / 255.0,
                            g: g as f64 / 255.0,
                            b: b as f64 / 255.0,
                            a: a as f64 / 255.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }

        // Staging buffer for readback
        let buffer_size = (width as u64)
            .saturating_mul(height as u64)
            .saturating_mul(4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Headless Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
            },
            size,
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Map and read back
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

        Ok(result)
    }

    /// Get instance for external use (creating surfaces)
    pub fn instance(&self) -> Arc<Instance> {
        self.instance.clone()
    }

    /// Composite all textured windows to a headless render target and
    /// read the result back to CPU memory.
    ///
    /// Creates a temporary headless texture, runs the full compositor
    /// render pipeline (projection → vertex shader → textured quad
    /// fragment shader with alpha blending), copies the result to a
    /// staging buffer, and maps it back as RGBA bytes. The returned
    /// `Vec<u8>` has `width × height × 4` elements.
    ///
    /// This exercises the full textured-quad render path — GPU buffers,
    /// bind groups, pipeline, draw calls, and readback — without
    /// needing a `wgpu::Surface`. Designed for integration tests.
    pub fn render_to_headless_target(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        // Ensure per-window GPU buffers are up to date
        self.prepare_window_resources();

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        // Headless render target — must match the pipeline's format.
        // The headless pipeline was created with Bgra8UnormSrgb.
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Headless Composite Target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // Reuse cached projection buffer when dimensions match.
        // Mirrors the optimisation in render_to_surface — avoids
        // per-frame GPU allocation churn for repeated same-size
        // headless composites (e.g. integration test loops).
        let dims = (width, height);
        if self.cached_projection_dims != dims {
            let projection = create_projection_matrix(width as f32, height as f32);
            let flat: Vec<f32> = projection.iter().flatten().copied().collect();
            self.cached_projection_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Headless Projection Uniform"),
                    contents: bytemuck::cast_slice(&flat),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ));
            self.cached_projection_dims = dims;
        }
        let proj_buffer = self
            .cached_projection_buffer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("projection buffer not initialized"))?;

        let mut encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Headless Composite Encoder"),
                });

        // Build draw commands before the render pass to keep borrows happy
        let mut draw_commands = Vec::new();
        for window in &self.windows {
            if let Some(texture_view) = &window.texture_view {
                let window_ub = window
                    .cached_uniform_buffer
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "uniform buffer for window {} not initialized",
                            window.id
                        )
                    })?;
                let vertex_buf = window
                    .cached_vertex_buffer
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "vertex buffer for window {} not initialized",
                            window.id
                        )
                    })?;

                let bind_group =
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        layout: &self.render_pipeline.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: proj_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(
                                    texture_view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(
                                    &self.sampler,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: window_ub.as_entire_binding(),
                            },
                        ],
                        label: Some(&format!(
                            "Window {} Headless Bind Group",
                            window.id
                        )),
                    });

                draw_commands.push((window.id, bind_group, vertex_buf));
            }
        }

        // Render pass: clear to transparent black, then composite windows
        {
            let mut render_pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Headless Composite Pass"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.0,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 0.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        },
                    )],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });

            render_pass.set_pipeline(&self.render_pipeline);

            for (_id, bind_group, vertex_buf) in &draw_commands {
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.set_vertex_buffer(0, vertex_buf.slice(..));
                render_pass.draw(0..6, 0..1);
            }
        }

        // Staging buffer for GPU→CPU readback
        let buffer_size = (width as u64)
            .saturating_mul(height as u64)
            .saturating_mul(4);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Headless Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
            },
            size,
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Map and read back
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

        Ok(result)
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
        self.pending_shadows.store(0, Ordering::Relaxed);
        self.pending_blurs.store(0, Ordering::Relaxed);

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

    /// Set the border width in pixels for window decoration rendering.
    ///
    /// When the border width changes, all per-window uniform buffers are
    /// invalidated so the next `render_to_surface` pass will recreate them
    /// with the updated value. Without this, windows whose opacity hasn't
    /// changed would render with a stale border width baked into their
    /// cached uniform buffer.
    pub fn set_border_width(&mut self, width: f32) {
        if (self.border_width - width).abs() < f32::EPSILON {
            return;
        }
        self.border_width = width;
        // Invalidate all per-window uniform buffer caches: border_width is
        // embedded in every WindowUniforms write, so they must be
        // regenerated on the next render pass.
        for window in &mut self.windows {
            window.cached_uniform_buffer = None;
        }
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
        // Mirror the queue length into the atomic so the backend can
        // observe whether work is pending without acquiring the renderer
        // lock. `len()` is O(1) for HashMap and reflects reality after
        // the insert.
        self.pending_shadows
            .store(self.window_shadows.len(), Ordering::Relaxed);
    }

    /// Clear the per-frame shadow queue (should be called at the start of each frame).
    pub fn clear_shadows(&mut self) {
        self.window_shadows.clear();
        self.pending_shadows.store(0, Ordering::Relaxed);
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
        self.pending_blurs
            .store(self.window_blurs.len(), Ordering::Relaxed);
    }

    /// Clear the per-frame blur queue (should be called at the start of each frame).
    pub fn clear_blurs(&mut self) {
        self.window_blurs.clear();
        self.pending_blurs.store(0, Ordering::Relaxed);
    }

    /// Get number of rendered windows
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Remove a window and its associated state (texture, queued shadow/blur).
    /// GPU textures owned by this renderer are dropped along with the
    /// `RenderedWindow` entry, fixing a long-running compositor that would
    /// otherwise accumulate stale GPU resources across window lifecycle.
    /// Check whether the projection buffer cache is populated.
    /// Used by integration tests to verify caching behaviour.
    pub fn has_cached_projection(&self) -> bool {
        self.cached_projection_buffer.is_some()
    }

    /// Get the cached projection dimensions, or `(0, 0)` when empty.
    /// Used by integration tests to verify cache reuse across calls.
    pub fn cached_projection_dims(&self) -> (u32, u32) {
        self.cached_projection_dims
    }

    /// Check whether a window has a queued shadow effect.
    /// Used by integration tests to verify queue lifecycle.
    pub fn has_window_shadow(&self, id: u64) -> bool {
        self.window_shadows.contains_key(&id)
    }

    /// Check whether a window has a queued blur effect.
    /// Used by integration tests to verify queue lifecycle.
    pub fn has_window_blur(&self, id: u64) -> bool {
        self.window_blurs.contains_key(&id)
    }

    /// Remove a window and its associated state (texture, queued shadow/blur).
    /// GPU textures owned by this renderer are dropped along with the
    /// `RenderedWindow` entry, fixing a long-running compositor that would
    /// otherwise accumulate stale GPU resources across window lifecycle.
    ///
    /// Returns `true` if the window existed and was removed, `false` if
    /// the ID was not found (no-op).
    pub fn remove_window(&mut self, id: u64) -> bool {
        let before = self.windows.len();
        self.windows.retain(|w| w.id != id);
        let removed = self.windows.len() != before;
        if removed {
            self.window_shadows.remove(&id);
            self.window_blurs.remove(&id);
            // Unconditional `store(Relaxed)` is a few cycles; cheaper than
            // the len()/compare branch the previous code had, and the
            // hashmap `.remove` already invalidated any stale queue entry.
            self.pending_shadows
                .store(self.window_shadows.len(), Ordering::Relaxed);
            self.pending_blurs
                .store(self.window_blurs.len(), Ordering::Relaxed);
            log::trace!("🗑️ Renderer: removed window {}", id);
        }
        removed
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
) -> Result<(&'a Texture, &'a TextureView)> {
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
        .ok_or_else(|| anyhow::anyhow!("headless_target must be initialized before ensure_headless_target returns"))?;
    Ok((tex, view))
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
            cached_uniform_size: (f32::NAN, f32::NAN),
            cached_vertex_buffer: None,
            cached_position: (f32::NAN, f32::NAN),
            cached_size: (f32::NAN, f32::NAN),
        };

        assert_eq!(window.id, 1);
        assert_eq!(window.position, (100.0, 100.0));
        assert_eq!(window.size, (400.0, 300.0));
        assert!(window.dirty);
    }

    // ---- Post-processing queue / fast-path ----
    //
    // The atomic-counter mirroring and `drain_post_process` no-op
    // branches are exercised end-to-end by the compositor integration
    // tests (which actually run a real headless `AxiomRenderer`).
    // Per-process unit tests that hand-craft a `wgpu::Device` would
    // require `Arc::new(unsafe { mem::zeroed() })` on internal
    // non-Zeroable handles — which is UB even when the GPU is never
    // touched — so we keep coverage at the integration layer only.

    // ── Cache invalidation tests ──────────────────────────────────
    //
    // These tests verify the uniform-buffer and vertex-buffer caching
    // logic added for the L1 renderer optimisation. They use a real
    // headless WGPU renderer so we can create genuine Buffer handles
    // and exercise the invalidation paths without a surface.

    /// Verify that `set_border_width(new_value)` sets
    /// `cached_uniform_buffer = None` on every managed window so the
    /// next `render_to_surface` pass regenerates them with the updated
    /// border width baked into the uniform data.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_set_border_width_invalidates_cached_uniform_buffers() {
        let mut renderer = AxiomRenderer::new_headless()
            .await
            .expect("headless renderer");

        // Add two windows
        renderer.add_window(1, (0.0, 0.0), (400.0, 300.0));
        renderer.add_window(2, (500.0, 0.0), (400.0, 300.0));

        // Create real uniform buffers and stash them as cached so we
        // can verify they are cleared by set_border_width.
        let dev = renderer.device();
        for window in &mut renderer.windows {
            let buf = dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("test uniform buffer"),
                size: std::mem::size_of::<WindowUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            window.cached_uniform_buffer = Some(buf);
        }

        // Sanity: both windows have cached buffers
        assert!(renderer.windows[0].cached_uniform_buffer.is_some());
        assert!(renderer.windows[1].cached_uniform_buffer.is_some());

        // Change border width — should invalidate ALL cached uniform buffers
        renderer.set_border_width(5.0);

        assert!(
            renderer.windows[0].cached_uniform_buffer.is_none(),
            "window 0 cached uniform buffer should be invalidated"
        );
        assert!(
            renderer.windows[1].cached_uniform_buffer.is_none(),
            "window 1 cached uniform buffer should be invalidated"
        );
        assert!(
            (renderer.border_width - 5.0).abs() < f32::EPSILON,
            "border_width should be updated"
        );
    }

    /// `set_border_width` with the same value that's already stored
    /// must be a no-op — cached uniform buffers survive untouched.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_set_border_width_noop_for_same_value() {
        let mut renderer = AxiomRenderer::new_headless()
            .await
            .expect("headless renderer");
        renderer.add_window(1, (0.0, 0.0), (400.0, 300.0));

        // Seed a real cached uniform buffer
        let dev = renderer.device();
        let buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test uniform buffer"),
            size: std::mem::size_of::<WindowUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        renderer.windows[0].cached_uniform_buffer = Some(buf);
        assert!(renderer.windows[0].cached_uniform_buffer.is_some());

        // Set to the same width (default is 2.0)
        renderer.set_border_width(2.0);

        assert!(
            renderer.windows[0].cached_uniform_buffer.is_some(),
            "buffer should survive a no-op set_border_width call"
        );
    }

    /// `set_border_width` with zero windows in the renderer must not
    /// panic — the `for window in &mut self.windows` loop simply
    /// executes zero iterations and the border width is updated.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_set_border_width_zero_windows_no_panic() {
        let mut renderer = AxiomRenderer::new_headless()
            .await
            .expect("headless renderer");

        assert!(renderer.windows.is_empty(), "renderer starts with zero windows");

        // Should not panic — iterates an empty Vec and updates the field
        renderer.set_border_width(5.0);

        assert!(
            (renderer.border_width - 5.0).abs() < f32::EPSILON,
            "border_width should be updated even with zero windows"
        );
    }

    /// After `upsert_window_rect` changes a window's size, the
    /// `cached_uniform_size` field must remain stale (i.e. mismatch
    /// the new size) so the next `render_to_surface` pass detects
    /// `size_changed = true` and regenerates the uniform buffer.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_size_change_marks_uniform_stale() {
        let mut renderer = AxiomRenderer::new_headless()
            .await
            .expect("headless renderer");
        renderer.add_window(1, (0.0, 0.0), (400.0, 300.0));

        // Simulate a previous render pass that synced cached_uniform_size
        renderer.windows[0].cached_uniform_size = (400.0, 300.0);

        // Resize the window (as the compositor does each frame)
        renderer.upsert_window_rect(1, (0.0, 0.0), (600.0, 400.0), 1.0);

        let w = &renderer.windows[0];
        assert_eq!(w.size, (600.0, 400.0), "size should be updated");
        assert_eq!(
            w.cached_uniform_size,
            (400.0, 300.0),
            "cached_uniform_size should retain old value (stale)"
        );
        // The mismatch means render_to_surface would recreate the buffer
    }

    /// Changing a window's opacity via `upsert_window_rect` must leave
    /// `cached_opacity` stale so the next render pass recreates the
    /// uniform buffer with the new opacity baked in.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_opacity_change_marks_uniform_stale() {
        let mut renderer = AxiomRenderer::new_headless()
            .await
            .expect("headless renderer");
        renderer.add_window(1, (0.0, 0.0), (400.0, 300.0));

        // Simulate a previous render pass syncing cached_opacity
        renderer.windows[0].cached_opacity = 1.0;

        // Change opacity via upsert
        renderer.upsert_window_rect(1, (0.0, 0.0), (400.0, 300.0), 0.5);

        let w = &renderer.windows[0];
        assert!((w.opacity - 0.5).abs() < f32::EPSILON, "opacity should be updated");
        assert!(
            (w.cached_opacity - 1.0).abs() < f32::EPSILON,
            "cached_opacity should retain old value (stale)"
        );
        // The mismatch triggers uniform buffer recreation on next render
    }
}
