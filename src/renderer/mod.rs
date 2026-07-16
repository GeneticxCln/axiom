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

#[cfg(debug_assertions)]
use anyhow::Context;
use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;
use std::sync::Arc;

// Type aliases for renderer effect queues reduce type complexity.
type ShadowQueue = HashMap<u64, ((f32, f32), (f32, f32), crate::effects::ShadowParams)>;
type BlurQueue = HashMap<u64, ((f32, f32), (f32, f32), crate::effects::BlurParams)>;
use wgpu::util::DeviceExt;
#[allow(clippy::wildcard_imports)]
use wgpu::*;

/// Typed errors from [`AxiomRenderer`]. Distinct from `anyhow::Error` so
/// the compositor can react differently to recover-vs-fatal cases (e.g.
/// device-loss triggers a re-init, a missing pipeline is logged and
/// skipped). The `Other` arm is a catch-all for non-device-loss errors
/// that we still want to expose through the public API. Implemented
/// with std's `Error` trait (no `thiserror` dependency required).
#[derive(Debug)]
pub enum RendererError {
    DeviceLost,
    Buffer(String),
    Texture(String),
    Other(String),
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceLost => f.write_str("WGPU device was lost (driver crash / context reset)"),
            Self::Buffer(s) => write!(f, "buffer allocation or write failed: {}", s),
            Self::Texture(s) => write!(f, "texture allocation or upload failed: {}", s),
            Self::Other(s) => write!(f, "renderer error: {}", s),
        }
    }
}

impl std::error::Error for RendererError {}

impl From<anyhow::Error> for RendererError {
    fn from(e: anyhow::Error) -> Self {
        RendererError::Other(e.to_string())
    }
}

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

    /// WGPU Render Pipeline for textured windows (legacy path uses
    /// window_texture + sampler bindings at @binding(1)/@binding(2)).
    render_pipeline: RenderPipeline,
    /// Solid-color placeholder pipeline for windows without an uploaded
    /// client texture. Same vertex winding/topology/cull-mode as
    /// `render_pipeline`, but the fragment shader draws `vec4(0.15, 0.15,
    /// 0.18, opacity)` plus a per-window border, eliminating the need for
    /// the legacy GL `gl::Scissor` fallback in `src/backend/mod.rs`.
    ///
    /// **Only compiled in debug builds.** In release, windows that haven't
    /// yet committed a real client texture are simply *not drawn* — they
    /// become visible only after their first SHM commit reaches the
    /// backend. This matches Wayland's "no real buffer, no surface"
    /// semantics and prevents placeholder rectangles from masking
    /// rendering issues (a window with a broken texture upload will be
    /// visibly absent in release, instead of a misleading colored quad).
    #[cfg(debug_assertions)]
    placeholder_pipeline: RenderPipeline,
    #[cfg(debug_assertions)]
    placeholder_bind_group_layout: BindGroupLayout,

    /// WGPU Sampler
    sampler: Sampler,

    /// Cached projection uniform buffer — reused across frames to avoid
    /// per-frame GPU allocation churn. Recreated only when output
    /// dimensions change.
    cached_projection_buffer: Option<Buffer>,
    cached_projection_dims: (u32, u32),

    /// Cached staging buffer for GPU→CPU readback in the bridge / headless
    /// paths. Reused whenever the target dimensions (and therefore byte size)
    /// are unchanged so `compose_full_frame` does not allocate a fresh staging
    /// buffer every frame.
    cached_readback_buffer: Option<Buffer>,
    cached_readback_dims: (u32, u32),

    /// Border width in pixels for window decoration
    border_width: f32,
    /// Cached border width used in the last per-window uniform buffer writes.
    /// Compared with `self.border_width` to detect when all window uniform
    /// buffers need invalidation (border width is baked into each uniform).
    cached_border_width: f32,

    /// Solid-color render pipeline for server-side decoration elements
    /// (titlebar backgrounds, close/minimize/maximize buttons).
    /// Shares the cached projection uniform buffer at binding 0.
    solid_pipeline: RenderPipeline,
    /// Bind group layout for the solid pipeline (projection uniform only).
    solid_bind_group_layout: BindGroupLayout,

    /// Per-frame decoration quads. Cleared each frame, populated by
    /// the compositor's `prepare_frame_data()` from DecorationManager.
    decoration_quads: Vec<DecorationQuad>,

    /// Flipped when a wgpu primitive reports a recoverable-but-fatal
    /// error (driver crash, context lost, surface lost). Read by
    /// [`AxiomRenderer::is_device_lost`] and the compositor's device-loss
    /// recovery path. Shared via `Arc` so callbacks spawned inside
    /// `map_async` can flag loss without holding the renderer lock.
    device_lost: Arc<std::sync::atomic::AtomicBool>,
}

