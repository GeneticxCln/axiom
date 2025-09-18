//! Real GPU rendering pipeline for Axiom compositor
//!
//! This module implements actual GPU rendering using wgpu to composite
//! windows and effects to the screen - not just stubs.

#![allow(dead_code)]
use anyhow::Result;
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use wgpu::util::DeviceExt;
use wgpu::*;

/// Real GPU rendering pipeline
pub struct AxiomRenderer {
    /// WGPU device for GPU operations
    device: Arc<Device>,
    /// Command queue for GPU commands
    queue: Arc<Queue>,
    /// Window dimensions
    size: (u32, u32),
    /// Rendered windows
    windows: Vec<RenderedWindow>,
    /// Whether this renderer was created with an adapter compatible with the given surface
    surface_compatible: bool,
    /// Pipeline for textured quad rendering (present when surface-backed)
    pipeline: Option<RenderPipeline>,
    /// Bind group layout for sampled textures
    bind_group_layout: Option<BindGroupLayout>,
    /// Default sampler
    sampler: Option<Sampler>,
    /// Surface format in use
    surface_format: Option<TextureFormat>,

    /// Texture reuse pool keyed by (width,height,format)
    texture_pool: HashMap<(u32, u32, TextureFormat), Vec<Texture>>,
    /// Uniform buffer pool for 32-byte window uniforms
    uniform_pool: Vec<Buffer>,

    /// 1x1 white texture and view for solid fills (e.g., shadow quads)
    white_texture: Option<Texture>,
    white_view: Option<TextureView>,
    /// Shared uniform buffer for drawing shadows and solid fills (updated per draw)
    shadow_uniform: Option<Buffer>,
    /// Bind group for shadow/solid drawing (uniform + white texture + sampler)
    shadow_bind_group: Option<BindGroup>,

    /// Overlay fill rectangles for current frame (e.g., decorations)
    overlays: Vec<OverlayRect>,

    /// Optional cache for pipelines by surface format
    pipeline_cache: HashMap<TextureFormat, RenderPipeline>,
}

/// Represents a rendered window surface
#[derive(Debug)]
pub struct RenderedWindow {
    /// Unique window ID
    pub id: u64,
    /// Window position on screen
    pub position: (f32, f32),
    /// Window size (on-screen size)
    pub size: (f32, f32),
    /// Window texture (actual pixel data)
    pub texture: Option<Texture>,
    /// Window texture view for rendering
    pub texture_view: Option<TextureView>,
    /// Per-window uniform buffer
    pub uniform: Option<Buffer>,
    /// Bind group for sampling the texture and uniforms
    pub bind_group: Option<BindGroup>,
    /// Whether window needs redraw
    pub dirty: bool,
    /// Window opacity
    pub opacity: f32,
    /// Corner radius in pixels
    pub corner_radius_px: f32,
    /// Texture pixel size (width,height) if texture exists
    pub tex_size: Option<(u32, u32)>,
    /// Pending damage regions in window-local pixels (x, y, w, h)
    pub damage_regions: Vec<(u32, u32, u32, u32)>,
}

/// Vertex data for rendering quads
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

static RENDER_STATE: OnceLock<Arc<Mutex<SharedRenderState>>> = OnceLock::new();

type Pos = (f32, f32);
type Size = (f32, f32);

#[derive(Default)]
struct SharedRenderState {
    placeholders: HashMap<u64, (Pos, Size, f32)>,
    pending_textures: Vec<(u64, Vec<u8>, u32, u32)>,
    pending_texture_regions: Vec<RegionUpdate>,
    overlay_rects: Vec<OverlayRect>,
}

// Caps for caches and queues to bound memory usage
const MAX_PLACEHOLDERS: usize = 1024;
const MAX_PENDING_TEXTURES: usize = 128;
const MAX_PENDING_REGIONS: usize = 512;

pub fn push_placeholder_quad(id: u64, position: (f32, f32), size: (f32, f32), opacity: f32) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.placeholders.insert(id, (position, size, opacity));
        // Cap total placeholders to avoid unbounded growth
        if s.placeholders.len() > MAX_PLACEHOLDERS {
            if let Some((&victim, _)) = s.placeholders.iter().next() {
                s.placeholders.remove(&victim);
                debug!(
                    "🧹 Evicted placeholder quad {} to respect cap {}",
                    victim, MAX_PLACEHOLDERS
                );
            }
        }
    }
}

pub fn remove_placeholder_quad(id: u64) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.placeholders.remove(&id);
    }
}

pub fn queue_texture_update(id: u64, data: Vec<u8>, width: u32, height: u32) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.pending_textures.push((id, data, width, height));
        if s.pending_textures.len() > MAX_PENDING_TEXTURES {
            // Drop oldest to respect cap
            let dropped = s.pending_textures.remove(0);
            debug!(
                "🧹 Dropped oldest pending texture update for window {} to respect cap {}",
                dropped.0, MAX_PENDING_TEXTURES
            );
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegionUpdate {
    pub id: u64,
    pub full_size: (u32, u32),
    pub rect: (u32, u32, u32, u32), // x, y, w, h
    pub bytes: Vec<u8>,             // tightly packed RGBA for rect (bytes_per_row = 4*w)
}

pub fn queue_texture_update_region(
    id: u64,
    full_w: u32,
    full_h: u32,
    rect: (u32, u32, u32, u32),
    bytes: Vec<u8>,
) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.pending_texture_regions.push(RegionUpdate {
            id,
            full_size: (full_w, full_h),
            rect,
            bytes,
        });
        if s.pending_texture_regions.len() > MAX_PENDING_REGIONS {
            // Drop oldest to respect cap
            if let Some(dropped) = s.pending_texture_regions.first() {
                debug!(
                    "🧹 Dropped oldest pending region update for window {} to respect cap {}",
                    dropped.id, MAX_PENDING_REGIONS
                );
            }
            s.pending_texture_regions.remove(0);
        }
    }
}

#[derive(Clone, Debug)]
pub struct OverlayRect {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 4], // RGBA
    pub radius: f32,
}

pub fn queue_overlay_fill(id: u64, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.overlay_rects.push(OverlayRect { id, x, y, w, h, color, radius: 0.0 });
        // Cap overlays to a reasonable number per frame
        let len = s.overlay_rects.len();
        if len > 4096 {
            let drop_n = len - 4096;
            s.overlay_rects.drain(0..drop_n);
        }
    }
}

pub fn queue_overlay_fill_rounded(id: u64, x: f32, y: f32, w: f32, h: f32, color: [f32; 4], radius: f32) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.overlay_rects.push(OverlayRect { id, x, y, w, h, color, radius });
        let len = s.overlay_rects.len();
        if len > 4096 {
            let drop_n = len - 4096;
            s.overlay_rects.drain(0..drop_n);
        }
    }
}

impl AxiomRenderer {
    /// Create a new real GPU renderer with an actual surface using the provided instance.
    /// If no compatible adapter is found, gracefully falls back to a headless renderer and
    /// marks `surface_compatible = false` so on-screen rendering can be skipped without crashing.
    pub async fn new_with_instance(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        if surface.is_some() {
            info!(
                "🎨 Creating real GPU renderer with surface ({}x{})",
                width, height
            );
        } else {
            info!(
                "🎨 Creating real GPU renderer (no surface) width={} height={}",
                width, height
            );
        }

        // Try to find an adapter compatible with the surface
        let mut chosen_adapter: Option<wgpu::Adapter> = None;
        let surface_for_opts = surface;

        // Detect if system is on battery to bias adapter selection
        let on_battery = Self::detect_on_battery();
        if on_battery {
            info!("🔋 Battery power detected — preferring LowPower GPU adapter");
        } else {
            info!("🔌 AC power detected — preferring HighPerformance GPU adapter");
        }

        let power_order = if on_battery {
            [
                wgpu::PowerPreference::LowPower,
                wgpu::PowerPreference::HighPerformance,
            ]
        } else {
            [
                wgpu::PowerPreference::HighPerformance,
                wgpu::PowerPreference::LowPower,
            ]
        };

        for power in power_order {
            for fallback in [false, true] {
                if let Some(adapter) = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: power,
                        compatible_surface: surface_for_opts,
                        force_fallback_adapter: fallback,
                    })
                    .await
                {
                    chosen_adapter = Some(adapter);
                    break;
                }
            }
            if chosen_adapter.is_some() {
                break;
            }
        }

        // If we couldn't find a surface-compatible adapter, fall back to headless mode
        let (device, queue, surface_compatible, pipeline, bgl, sampler, surface_format) =
            if let Some(adapter) = chosen_adapter {
                info!("🖥️ Using GPU: {}", adapter.get_info().name);

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

                let mut out_pipeline = None;
                let mut out_bgl = None;
                let mut out_sampler = None;
                let mut out_format = None;

                if let Some(s) = surface {
                    // Configure surface with robust defaults
                    let caps = s.get_capabilities(&adapter);
                    let fmts: Vec<String> =
                        caps.formats.iter().map(|f| format!("{:?}", f)).collect();
                    let pmods: Vec<String> = caps
                        .present_modes
                        .iter()
                        .map(|m| format!("{:?}", m))
                        .collect();
                    let amods: Vec<String> = caps
                        .alpha_modes
                        .iter()
                        .map(|a| format!("{:?}", a))
                        .collect();
                    info!(
                    "🧩 Surface capabilities: formats={:?}, present_modes={:?}, alpha_modes={:?}",
                    fmts, pmods, amods
                );

                    let format = caps
                        .formats
                        .iter()
                        .copied()
                        .find(|f| f.is_srgb())
                        .unwrap_or(caps.formats[0]);
                    // Choose present mode: honor AXIOM_PRESENT_MODE if set and supported, else default to FIFO
                    let present_mode = {
                        let override_pm = std::env::var("AXIOM_PRESENT_MODE").ok().map(|s| s.to_lowercase());
                        let pick = match override_pm.as_deref() {
                            Some("mailbox") => Some(wgpu::PresentMode::Mailbox),
                            Some("immediate") => Some(wgpu::PresentMode::Immediate),
                            Some("fifo") => Some(wgpu::PresentMode::Fifo),
                            _ => None,
                        };
                        if let Some(req) = pick {
                            if caps.present_modes.contains(&req) {
                                req
                            } else {
                                warn!("Requested present mode {:?} not supported; falling back", req);
                                // fallback to FIFO if available
                                if caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
                                    wgpu::PresentMode::Fifo
                                } else {
                                    caps.present_modes[0]
                                }
                            }
                        } else if caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
                            wgpu::PresentMode::Fifo
                        } else {
                            caps.present_modes[0]
                        }
                    };
                    let alpha_mode = caps
                        .alpha_modes
                        .iter()
                        .copied()
                        .find(|m| {
                            matches!(
                                m,
                                wgpu::CompositeAlphaMode::Auto | wgpu::CompositeAlphaMode::Opaque
                            )
                        })
                        .unwrap_or(caps.alpha_modes[0]);

                    info!(
                        "🔧 Using format={:?}, present_mode={:?}, alpha_mode={:?}",
                        format, present_mode, alpha_mode
                    );

                    let config = wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format,
                        width,
                        height,
                        present_mode,
                        alpha_mode,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 2,
                    };
                    s.configure(&device, &config);

                    // Create bind group layout and pipeline for textured quad
                    let bind_group_layout =
                        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: Some("Axiom Texture BGL"),
                            entries: &[
                                // binding 0: uniform buffer with WindowUniforms
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Buffer {
                                        ty: wgpu::BufferBindingType::Uniform,
                                        has_dynamic_offset: false,
                                        min_binding_size: std::num::NonZeroU64::new(32),
                                    },
                                    count: None,
                                },
                                // binding 1: texture
                                wgpu::BindGroupLayoutEntry {
                                    binding: 1,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Texture {
                                        sample_type: wgpu::TextureSampleType::Float {
                                            filterable: true,
                                        },
                                        view_dimension: wgpu::TextureViewDimension::D2,
                                        multisampled: false,
                                    },
                                    count: None,
                                },
                                // binding 2: sampler
                                wgpu::BindGroupLayoutEntry {
                                    binding: 2,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Sampler(
                                        wgpu::SamplerBindingType::Filtering,
                                    ),
                                    count: None,
                                },
                            ],
                        });

                    let pipeline_layout =
                        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("Axiom Pipeline Layout"),
                            bind_group_layouts: &[&bind_group_layout],
                            push_constant_ranges: &[],
                        });

                    let shader_src = include_str!("./textured_quad.wgsl");
                    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("Axiom Textured Quad Shader"),
                        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
                    });

                    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("Axiom Textured Quad Pipeline"),
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
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        multiview: None,
                    });

                    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("Axiom Default Sampler"),
                        address_mode_u: wgpu::AddressMode::ClampToEdge,
                        address_mode_v: wgpu::AddressMode::ClampToEdge,
                        address_mode_w: wgpu::AddressMode::ClampToEdge,
                        mag_filter: wgpu::FilterMode::Linear,
                        min_filter: wgpu::FilterMode::Linear,
                        mipmap_filter: wgpu::FilterMode::Linear,
                        ..Default::default()
                    });

                    out_pipeline = Some(pipeline);
                    out_bgl = Some(bind_group_layout);
                    out_sampler = Some(sampler);
                    out_format = Some(format);
                }

                (
                    device,
                    queue,
                    surface.is_some(),
                    out_pipeline,
                    out_bgl,
                    out_sampler,
                    out_format,
                )
            } else {
                warn!(
                "⚠️ No surface-compatible GPU adapter found. Falling back to headless renderer; on-screen presentation will be disabled."
            );
                let headless = Self::new_headless().await?;
                return Ok(Self {
                    device: headless.device,
                    queue: headless.queue,
                    size: (width, height),
                    windows: Vec::new(),
                    surface_compatible: false,
                    pipeline: None,
                    bind_group_layout: None,
                    sampler: None,
                    surface_format: None,
                    white_texture: None,
                    white_view: None,
                    shadow_uniform: None,
                    shadow_bind_group: None,
                    overlays: Vec::new(),
                    pipeline_cache: HashMap::new(),
                    texture_pool: HashMap::new(),
                    uniform_pool: Vec::new(),
                });
            };

        info!("✅ GPU renderer initialized successfully");

        // Create 1x1 white texture for solid fills
        let mut white_texture = None;
        let mut white_view = None;
        let mut shadow_uniform = None;
        let mut shadow_bind_group = None;
        if let (Some(bgl), Some(sampler)) = (&bgl, &sampler) {
            let tex = device.create_texture(&TextureDescriptor {
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8UnormSrgb,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                label: Some("Axiom White 1x1"),
                view_formats: &[],
            });
            let view = tex.create_view(&TextureViewDescriptor::default());
            queue.write_texture(
                ImageCopyTexture {
                    aspect: TextureAspect::All,
                    texture: &tex,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                },
                &[255u8, 255, 255, 255],
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Axiom Shadow Uniform"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bgroup = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Axiom Shadow BindGroup"),
                layout: bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            white_texture = Some(tex);
            white_view = Some(view);
            shadow_uniform = Some(ubuf);
            shadow_bind_group = Some(bgroup);
        }

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            size: (width, height),
            windows: Vec::new(),
            surface_compatible,
            pipeline,
            bind_group_layout: bgl,
            sampler,
            surface_format,
            white_texture,
            white_view,
            shadow_uniform,
            shadow_bind_group,
            overlays: Vec::new(),
            pipeline_cache: HashMap::new(),
            texture_pool: HashMap::new(),
            uniform_pool: Vec::new(),
        })
    }

    /// Backward-compatible constructor; may fall back to headless if given surface was created
    /// from a different Instance and no compatible adapter is found.
    pub async fn new(surface: &wgpu::Surface<'_>, width: u32, height: u32) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        Self::new_with_instance(&instance, Some(surface), width, height).await
    }

    /// Create a headless renderer for testing with specified backends
    pub async fn new_headless_with_backends(backends: wgpu::Backends) -> Result<Self> {
        info!(
            "🎨 Creating headless GPU renderer for testing (backends={:?})",
            backends
        );

        // Create wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
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

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            size: (1920, 1080), // Default size for headless
            windows: Vec::new(),
            surface_compatible: false,
            pipeline: None,
            bind_group_layout: None,
            sampler: None,
            surface_format: None,
            white_texture: None,
            white_view: None,
            shadow_uniform: None,
            shadow_bind_group: None,
            overlays: Vec::new(),
            pipeline_cache: HashMap::new(),
            texture_pool: HashMap::new(),
            uniform_pool: Vec::new(),
        })
    }

    /// Create a headless renderer with default backends (all)
    pub async fn new_headless() -> Result<Self> {
        Self::new_headless_with_backends(wgpu::Backends::all()).await
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
            uniform: None,
            bind_group: None,
            dirty: true,
            opacity: 1.0,
            corner_radius_px: 8.0,
            tex_size: None,
            damage_regions: Vec::new(),
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
            // Only mark dirty if something actually changed
            if w.position != position
                || w.size != size
                || (w.opacity - opacity).abs() > f32::EPSILON
            {
                w.position = position;
                w.size = size;
                w.opacity = opacity;
                w.dirty = true;
            }
        } else {
            let window = RenderedWindow {
                id,
                position,
                size,
                texture: None,
                texture_view: None,
                uniform: None,
                bind_group: None,
                dirty: true,
                opacity,
                corner_radius_px: 8.0,
                tex_size: None,
                damage_regions: Vec::new(),
            };
            self.windows.push(window);
        }
    }

    /// Update window texture with actual pixel data
    pub fn update_window_texture(
        &mut self,
        window_id: u64,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        debug!(
            "🖼️ Updating texture for window {} ({}x{})",
            window_id, width, height
        );

        if let Some(window) = self.windows.iter_mut().find(|w| w.id == window_id) {
            // If texture exists and same size, just overwrite contents; else recreate
            let recreate = match window.tex_size {
                Some((tw, th)) => !(tw == width && th == height),
                None => true,
            };
            if recreate {
                // Try reuse pool first
                let key = (width, height, TextureFormat::Rgba8UnormSrgb);
                let texture = if let Some(vec) = self.texture_pool.get_mut(&key) {
                    vec.pop().unwrap_or_else(|| {
                        self.device.create_texture(&TextureDescriptor {
                            size: Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: TextureDimension::D2,
                            format: TextureFormat::Rgba8UnormSrgb,
                            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                            label: Some(&format!("Window {} Texture", window_id)),
                            view_formats: &[],
                        })
                    })
                } else {
                    self.device.create_texture(&TextureDescriptor {
                        size: Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format: TextureFormat::Rgba8UnormSrgb,
                        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                        label: Some(&format!("Window {} Texture", window_id)),
                        view_formats: &[],
                    })
                };
                let texture_view = texture.create_view(&TextureViewDescriptor::default());
                window.texture = Some(texture);
                window.texture_view = Some(texture_view);
                window.uniform = None;
                window.bind_group = None;
                window.tex_size = Some((width, height));
            }
            // Upload pixel data to GPU
            let texture_ref = window.texture.as_ref().unwrap();
            self.queue.write_texture(
                ImageCopyTexture {
                    aspect: TextureAspect::All,
                    texture: texture_ref,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                },
                data,
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            window.dirty = true;
            window.damage_regions.clear();
            info!("✅ Updated texture for window {}", window_id);
        }

        Ok(())
    }

    /// Update only a region of the window texture. Creates texture if missing.
    pub fn update_window_texture_region(
        &mut self,
        window_id: u64,
        full_width: u32,
        full_height: u32,
        rect: (u32, u32, u32, u32),
        bytes: &[u8],
    ) -> Result<()> {
        if let Some(window) = self.windows.iter_mut().find(|w| w.id == window_id) {
            // Ensure texture exists and is correct size
            let recreate = match window.tex_size {
                Some((tw, th)) => !(tw == full_width && th == full_height),
                None => true,
            };
            if recreate {
                let texture = self.device.create_texture(&TextureDescriptor {
                    size: Extent3d {
                        width: full_width,
                        height: full_height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8UnormSrgb,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    label: Some(&format!("Window {} Texture", window_id)),
                    view_formats: &[],
                });
                let texture_view = texture.create_view(&TextureViewDescriptor::default());
                window.texture = Some(texture);
                window.texture_view = Some(texture_view);
                window.uniform = None;
                window.bind_group = None;
                window.tex_size = Some((full_width, full_height));
            }
            let texture_ref = window.texture.as_ref().unwrap();
            let (x, y, w, h) = rect;
            self.queue.write_texture(
                ImageCopyTexture {
                    aspect: TextureAspect::All,
                    texture: texture_ref,
                    mip_level: 0,
                    origin: Origin3d { x, y, z: 0 },
                },
                bytes,
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
                Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            window.dirty = true;
            window.damage_regions.push(rect);
        }
        Ok(())
    }

    /// Render all windows (simplified for now - needs actual surface)
    pub fn render(&mut self) -> Result<()> {
        info!("🎨 Rendering {} windows to GPU", self.windows.len());

        // For now, just validate that we have the GPU device and queue
        // In a real implementation, this would render to an actual surface

        for window in &self.windows {
            if window.texture_view.is_some() {
                debug!("✅ Would render window {} with texture", window.id);
            } else {
                debug!(
                    "🟦 Rendering placeholder quad for window {} at ({:.1},{:.1}) size {:.1}x{:.1} opacity {:.2}",
                    window.id, window.position.0, window.position.1, window.size.0, window.size.1, window.opacity
                );
            }
        }

        debug!("🖥️ Frame rendered with {} windows", self.windows.len());
        // Clear damage and reset dirty flags after a successful draw (headless)
        for win in &mut self.windows {
            win.damage_regions.clear();
            win.dirty = false;
        }
        Ok(())
    }

    /// Render all windows to a wgpu surface (real rendering)
    pub fn render_to_surface(
        &mut self,
        _surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
    ) -> Result<()> {
        self.render_to_surface_with_outputs_scaled(_surface, surface_texture, &[], false)
    }

    /// Backwards-compat wrapper without per-output scales (assumes scale=1)
    pub fn render_to_surface_with_outputs(
        &mut self,
        _surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
        outputs: &[(u32, u32, u32, u32)],
        debug_overlay: bool,
    ) -> Result<()> {
        let with_scales: Vec<(u32, u32, u32, u32, i32)> = outputs
            .iter()
            .map(|&(x, y, w, h)| (x, y, w, h, 1))
            .collect();
        self.render_to_surface_with_outputs_scaled(_surface, surface_texture, &with_scales, debug_overlay)
    }

    /// Render all windows to a wgpu surface with per-output scissor rectangles
    /// If outputs is empty, renders normally across full surface.
    pub fn render_to_surface_with_outputs_scaled(
        &mut self,
        _surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
        outputs: &[(u32, u32, u32, u32, i32)],
        debug_overlay: bool,
    ) -> Result<()> {
        if !self.surface_compatible {
            // Graceful no-op: we don't have a surface-compatible device, skip rendering
            debug!("🚫 Skipping surface render (no compatible adapter); presenting empty frame");
            return Ok(());
        }
        let use_outputs = !outputs.is_empty();
        debug!("🎨 Rendering {} windows to surface", self.windows.len());

        // Create render pass
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        if let (Some(pipeline_ref), Some(bgl), Some(sampler)) =
            (self.pipeline.as_ref().or_else(|| self.surface_format.and_then(|f| self.pipeline_cache.get(&f))), &self.bind_group_layout, &self.sampler)
        {
            let pipeline = pipeline_ref;
            // Pre-allocate buffers once per frame for all windows and draw
            // Flatten all quads into a single draw list
            let mut all_vertices: Vec<Vertex> = Vec::new();
            let mut all_indices: Vec<u16> = Vec::new();

            #[derive(Copy, Clone)]
            enum DrawKind {
                Shadow {
                    widx: usize,
                    scissor: (u32, u32, u32, u32),
                },
                Window {
                    widx: usize,
                },
                SolidFill {
                    scissor: (u32, u32, u32, u32),
                    color: [f32; 4],
                    radius: f32,
                },
            }
            let mut draw_cmds: Vec<(DrawKind, u32)> = Vec::new();

            // Optional: add debug overlay quads for each output rectangle
            if use_outputs && debug_overlay {
                // Build thin border quads (top, bottom, left, right) for each output
                let border: f32 = 2.0;
                for &(ox, oy, ow, oh, _scale) in outputs {
                    let fw = self.size.0 as f32;
                    let fh = self.size.1 as f32;
                    let oxf = ox as f32;
                    let oyf = oy as f32;
                    let owf = ow as f32;
                    let ohf = oh as f32;
                    let mut push_rect = |x: f32, y: f32, w: f32, h: f32| {
                        let x0 = (x / fw) * 2.0 - 1.0;
                        let y0 = 1.0 - (y / fh) * 2.0;
                        let x1 = ((x + w) / fw) * 2.0 - 1.0;
                        let y1 = 1.0 - ((y + h) / fh) * 2.0;
                        let base = all_vertices.len() as u16;
                        all_vertices.extend_from_slice(&[
                            Vertex {
                                position: [x0, y1, 0.0],
                                tex_coords: [0.0, 1.0],
                            },
                            Vertex {
                                position: [x1, y1, 0.0],
                                tex_coords: [1.0, 1.0],
                            },
                            Vertex {
                                position: [x0, y0, 0.0],
                                tex_coords: [0.0, 0.0],
                            },
                            Vertex {
                                position: [x1, y0, 0.0],
                                tex_coords: [1.0, 0.0],
                            },
                        ]);
                        let first = all_indices.len() as u32;
                        all_indices.extend_from_slice(&[
                            base,
                            base + 1,
                            base + 2,
                            base + 2,
                            base + 1,
                            base + 3,
                        ]);
                        // Scissor to exactly the rect area
                        let sx = x.floor().max(0.0) as u32;
                        let sy = y.floor().max(0.0) as u32;
                        let sw = (w.ceil() as u32).min(self.size.0.saturating_sub(sx));
                        let sh = (h.ceil() as u32).min(self.size.1.saturating_sub(sy));
                        draw_cmds.push((
                            DrawKind::Shadow {
                                widx: 0,
                                scissor: (sx, sy, sw, sh),
                            },
                            first,
                        ));
                    };
                    // Top
                    push_rect(oxf, oyf, owf, border);
                    // Bottom
                    push_rect(oxf, oyf + ohf - border, owf, border);
                    // Left
                    push_rect(oxf, oyf, border, ohf);
                    // Right
                    push_rect(oxf + owf - border, oyf, border, ohf);
                }
            }

            let shadow_spread: f32 = 12.0;
            let shadow_offset: (f32, f32) = (4.0, 6.0);

            for widx in 0..self.windows.len() {
                // Short mutable borrow to ensure bind_group exists and fetch geometry
                let geo = {
                    let window = &mut self.windows[widx];
                    if let Some(tex_view) = &window.texture_view {
                        // Ensure uniform buffer exists and is updated
                        if window.uniform.is_none() {
                            // reuse uniform pool if possible
                            let ubuf = if let Some(buf) = self.uniform_pool.pop() {
                                buf
                            } else {
                                self.device.create_buffer(&wgpu::BufferDescriptor {
                                    label: Some("Axiom Window Uniform"),
                                    size: 32,
                                    usage: wgpu::BufferUsages::UNIFORM
                                        | wgpu::BufferUsages::COPY_DST,
                                    mapped_at_creation: false,
                                })
                            };
                            window.uniform = Some(ubuf);
                        }
                        // Write uniform params for window
                        let params: [f32; 4] = [
                            window.opacity,
                            window.corner_radius_px,
                            window.size.0.max(1.0),
                            window.size.1.max(1.0),
                        ];
                        let params2: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
                        if let Some(ubuf) = &window.uniform {
                            self.queue
                                .write_buffer(ubuf, 0, bytemuck::cast_slice(&params));
                            self.queue
                                .write_buffer(ubuf, 16, bytemuck::cast_slice(&params2));
                        }
                        if window.bind_group.is_none() {
                            window.bind_group = Some(
                                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("Axiom Window BindGroup"),
                                    layout: bgl,
                                    entries: &[
                                        wgpu::BindGroupEntry {
                                            binding: 0,
                                            resource: window
                                                .uniform
                                                .as_ref()
                                                .unwrap()
                                                .as_entire_binding(),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 1,
                                            resource: wgpu::BindingResource::TextureView(tex_view),
                                        },
                                        wgpu::BindGroupEntry {
                                            binding: 2,
                                            resource: wgpu::BindingResource::Sampler(sampler),
                                        },
                                    ],
                                }),
                            );
                        }
                        Some((window.position, window.size))
                    } else {
                        None
                    }
                };
                if let Some((position, size)) = geo {
                    // Build shadow quad (expanded by spread and offset) and then window quad, both in clip space
                    let (x, y) = position;
                    let (w, h) = size;
                    let fw = self.size.0 as f32;
                    let fh = self.size.1 as f32;

                    // Shadow rect in pixels
                    let sx_px = (x - shadow_spread + shadow_offset.0).max(0.0);
                    let sy_px = (y - shadow_spread + shadow_offset.1).max(0.0);
                    let sw_px = (w + 2.0 * shadow_spread).min(fw);
                    let sh_px = (h + 2.0 * shadow_spread).min(fh);

                    // Compute scissor rect for shadow in framebuffer space
                    let scissor_x = sx_px.floor().max(0.0) as u32;
                    let scissor_y = sy_px.floor().max(0.0) as u32;
                    let scissor_w =
                        (sw_px.ceil() as u32).min(self.size.0.saturating_sub(scissor_x));
                    let scissor_h =
                        (sh_px.ceil() as u32).min(self.size.1.saturating_sub(scissor_y));

                    let sx0 = (sx_px / fw) * 2.0 - 1.0;
                    let sy0 = 1.0 - (sy_px / fh) * 2.0;
                    let sx1 = ((sx_px + sw_px) / fw) * 2.0 - 1.0;
                    let sy1 = 1.0 - ((sy_px + sh_px) / fh) * 2.0;

                    let s_base = all_vertices.len() as u16;
                    all_vertices.extend_from_slice(&[
                        Vertex {
                            position: [sx0, sy1, 0.0],
                            tex_coords: [0.0, 1.0],
                        },
                        Vertex {
                            position: [sx1, sy1, 0.0],
                            tex_coords: [1.0, 1.0],
                        },
                        Vertex {
                            position: [sx0, sy0, 0.0],
                            tex_coords: [0.0, 0.0],
                        },
                        Vertex {
                            position: [sx1, sy0, 0.0],
                            tex_coords: [1.0, 0.0],
                        },
                    ]);
                    let s_first = all_indices.len() as u32;
                    all_indices.extend_from_slice(&[
                        s_base,
                        s_base + 1,
                        s_base + 2,
                        s_base + 2,
                        s_base + 1,
                        s_base + 3,
                    ]);
                    draw_cmds.push((
                        DrawKind::Shadow {
                            widx,
                            scissor: (scissor_x, scissor_y, scissor_w, scissor_h),
                        },
                        s_first,
                    ));

                    // Pixel snapping for window quad to reduce sampling artifacts
                    let x_px = x.round().clamp(0.0, fw);
                    let y_px = y.round().clamp(0.0, fh);
                    let w_px = (w.round()).min(fw - x_px).max(0.0);
                    let h_px = (h.round()).min(fh - y_px).max(0.0);
                    // Window rect in clip space
                    let x0 = (x_px / fw) * 2.0 - 1.0;
                    let y0 = 1.0 - (y_px / fh) * 2.0;
                    let x1 = ((x_px + w_px) / fw) * 2.0 - 1.0;
                    let y1 = 1.0 - ((y_px + h_px) / fh) * 2.0;

                    let base_index = all_vertices.len() as u16;
                    all_vertices.extend_from_slice(&[
                        Vertex {
                            position: [x0, y1, 0.0],
                            tex_coords: [0.0, 1.0],
                        },
                        Vertex {
                            position: [x1, y1, 0.0],
                            tex_coords: [1.0, 1.0],
                        },
                        Vertex {
                            position: [x0, y0, 0.0],
                            tex_coords: [0.0, 0.0],
                        },
                        Vertex {
                            position: [x1, y0, 0.0],
                            tex_coords: [1.0, 0.0],
                        },
                    ]);
                    let first_index = all_indices.len() as u32;
                    all_indices.extend_from_slice(&[
                        base_index,
                        base_index + 1,
                        base_index + 2,
                        base_index + 2,
                        base_index + 1,
                        base_index + 3,
                    ]);
                    draw_cmds.push((DrawKind::Window { widx }, first_index));
                }
            }

            // Append overlay fill quads (e.g., decorations) after windows to ensure they appear on top
            let mut push_overlay_rect = |x: f32, y: f32, w: f32, h: f32, color: [f32; 4], radius: f32| {
                let fw = self.size.0 as f32;
                let fh = self.size.1 as f32;
                let x0 = (x / fw) * 2.0 - 1.0;
                let y0 = 1.0 - (y / fh) * 2.0;
                let x1 = ((x + w) / fw) * 2.0 - 1.0;
                let y1 = 1.0 - ((y + h) / fh) * 2.0;
                let base = all_vertices.len() as u16;
                all_vertices.extend_from_slice(&[
                    Vertex { position: [x0, y1, 0.0], tex_coords: [0.0, 1.0] },
                    Vertex { position: [x1, y1, 0.0], tex_coords: [1.0, 1.0] },
                    Vertex { position: [x0, y0, 0.0], tex_coords: [0.0, 0.0] },
                    Vertex { position: [x1, y0, 0.0], tex_coords: [1.0, 0.0] },
                ]);
                let first = all_indices.len() as u32;
                all_indices.extend_from_slice(&[base, base+1, base+2, base+2, base+1, base+3]);
                // Compute scissor rect in framebuffer space
                let sx = x.floor().max(0.0) as u32;
                let sy = y.floor().max(0.0) as u32;
                let sw = (w.ceil() as u32).min(self.size.0.saturating_sub(sx));
                let sh = (h.ceil() as u32).min(self.size.1.saturating_sub(sy));
                draw_cmds.push((DrawKind::SolidFill { scissor: (sx, sy, sw, sh), color, radius }, first));
            };
            for ov in std::mem::take(&mut self.overlays) {
                push_overlay_rect(ov.x, ov.y, ov.w, ov.h, ov.color, ov.radius);
            }

            if !all_vertices.is_empty() {
                let vbuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Axiom Quad Verts Batch"),
                        contents: bytemuck::cast_slice(&all_vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let ibuf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Axiom Quad Indices Batch"),
                        contents: bytemuck::cast_slice(&all_indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });

                {
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Axiom Render Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.05,
                                    g: 0.05,
                                    b: 0.06,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        occlusion_query_set: None,
                        timestamp_writes: None,
                    });

                    rpass.set_pipeline(pipeline);
                    rpass.set_vertex_buffer(0, vbuf.slice(..));
                    rpass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint16);

                    let shadow_opacity: f32 = 0.3;

                    for (kind, first_index) in draw_cmds.iter().copied() {
                        match kind {
                            DrawKind::Shadow { widx, scissor } => {
                                if let (Some(shadow_bg), Some(sh_ubuf)) =
                                    (&self.shadow_bind_group, &self.shadow_uniform)
                                {
                                    // Snapshot size for uniforms
                                    let wsize = self.windows[widx].size;
                                    let sh_params: [f32; 4] = [
                                        shadow_opacity,
                                        8.0,
                                        (wsize.0 + 2.0 * shadow_spread).max(1.0),
                                        (wsize.1 + 2.0 * shadow_spread).max(1.0),
                                    ];
                                    let sh_params2: [f32; 4] =
                                        [1.0, shadow_spread, shadow_offset.0, shadow_offset.1];
                                    self.queue.write_buffer(
                                        sh_ubuf,
                                        0,
                                        bytemuck::cast_slice(&sh_params),
                                    );
                                    self.queue.write_buffer(
                                        sh_ubuf,
                                        16,
                                        bytemuck::cast_slice(&sh_params2),
                                    );

                                    if use_outputs {
                                        for &(ox, oy, ow, oh, _scale) in outputs {
                                            let ix = ox.max(scissor.0);
                                            let iy = oy.max(scissor.1);
                                            let ix2 = ox
                                                .saturating_add(ow)
                                                .min(scissor.0.saturating_add(scissor.2));
                                            let iy2 = oy
                                                .saturating_add(oh)
                                                .min(scissor.1.saturating_add(scissor.3));
                                            let iw = ix2.saturating_sub(ix);
                                            let ih = iy2.saturating_sub(iy);
                                            if iw == 0 || ih == 0 {
                                                continue;
                                            }
                                            rpass.set_scissor_rect(ix, iy, iw, ih);
                                            rpass.set_bind_group(0, shadow_bg, &[]);
                                            rpass.draw_indexed(
                                                first_index..first_index + 6,
                                                0,
                                                0..1,
                                            );
                                        }
                                    } else {
                                        rpass.set_scissor_rect(
                                            scissor.0, scissor.1, scissor.2, scissor.3,
                                        );
                                        rpass.set_bind_group(0, shadow_bg, &[]);
                                        rpass.draw_indexed(first_index..first_index + 6, 0, 0..1);
                                    }
                                }
                            }
                            DrawKind::Window { widx } => {
                                // Snapshot needed state for scissor regions
                                let (bind_group, wxu, wyu, regions) = {
                                    let win = &self.windows[widx];
                                    (
                                        win.bind_group.as_ref().expect("bind group set"),
                                        win.position.0 as u32,
                                        win.position.1 as u32,
                                        win.damage_regions.clone(),
                                    )
                                };

                                rpass.set_bind_group(0, bind_group, &[]);
                                if regions.is_empty() {
                                    if use_outputs {
for &(ox, oy, ow, oh, _scale) in outputs {
                                            // Intersect full framebuffer with output rect => output rect
                                            rpass.set_scissor_rect(ox, oy, ow, oh);
                                            rpass.draw_indexed(
                                                first_index..first_index + 6,
                                                0,
                                                0..1,
                                            );
                                        }
                                    } else {
                                        rpass.set_scissor_rect(0, 0, self.size.0, self.size.1);
                                        rpass.draw_indexed(first_index..first_index + 6, 0, 0..1);
                                    }
                                } else {
                                    for (dx, dy, dw, dh) in regions.into_iter() {
                                        let sx = wxu.saturating_add(dx);
                                        let sy = wyu.saturating_add(dy);
                                        let sw = dw.min(self.size.0.saturating_sub(sx));
                                        let sh = dh.min(self.size.1.saturating_sub(sy));
                                        if sw == 0 || sh == 0 {
                                            continue;
                                        }
                                        if use_outputs {
                                            for &(ox, oy, ow, oh, _scale) in outputs {
                                                let ix = ox.max(sx);
                                                let iy = oy.max(sy);
                                                let ix2 = ox
                                                    .saturating_add(ow)
                                                    .min(sx.saturating_add(sw));
                                                let iy2 = oy
                                                    .saturating_add(oh)
                                                    .min(sy.saturating_add(sh));
                                                let iw = ix2.saturating_sub(ix);
                                                let ih = iy2.saturating_sub(iy);
                                                if iw == 0 || ih == 0 {
                                                    continue;
                                                }
                                                rpass.set_scissor_rect(ix, iy, iw, ih);
                                                rpass.draw_indexed(
                                                    first_index..first_index + 6,
                                                    0,
                                                    0..1,
                                                );
                                            }
                                        } else {
                                            rpass.set_scissor_rect(sx, sy, sw, sh);
                                            rpass.draw_indexed(
                                                first_index..first_index + 6,
                                                0,
                                                0..1,
                                            );
                                        }
                                    }
                                }
                            }
                            DrawKind::SolidFill { scissor, color, radius } => {
                                if let (Some(shadow_bg), Some(sh_ubuf)) = (&self.shadow_bind_group, &self.shadow_uniform) {
                                    // Encode solid fill via mode=2 and color in params2.yzw; alpha in params.x
                                    let w_px = scissor.2 as f32;
                                    let h_px = scissor.3 as f32;
                                    let fill_params: [f32; 4] = [color[3], radius, w_px, h_px];
                                    let mode_val = if radius > 0.0 { 3.0 } else { 2.0 };
                                    let fill_params2: [f32; 4] = [mode_val, color[0], color[1], color[2]];
                                    self.queue.write_buffer(sh_ubuf, 0, bytemuck::cast_slice(&fill_params));
                                    self.queue.write_buffer(sh_ubuf, 16, bytemuck::cast_slice(&fill_params2));
                                    if use_outputs {
                                        for &(ox, oy, ow, oh, _scale) in outputs {
                                            let ix = ox.max(scissor.0);
                                            let iy = oy.max(scissor.1);
                                            let ix2 = ox.saturating_add(ow).min(scissor.0.saturating_add(scissor.2));
                                            let iy2 = oy.saturating_add(oh).min(scissor.1.saturating_add(scissor.3));
                                            let iw = ix2.saturating_sub(ix);
                                            let ih = iy2.saturating_sub(iy);
                                            if iw == 0 || ih == 0 { continue; }
                                            rpass.set_scissor_rect(ix, iy, iw, ih);
                                            rpass.set_bind_group(0, shadow_bg, &[]);
                                            rpass.draw_indexed(first_index..first_index+6, 0, 0..1);
                                        }
                                    } else {
                                        rpass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
                                        rpass.set_bind_group(0, shadow_bg, &[]);
                                        rpass.draw_indexed(first_index..first_index+6, 0, 0..1);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Submit commands to GPU
        self.queue.submit(std::iter::once(encoder.finish()));

        // Clear damage and reset dirty flags after a successful draw
        for win in &mut self.windows {
            win.damage_regions.clear();
            win.dirty = false;
        }

        info!("✅ Rendered {} windows to surface", self.windows.len());
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

    /// Get number of rendered windows
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// Whether this renderer can present to the given surface
    pub fn can_present(&self) -> bool {
        self.surface_compatible
    }

    /// Whether there is any pending damage/dirty content to render
    pub fn has_dirty(&self) -> bool {
        self.windows
            .iter()
            .any(|w| w.dirty || !w.damage_regions.is_empty())
    }

    /// Trim textures for windows that are not in the provided keep-set (e.g., unmapped or offscreen)
    /// Returns the number of textures trimmed.
    pub fn trim_textures_except(&mut self, keep_ids: &HashSet<u64>) -> usize {
        let mut trimmed = 0;
        for w in &mut self.windows {
            if !keep_ids.contains(&w.id)
                && (w.texture.is_some()
                    || w.texture_view.is_some()
                    || w.bind_group.is_some()
                    || w.tex_size.is_some())
            {
                // Return resources to pools where possible
                if let (Some(tex), Some((tw, th))) = (w.texture.take(), w.tex_size.take()) {
                    let key = (tw, th, TextureFormat::Rgba8UnormSrgb);
                    self.texture_pool.entry(key).or_default().push(tex);
                }
                if let Some(ubuf) = w.uniform.take() {
                    self.uniform_pool.push(ubuf);
                }
                w.texture_view = None;
                w.bind_group = None;
                trimmed += 1;
            }
        }
        if trimmed > 0 {
            debug!(
                "🧹 Trimmed {} window textures not currently visible",
                trimmed
            );
        }
        trimmed
    }

    /// Sync renderer state from shared placeholders and pending textures
    pub fn sync_from_shared(&mut self) {
        let mut keep_ids: Option<HashSet<u64>> = None;
        if let Some(state) = RENDER_STATE.get() {
            if let Ok(mut s) = state.lock() {
                // Capture the set of IDs that currently have placeholders (visible/mapped)
                let mut ks = HashSet::new();
                for id in s.placeholders.keys() {
                    ks.insert(*id);
                }
                keep_ids = Some(ks);

                for (id, (pos, size, opacity)) in s.placeholders.iter() {
                    self.upsert_window_rect(*id, *pos, *size, *opacity);
                }
                let updates: Vec<_> = s.pending_textures.drain(..).collect();
                let region_updates: Vec<_> = s.pending_texture_regions.drain(..).collect();
                let overlays: Vec<_> = s.overlay_rects.drain(..).collect();
                drop(s);
                // Batch flush updates using a single encoder
                let _ = self.flush_batched_texture_updates(updates, region_updates);
                // Update overlays to be drawn on next render
                self.overlays = overlays;
            }
        }
        // After syncing, trim textures that are not currently referenced by placeholders
        if let Some(keep) = keep_ids {
            let _ = self.trim_textures_except(&keep);
        }
    }

    /// Start a simple headless render loop at ~60 FPS for development
    pub async fn start_headless_loop_with_backends(
        backends: wgpu::Backends,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let mut renderer = Self::new_headless_with_backends(backends).await?;
        // Initialize shared render state if not already
        let _ = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
        info!("🖥️ Starting headless render loop (~60 FPS)");

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(16));
            loop {
                ticker.tick().await;
                // Sync placeholders into renderer's window list
                if let Some(state) = RENDER_STATE.get() {
                    if let Ok(mut s) = state.lock() {
                        for (id, (pos, size, opacity)) in s.placeholders.iter() {
                            renderer.upsert_window_rect(*id, *pos, *size, *opacity);
                        }
                        // Drain pending textures into renderer and batch copy
                        let updates: Vec<_> = s.pending_textures.drain(..).collect();
                        let region_updates: Vec<_> = s.pending_texture_regions.drain(..).collect();
                        drop(s);
                        let _ = renderer.flush_batched_texture_updates(updates, region_updates);
                    }
                }
                if let Err(e) = renderer.render() {
                    debug!("render error: {}", e);
                }
            }
        });

        Ok(handle)
    }

    /// Backwards-compatible headless loop using all backends
    pub async fn start_headless_loop() -> Result<tokio::task::JoinHandle<()>> {
        Self::start_headless_loop_with_backends(wgpu::Backends::all()).await
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

impl AxiomRenderer {
    /// Batch flush pending texture updates and regions using a single encoder with staging buffers
    fn flush_batched_texture_updates(
        &mut self,
        updates: Vec<(u64, Vec<u8>, u32, u32)>,
        region_updates: Vec<RegionUpdate>,
    ) -> Result<()> {
        if updates.is_empty() && region_updates.is_empty() {
            return Ok(());
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Axiom Texture Uploads") });

        // Helper: ensure texture exists and size matches
        let ensure_texture = |win: &mut RenderedWindow, w: u32, h: u32| {
            let recreate = match win.tex_size { Some((tw, th)) => !(tw == w && th == h), None => true };
            if recreate {
                let texture = self.device.create_texture(&TextureDescriptor {
                    size: Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8UnormSrgb,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    label: Some("Axiom Window Texture"),
                    view_formats: &[],
                });
                win.texture = Some(texture);
                win.texture_view = win.texture.as_ref().map(|t| t.create_view(&TextureViewDescriptor::default()));
                win.uniform = None;
                win.bind_group = None;
                win.tex_size = Some((w, h));
            }
        };

        // Full updates
        for (id, data, w, h) in updates.into_iter() {
            if let Some(win) = self.windows.iter_mut().find(|w| w.id == id) {
                ensure_texture(win, w, h);
                let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Axiom Staging Full"),
                    contents: &data,
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
                let texture_ref = win.texture.as_ref().unwrap();
                encoder.copy_buffer_to_texture(
                    wgpu::ImageCopyBuffer {
                        buffer: &buf,
                        layout: wgpu::ImageDataLayout {
                            offset: 0,
bytes_per_row: Some(4 * w),
rows_per_image: Some(h),
                        },
                    },
                    wgpu::ImageCopyTexture {
                        texture: texture_ref,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: TextureAspect::All,
                    },
                    Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                win.dirty = true;
                win.damage_regions.clear();
            }
        }

        // Region updates
        for up in region_updates.into_iter() {
            if let Some(win) = self.windows.iter_mut().find(|w| w.id == up.id) {
                ensure_texture(win, up.full_size.0, up.full_size.1);
                let (x, y, w, h) = up.rect;
                let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Axiom Staging Region"),
                    contents: &up.bytes,
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
                let texture_ref = win.texture.as_ref().unwrap();
                encoder.copy_buffer_to_texture(
                    wgpu::ImageCopyBuffer {
                        buffer: &buf,
                        layout: wgpu::ImageDataLayout {
                            offset: 0,
bytes_per_row: Some(4 * w),
rows_per_image: Some(h),
                        },
                    },
                    wgpu::ImageCopyTexture {
                        texture: texture_ref,
                        mip_level: 0,
                        origin: Origin3d { x, y, z: 0 },
                        aspect: TextureAspect::All,
                    },
                    Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                win.dirty = true;
                win.damage_regions.push(up.rect);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// Detect whether the system is currently on battery power (Linux power_supply sysfs)
    fn detect_on_battery() -> bool {
        // Best-effort: look for BAT* devices and check status
        if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
            let mut found_battery = false;
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    if name.starts_with("BAT") {
                        found_battery = true;
                        let status_path = entry.path().join("status");
                        if let Ok(s) = fs::read_to_string(status_path) {
                            let sl = s.trim().to_lowercase();
                            if sl.contains("discharging") {
                                return true;
                            }
                        }
                    }
                    if name.starts_with("AC")
                        || name.to_lowercase().contains("ac")
                        || name.to_lowercase().contains("ac_adapter")
                    {
                        // If AC online is present and set, assume not on battery
                        let online_path = entry.path().join("online");
                        if let Ok(s) = fs::read_to_string(online_path) {
                            if s.trim() == "1" {
                                return false;
                            }
                        }
                    }
                }
            }
            // If we found a battery but couldn't confirm charging, assume not on battery unless status said so
            if found_battery {
                return false;
            }
        }
        // Fallback: unknown platform or missing sysfs — assume not on battery
        false
    }
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
            uniform: None,
            bind_group: None,
            dirty: true,
            opacity: 1.0,
            corner_radius_px: 0.0,
            tex_size: None,
            damage_regions: Vec::new(),
        };

        assert_eq!(window.id, 1);
        assert_eq!(window.position, (100.0, 100.0));
        assert_eq!(window.size, (400.0, 300.0));
        assert!(window.dirty);
    }
}