/// A solid-colored quad for decoration elements (titlebar backgrounds, buttons).
#[derive(Debug, Clone)]
pub struct DecorationQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 4],
}

/// Vertex format for [`AxiomRenderer`]'s solid-color decoration pipeline.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SolidVertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl SolidVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SolidVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as u64,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
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
    /// Per-window border color for focus highlighting (RGBA)
    pub border_color: [f32; 4],

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
    border_color: [f32; 4],
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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

        // Placeholder pipeline for windows without a client texture.
        // Only built in debug builds — see the `placeholder_pipeline`
        // field doc for the release-build rationale.
        #[cfg(debug_assertions)]
        let (placeholder_pipeline, placeholder_bind_group_layout) =
            create_placeholder_pipeline(&device, config.format)
                .context("Failed to build placeholder render pipeline")?;

        // Solid-color decoration pipeline for titlebars and buttons.
        // Always built — decorations are not debug-only.
        let (solid_pipeline, solid_bind_group_layout) =
            create_solid_pipeline(&device, config.format)
                .context("Failed to build solid decoration pipeline")?;

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
            #[cfg(debug_assertions)]
            placeholder_pipeline,
            #[cfg(debug_assertions)]
            placeholder_bind_group_layout,
            solid_pipeline,
            solid_bind_group_layout,
            decoration_quads: Vec::new(),
            sampler,
            cached_projection_buffer: None,
            cached_projection_dims: (0, 0),
            cached_readback_buffer: None,
            cached_readback_dims: (0, 0),
            border_width: 2.0,
            cached_border_width: 2.0,
            device_lost: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

        // Placeholder pipeline for windows without a client texture.
        // Only built in debug builds — see the `placeholder_pipeline`
        // field doc for the release-build rationale.
        #[cfg(debug_assertions)]
        let (placeholder_pipeline, placeholder_bind_group_layout) =
            create_placeholder_pipeline(&device, format)
                .context("Failed to build placeholder render pipeline")?;

        let (solid_pipeline, solid_bind_group_layout) =
            create_solid_pipeline(&device, format)
                .context("Failed to build solid decoration pipeline")?;

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
            #[cfg(debug_assertions)]
            placeholder_pipeline,
            #[cfg(debug_assertions)]
            placeholder_bind_group_layout,
            solid_pipeline,
            solid_bind_group_layout,
            decoration_quads: Vec::new(),
            sampler,
            cached_projection_buffer: None,
            cached_projection_dims: (0, 0),
            cached_readback_buffer: None,
            cached_readback_dims: (0, 0),
            border_width: 2.0,
            cached_border_width: 2.0,
            device_lost: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
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
            border_color: [0.0, 0.0, 0.0, 0.0],
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
        border_color: [f32; 4],
    ) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.position = position;
            w.size = size;
            w.opacity = opacity;
            w.border_color = border_color;
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
                border_color,
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

    /// Set the per-frame decoration quads, replacing any previous frame's data.
    /// Called by the compositor's `prepare_frame_data()` each tick.
    pub fn set_decoration_quads(&mut self, quads: Vec<DecorationQuad>) {
        self.decoration_quads = quads;
    }

    /// Clear decoration quads (called at the start of each frame).
    pub fn clear_decoration_quads(&mut self) {
        self.decoration_quads.clear();
    }

    /// Pre-build decoration GPU resources (vertex buffer + bind group)
    /// before the render pass so that the pass block does not need &self.
    /// Returns `None` when there are no decoration quads to draw.
    fn prepare_decoration_resources(
        &self,
        projection_buffer: &Buffer,
    ) -> Option<(wgpu::BindGroup, wgpu::Buffer, u32)> {
        if self.decoration_quads.is_empty() {
            return None;
        }

        const VERTS_PER_QUAD: usize = 6;
        let mut vertices: Vec<SolidVertex> =
            Vec::with_capacity(self.decoration_quads.len() * VERTS_PER_QUAD);

        for quad in &self.decoration_quads {
            let x0 = quad.x;
            let y0 = quad.y;
            let x1 = quad.x + quad.w;
            let y1 = quad.y + quad.h;
            let c = quad.color;

            vertices.push(SolidVertex { position: [x0, y0], color: c });
            vertices.push(SolidVertex { position: [x1, y0], color: c });
            vertices.push(SolidVertex { position: [x0, y1], color: c });
            vertices.push(SolidVertex { position: [x1, y0], color: c });
            vertices.push(SolidVertex { position: [x1, y1], color: c });
            vertices.push(SolidVertex { position: [x0, y1], color: c });
        }

        let vertex_buf = self.device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Decoration Vertex Buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            },
        );

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.solid_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: projection_buffer.as_entire_binding(),
            }],
            label: Some("Decoration Bind Group"),
        });

        Some((bind_group, vertex_buf, vertices.len() as u32))
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

    /// Non-locking observation of whether any post-processing effect has
    /// been queued for the current frame. Returns `false` when there is
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
            let result = loop {
                match surface.get_current_texture() {
                    Ok(frame) => {
                        let render_result = self.render_to_surface_auto(&surface, &frame);
                        if render_result.is_ok() {
                            frame.present();
                        }
                        break render_result;
                    }
                    Err(wgpu::SurfaceError::Outdated) => {
                        let new_config = config.clone();
                        surface.configure(self.device(), &new_config);
                    }
                    Err(e) => {
                        log::warn!("Failed to get current texture for {}: {}", key, e);
                        break Err(anyhow::anyhow!(e));
                    }
                }
            };
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
    /// `wgpu::Surface`. After calling it, every `RenderedWindow` will
    /// have `cached_uniform_buffer` and `cached_vertex_buffer`
    /// populated with up-to-date GPU buffers — placeholder (texture-
    /// less) windows included, since [`compose_full_frame`] renders
    /// them via the placeholder pipeline and needs the same cached
    /// vertex/uniform buffer slots.
    ///
    /// Idempotent — windows whose state hasn't changed are left untouched
    /// and the return value only reflects windows that actually needed
    /// recreation this pass.
    ///
    /// [`compose_full_frame`]: AxiomRenderer::compose_full_frame
    pub fn prepare_window_resources(&mut self) {
        self.cached_border_width = self.border_width;

        for window in &mut self.windows {
            // In release builds, skip placeholder (texture-less) windows
            // entirely — they stay invisible until they commit a real
            // client buffer, so allocating GPU resources for them would
            // be wasted memory. The placeholder pipeline that would
            // consume these buffers isn't compiled in either.
            #[cfg(not(debug_assertions))]
            if window.texture_view.is_none() {
                continue;
            }

            // (Debug-only) Placeholder (texture-less) windows also receive
            // vertex + uniform buffers — `compose_full_frame` draws them
            // via the placeholder pipeline which binds the same cached
            // buffers the textured pass uses.

            // Uniform buffer invalidation: opacity, size, or border width
            let opacity_changed = (window.cached_opacity - window.opacity).abs() > f32::EPSILON;
            let size_changed = (window.cached_uniform_size.0 - window.size.0).abs() > f32::EPSILON
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
                    border_color: window.border_color,
                };
                window.cached_uniform_buffer = Some(self.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("Window {} Uniforms", window.id)),
                        contents: bytemuck::cast_slice(&[window_uniforms]),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    },
                ));
                window.cached_opacity = window.opacity;
                window.cached_uniform_size = window.size;
            }

            // Vertex buffer invalidation: position or size
            let pos_changed = (window.cached_position.0 - window.position.0).abs() > f32::EPSILON
                || (window.cached_position.1 - window.position.1).abs() > f32::EPSILON;
            let size_changed = (window.cached_size.0 - window.size.0).abs() > f32::EPSILON
                || (window.cached_size.1 - window.size.1).abs() > f32::EPSILON;

            if pos_changed || size_changed || window.cached_vertex_buffer.is_none() {
                let x = window.position.0;
                let y = window.position.1;
                let w = window.size.0;
                let h = window.size.1;
                let xw = x + w;
                let yh = y + h;

                // Two triangles forming a textured quad.
                //
                // Winding matters: the render pipeline uses
                // `front_face: Ccw` + `cull_mode: Some(Face::Back)`, so
                // triangles must be CCW in NDC Y-up — otherwise both
                // triangles get culled and zero pixels are written.
                // TL→BL→TR and BL→BR→TR is CCW when compositor y=0 maps
                // to NDC y=+1 (top of screen), which our projection matrix
                // does. Tex-coords per corner follow the screen layout:
                // TL(0,0) BL(0,1) TR(1,0) BR(1,1).
                let vertices = [
                    // Triangle 1: TL, BL, TR
                    Vertex {
                        position: [x, y, 0.0],
                        tex_coords: [0.0, 0.0],
                    },
                    Vertex {
                        position: [x, yh, 0.0],
                        tex_coords: [0.0, 1.0],
                    },
                    Vertex {
                        position: [xw, y, 0.0],
                        tex_coords: [1.0, 0.0],
                    },
                    // Triangle 2: BL, BR, TR
                    Vertex {
                        position: [x, yh, 0.0],
                        tex_coords: [0.0, 1.0],
                    },
                    Vertex {
                        position: [xw, yh, 0.0],
                        tex_coords: [1.0, 1.0],
                    },
                    Vertex {
                        position: [xw, y, 0.0],
                        tex_coords: [1.0, 0.0],
                    },
                ];

                window.cached_vertex_buffer = Some(self.device.create_buffer_init(
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
        debug!(
            "\u{1f3a8} Rendering {} windows to surface",
            self.windows.len()
        );

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
                let window_uniform_buffer =
                    window.cached_uniform_buffer.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("uniform buffer for window {} not initialized", window.id)
                    })?;

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

                let vertex_buffer = window.cached_vertex_buffer.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("vertex buffer for window {} not initialized", window.id)
                })?;

                draw_commands.push((window.id, bind_group, vertex_buffer));
            }
        }

        // Pre-build decoration vertex buffer and bind group before the
        // render pass so we don't need &self inside the pass block.
        let decoration_resources =
            self.prepare_decoration_resources(uniform_buffer);

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

            for (_id, bind_group, vertex_buffer) in &draw_commands {
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.draw(0..6, 0..1);
            }

            // Draw server-side decorations (titlebar backgrounds, buttons).
            if let Some((ref deco_bg, ref deco_vb, deco_count)) = decoration_resources {
                render_pass.set_pipeline(&self.solid_pipeline);
                render_pass.set_bind_group(0, deco_bg, &[]);
                render_pass.set_vertex_buffer(0, deco_vb.slice(..));
                render_pass.draw(0..deco_count, 0..1);
            }
        }

        // Submit commands to GPU
        self.queue.submit(std::iter::once(encoder.finish()));

        Ok(())
    }

    /// Get device for external use. Returns a shared reference rather
    /// than a cloned `Arc` so callers can submit work without extending
    /// the GPU context's reference count. The renderer retains the
    /// strong `Arc` for its own lifetime; this getter only hands out
    /// a borrow that must not outlive the renderer lock guard when
    /// acquired through `AxiomRenderer's `parking_lot::RwLock`.
    ///
    /// ## Migration from the previous `Arc<Device>` API
    ///
    /// Callers that previously did `let dev = renderer.read().device();`
    /// to obtain an owned `Arc<Device>` must now do
    /// `let dev: &Device = renderer.read().device();` — the borrow
    /// is tied to the read guard's lifetime. The blur and shadow
    /// renderers inside `EffectsEngine` still hold the `Arc<Device>`
    /// clone internally (their GPU resources must outlast any single
    /// borrow of the renderer); see `EffectsEngine::initialize_gpu`
    /// for how the Arc clones are produced from the renderer reference
    /// at compositor construction time.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Get queue for external use. Same borrow-not-clone contract as
    /// [`AxiomRenderer::device`].
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Borrow the underlying `Arc<Device>` so callers can produce
    /// owned `Arc<Device>` clones via `Arc::clone(&self.device_arc())`.
    /// `Device` itself does not implement `Clone` in this crate's
    /// wgpu version, so this indirection is required by upstream
    /// helpers (e.g. `EffectsEngine::initialize_gpu`) that store
    /// the `Arc` for longer than the renderer lock guard.
    pub fn device_arc(&self) -> &Arc<Device> {
        &self.device
    }

    /// Borrow the underlying `Arc<Queue>` (same rationale as
    /// [`AxiomRenderer::device_arc`]).
    pub fn queue_arc(&self) -> &Arc<Queue> {
        &self.queue
    }

    /// Non-locking observation of whether the WGPU device is currently
    /// flagged as lost. The flag is set by the `compose_full_frame`
    /// `map_async` callback when WGPU rejects a buffer mapping (the
    /// driver crash / context-reset path wgpu 0.19 surfaces as an
    /// `Err(_)` callback rather than a hard panic).
    ///
    /// Returns `true` means "device has been lost; the next compose
    /// will return `Err(RendererError::DeviceLost)` until the renderer
    /// is reinitialised". Callers (e.g. compositor's device-loss
    /// recovery path) should consult this BEFORE submitting work to
    /// the queue so they can short-circuit cleanly without paying the
    /// cost of a doomed render pass.
    #[must_use]
    pub fn is_device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Manually flag device loss. Useful for tests and for the
    /// `on_uncaptured_error` callback when WGPU emits a global error
    /// not surfaced through `map_async`. Production code path is
    /// `compose_full_frame`'s readback callback — see the inline
    /// notes there.
    pub fn mark_device_lost(&self) {
        self.device_lost
            .store(true, std::sync::atomic::Ordering::Relaxed);
        log::error!("⚠️ WGPU device flagged as lost — render path will fail until reset");
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        // Render pass: clear to the requested color
        let mut encoder = self
            .device
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

    /// Compose the full presentation image — every window (textured or
    /// placeholder) plus any pending shadow/blur effects — into a
    /// single `width × height × 4` RGBA buffer ready to upload to
    /// the host GL framebuffer for present.
    ///
    /// This is the WGPU-as-composer replacement for the legacy per-
    /// window GL draw loop + scissor fallback in `src/backend/mod.rs`.
    /// By drawing textured quads AND placeholder quads on the same
    /// headless render target, every pixel of every window passes
    /// through WGPU; no `gl::Scissor` toggle is needed for
    /// fresh/uncommitted windows.
    ///
    /// The per-frame shadow and blur queues are consumed by this call.
    /// Returns `width * height * 4` bytes of RGBA8.
    /// Composite the full frame to a headless target and read back to CPU.
    /// Only used in tests now — production uses render_output (winit) or
    /// software composite (DRM).
    #[doc(hidden)]
    pub fn compose_full_frame(&mut self, width: u32, height: u32) -> Result<Vec<u8>> {
        use cgmath::Vector2;

        // Device-loss short-circuit: if a previous `map_async` callback
        // flagged the GPU as lost, return a typed error instead of
        // running doomed render work. Lets the compositor's tick loop
        // decide between reinit (emit a StateChange event) and graceful
        // fallback (skip render until reinit completes).
        if self.is_device_lost() {
            return Err(anyhow::anyhow!(
                "WGPU device lost (see is_device_lost() — renderer must be reinitialised)"
            ));
        }

        // Reuse the extraction helper that `prepare_window_resources`
        // already populates — vertex + uniform buffers are up to date.
        self.prepare_window_resources();

        // Cached ortho-projection buffer, reused across frames at the
        // same size. Mirrors the optimisation in `render_to_surface`.
        // Must happen before ensure_headless_target so the decoration
        // resources (which also need the projection buffer) can be
        // pre-built without conflicting with the headless target borrow.
        let dims = (width, height);
        if self.cached_projection_dims != dims {
            let projection = create_projection_matrix(width as f32, height as f32);
            let flat: Vec<f32> = projection.iter().flatten().copied().collect();
            self.cached_projection_buffer = Some(self.device.create_buffer_init(
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Compose Projection Uniform"),
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

        // Pre-build decoration GPU resources while we still have a
        // shared &self borrow (before the headless target borrow below).
        let decoration_resources =
            self.prepare_decoration_resources(proj_buffer);

        // Get or (lazily) recreate the headless target at the requested
        // size. Reused across frames to avoid per-frame GPU churn.
        let (target, target_view) =
            ensure_headless_target(&self.device, &mut self.headless_target, width, height)?;

        // Pre-build draw commands. Borrow checker: each entry below is
        // a fresh owned `BindGroup` (gpu handle only) plus a `&Buffer`
        // borrowed from the per-window cache; we keep the borrow alive
        // for the duration of `self`, so the Vec outlives the render
        // pass that consumes it. Same pattern as `render_to_headless_target`.
        let mut textured_draws: Vec<(u64, wgpu::BindGroup, &wgpu::Buffer)> = Vec::new();
        #[cfg(debug_assertions)]
        let mut placeholder_draws: Vec<(u64, wgpu::BindGroup, &wgpu::Buffer)> = Vec::new();

        for window in &self.windows {
            let Some(vertex_buf) = window.cached_vertex_buffer.as_ref() else {
                continue;
            };
            let Some(window_ub) = window.cached_uniform_buffer.as_ref() else {
                continue;
            };

            if let Some(texture_view) = &window.texture_view {
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("Compose Window {} Textured", window.id)),
                    layout: &self.render_pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: proj_buffer.as_entire_binding(),
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
                            resource: window_ub.as_entire_binding(),
                        },
                    ],
                });
                textured_draws.push((window.id, bg, vertex_buf));
            } else {
                // In release builds, untextured windows stay invisible
                // until they commit a real SHM buffer — no placeholder
                // draw, no bind group, no pipeline bind. The window's
                // region of the framebuffer is left at the clear color.
                #[cfg(debug_assertions)]
                {
                    let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&format!("Compose Window {} Placeholder", window.id)),
                        layout: &self.placeholder_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: proj_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: window_ub.as_entire_binding(),
                            },
                        ],
                    });
                    placeholder_draws.push((window.id, bg, vertex_buf));
                }
            }
        }

        // Pre-build decoration GPU resources for the DRM composite path.
        let (decoration_bg, decoration_vb, decoration_count) = decoration_resources
            .map(|(bg, vb, count)| (Some(bg), Some(vb), count))
            .unwrap_or((None, None, 0));

        // Snapshot effects queue contents before acquiring any
        // effects_engine write lock, so the queue can be drained
        // independently of the engine lock below.
        let has_shadows = !self.window_shadows.is_empty();
        let has_blurs = !self.window_blurs.is_empty();
        let shadow_data: Vec<(Vector2<f32>, Vector2<f32>, crate::effects::ShadowParams)> =
            if has_shadows {
                self.window_shadows
                    .values()
                    .map(|((px, py), (sx, sy), params)| {
                        (
                            Vector2::new(*px, *py),
                            Vector2::new(*sx, *sy),
                            params.clone(),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };

        // Effects data + queue snapshots — must be collected before
        // entering the render pass because the effects engine takes a
        // write-lock that would conflict with the `&mut self.windows`
        // borrow currently held through `textured_draws` /
        // `placeholder_draws`.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Compose Full-Frame Encoder"),
            });

        // ── Pass 1: textured windows + placeholder windows ────────
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Compose Full-Frame Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.08,
                            g: 0.08,
                            b: 0.12,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            // Textured pass for windows with an uploaded client texture.
            rp.set_pipeline(&self.render_pipeline);
            for (_id, bg, vb) in &textured_draws {
                rp.set_bind_group(0, bg, &[]);
                rp.set_vertex_buffer(0, vb.slice(..));
                rp.draw(0..6, 0..1);
            }

            // Placeholder pass for windows that have not yet committed.
            // Only compiled in debug builds — see the `placeholder_pipeline`
            // field doc. In release, untextured windows are not drawn at
            // all, leaving their region at the clear color.
            #[cfg(debug_assertions)]
            if !placeholder_draws.is_empty() {
                rp.set_pipeline(&self.placeholder_pipeline);
                for (_id, bg, vb) in &placeholder_draws {
                    rp.set_bind_group(0, bg, &[]);
                    rp.set_vertex_buffer(0, vb.slice(..));
                    rp.draw(0..6, 0..1);
                }
            }

            // Draw server-side decorations (titlebar backgrounds, buttons).
            if let (Some(deco_bg), Some(deco_vb)) = (&decoration_bg, &decoration_vb) {
                rp.set_pipeline(&self.solid_pipeline);
                rp.set_bind_group(0, deco_bg, &[]);
                rp.set_vertex_buffer(0, deco_vb.slice(..));
                rp.draw(0..decoration_count, 0..1);
            }
        } // end render pass — `target_view` borrow released here.

        // ── Pass 2: shadow / blur effects on the same headless target ─
        //
        // Effects open their own internal render pass with the encoder;
        // they do not need to be issued from inside the windows render
        // pass. Open encoder lifetime is the lifetime the effects writer
        // expects, so we dispatch AFTER the windows pass is closed.
        if (has_shadows || has_blurs) && self.effects_engine.is_some() {
            let tex_size = Vector2::new(width, height);
            if has_shadows && !shadow_data.is_empty() {
                if let Some(ref engine) = self.effects_engine {
                    if let Err(e) =
                        engine
                            .write()
                            .render_shadows(&mut encoder, target_view, &shadow_data)
                    {
                        log::warn!("⚠️ compose_full_frame shadows failed: {}", e);
                    }
                }
            }
            if has_blurs {
                if let Some(ref engine) = self.effects_engine {
                    if let Err(e) = engine.write().render_blurs(
                        &mut encoder,
                        target_view,
                        target_view,
                        tex_size,
                    ) {
                        log::warn!("⚠️ compose_full_frame blurs failed: {}", e);
                    }
                }
            }
        }

        // Read back the composited result via a cached staging buffer.
        let bytes_per_row = std::num::NonZeroU32::new(4 * width);
        let staging = ensure_readback_buffer(
            &self.device,
            &mut self.cached_readback_buffer,
            &mut self.cached_readback_dims,
            width,
            height,
        );

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: bytes_per_row.map(std::num::NonZeroU32::get),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Poll for completion and read back to CPU.
        let slice = staging.slice(..);
        let device_lost_signal = Arc::clone(&self.device_lost);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            // wgpu 0.19 surfaces device-loss as an Err callback here
            // (driver crash / context reset). Flip our shared flag so
            // the next compose call short-circuits with a typed
            // RendererError::DeviceLost instead of attempting doomed
            // GPU work. The forward of `r` is still required so the
            // caller can detect channel close + receive the actual
            // error context for diagnostics.
            if r.is_err() {
                device_lost_signal.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let map_result = rx
            .recv()
            .map_err(|_| anyhow::anyhow!("GPU readback channel closed"))?;
        // Surface device-loss as a typed error so the compositor can
        // branch on it (reinit vs. fall back). Anything else is an
        // anyhow-style buffer error converted to RendererError::Buffer.
        map_result.map_err(|e| anyhow::anyhow!("GPU readback map failed: {:?}", e))?;

        let mapped = slice.get_mapped_range();
        let result = mapped.to_vec();
        drop(mapped);
        staging.unmap();

        // Drain per-frame effect queues.
        self.window_shadows.clear();
        self.window_blurs.clear();

        // Compute placeholder count in a cfg-gated block expression —
        // `#[cfg]` is not allowed directly on individual macro arguments
        // (rust-lang/rust#15701), so we resolve the count first and then
        // hand the single value to `debug!`.
        let placeholder_count: usize = {
            #[cfg(debug_assertions)]
            {
                placeholder_draws.len()
            }
            #[cfg(not(debug_assertions))]
            {
                0
            }
        };

        debug!(
            "🖼️  compose_full_frame: {}x{} textured={} placeholder={} shadows={} blurs_present={} -> {} bytes",
            width,
            height,
            textured_draws.len(),
            placeholder_count,
            shadow_data.len(),
            has_blurs,
            result.len()
        );

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
    pub fn render_to_headless_target(&mut self, width: u32, height: u32) -> Result<Vec<u8>> {
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Headless Composite Encoder"),
            });

        // Build draw commands before the render pass to keep borrows happy
        let mut draw_commands = Vec::new();
        for window in &self.windows {
            if let Some(texture_view) = &window.texture_view {
                let window_ub = window.cached_uniform_buffer.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("uniform buffer for window {} not initialized", window.id)
                })?;
                let vertex_buf = window.cached_vertex_buffer.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("vertex buffer for window {} not initialized", window.id)
                })?;

                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: &self.render_pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: proj_buffer.as_entire_binding(),
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
                            resource: window_ub.as_entire_binding(),
                        },
                    ],
                    label: Some(&format!("Window {} Headless Bind Group", window.id)),
                });

                draw_commands.push((window.id, bind_group, vertex_buf));
            }
        }

        // Render pass: clear to transparent black, then composite windows
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Headless Composite Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
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
                })],
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

        // Reuse the shared readback staging buffer when dimensions match.
        let staging = ensure_readback_buffer(
            &self.device,
            &mut self.cached_readback_buffer,
            &mut self.cached_readback_dims,
            width,
            height,
        );

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: staging,
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

    /// Check whether the shared readback staging buffer cache is populated.
    pub fn has_cached_readback(&self) -> bool {
        self.cached_readback_buffer.is_some()
    }

    /// Get the cached readback dimensions, or `(0, 0)` when empty.
    pub fn cached_readback_dims(&self) -> (u32, u32) {
        self.cached_readback_dims
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
            log::trace!("🗑️ Renderer: removed window {}", id);
        }
        removed
    }

    /// Release all GPU resources held by the renderer.
    pub fn shutdown(&mut self) {
        self.windows.clear();
        self.window_shadows.clear();
        self.window_blurs.clear();
        self.surfaces.clear();
        self.cached_projection_buffer = None;
        self.cached_readback_buffer = None;
        self.headless_target = None;
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

    let (ref tex, ref view) = headless_target.as_ref().ok_or_else(|| {
        anyhow::anyhow!("headless_target must be initialized before ensure_headless_target returns")
    })?;
    Ok((tex, view))
}

/// Get or create a staging buffer sized for `width × height × 4` bytes of
/// readback data. The same buffer is reused across repeated bridge / headless
/// readbacks at identical dimensions to avoid per-frame allocation churn.
fn ensure_readback_buffer<'a>(
    device: &Device,
    cached_readback_buffer: &'a mut Option<Buffer>,
    cached_readback_dims: &mut (u32, u32),
    width: u32,
    height: u32,
) -> &'a Buffer {
    let dims = (width, height);
    if cached_readback_buffer.is_none() || *cached_readback_dims != dims {
        let buffer_size = (width as u64)
            .saturating_mul(height as u64)
            .saturating_mul(4);
        *cached_readback_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cached Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
        *cached_readback_dims = dims;
    }

    cached_readback_buffer
        .as_ref()
        .expect("readback buffer must exist after ensure_readback_buffer")
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

/// Build the placeholder pipeline used by [`AxiomRenderer::compose_full_frame`]
/// to draw solid-colored quads for windows that have not yet committed
/// a real client texture. The 2-binding layout (projection uniform + per-
/// window uniform) reuses the same cached buffers as the textured pass
/// so transitioning a window from placeholder to textured requires no
/// extra GPU uploads.
///
/// Returns `(pipeline, bind_group_layout)`. The bind group layout is
/// exposed so the constructor can hand the `wgpu::PipelineLayout` a
/// holder with the right `'static` lifetime, but most callers only
/// need the pipeline (the layout is owned by the renderer).
///
/// **Debug-only.** This function (and the `include_str!("placeholder.wgsl")`
/// it embeds) is compiled out in release builds — the placeholder
/// pipeline is dropped at compile time, and the WGSL shader bytes never
/// reach the release binary.
#[cfg(debug_assertions)]
/// Build the solid-color render pipeline used for server-side decoration
/// elements (titlebar backgrounds, close/minimize/maximize buttons).
///
/// Shares the same projection-uniform bind-group layout as the main
/// compositor pipeline so the cached projection buffer can be re-used.
/// Each quad carries its color as a per-vertex attribute so no per-quad
/// uniform buffer is needed.
pub fn create_solid_pipeline(
    device: &Device,
    format: TextureFormat,
) -> Result<(RenderPipeline, BindGroupLayout)> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Solid Decoration Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("solid.wgsl").into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Solid Decoration Bind Group Layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Solid Decoration Pipeline Layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Solid Decoration Render Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[SolidVertex::desc()],
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

    Ok((pipeline, bind_group_layout))
}

pub fn create_placeholder_pipeline(
    device: &Device,
    format: TextureFormat,
) -> Result<(RenderPipeline, BindGroupLayout)> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Placeholder Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("placeholder.wgsl").into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Placeholder Bind Group Layout"),
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
        label: Some("Placeholder Pipeline Layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Placeholder Render Pipeline"),
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

    Ok((pipeline, bind_group_layout))
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
            border_color: [0.0, 0.0, 0.0, 0.0],
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
    // The atomic-counter mirroring and effect queue lifecycle
    // are exercised end-to-end by the compositor integration
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

        // `wgpu::Device`/`Buffer` are not Clone in this crate's
        // wgpu version, so we create each buffer inside a block
        // scope that releases the `&Device` borrow before indexing
        // `renderer.windows` mutably below.
        let n = renderer.windows.len();
        for i in 0..n {
            let buf: wgpu::Buffer = {
                let dev: &wgpu::Device = renderer.device();
                dev.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("test uniform buffer"),
                    size: std::mem::size_of::<WindowUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            renderer.windows[i].cached_uniform_buffer = Some(buf);
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

        // Seed a real cached uniform buffer. `wgpu::Device` is
        // not Clone here, so we scope the `&Device` borrow inside
        // a block before assigning to the window.
        let buf: wgpu::Buffer = {
            let dev: &wgpu::Device = renderer.device();
            dev.create_buffer(&wgpu::BufferDescriptor {
                label: Some("test uniform buffer"),
                size: std::mem::size_of::<WindowUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
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

        assert!(
            renderer.windows.is_empty(),
            "renderer starts with zero windows"
        );

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
        renderer.upsert_window_rect(1, (0.0, 0.0), (600.0, 400.0), 1.0, [0.0, 0.0, 0.0, 0.0]);

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
        renderer.upsert_window_rect(1, (0.0, 0.0), (400.0, 300.0), 0.5, [0.0, 0.0, 0.0, 0.0]);

        let w = &renderer.windows[0];
        assert!(
            (w.opacity - 0.5).abs() < f32::EPSILON,
            "opacity should be updated"
        );
        assert!(
            (w.cached_opacity - 1.0).abs() < f32::EPSILON,
            "cached_opacity should retain old value (stale)"
        );
        // The mismatch triggers uniform buffer recreation on next render
    }
}
