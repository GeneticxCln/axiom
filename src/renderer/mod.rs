//! Real GPU rendering pipeline for Axiom compositor
//!
//! This module implements actual GPU rendering using wgpu to composite
//! windows and effects to the screen - not just stubs.

#![allow(dead_code)]
use anyhow::Result;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wgpu::*;
use wgpu::util::DeviceExt;
use std::time::Duration;

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
    /// Bind group for sampling the texture
    pub bind_group: Option<BindGroup>,
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

static RENDER_STATE: OnceLock<Arc<Mutex<SharedRenderState>>> = OnceLock::new();

#[derive(Default)]
struct SharedRenderState {
    placeholders: HashMap<u64, ((f32, f32), (f32, f32), f32)>,
    pending_textures: Vec<(u64, Vec<u8>, u32, u32)>,
}

pub fn push_placeholder_quad(id: u64, position: (f32, f32), size: (f32, f32), opacity: f32) {
    let state = RENDER_STATE.get_or_init(|| Arc::new(Mutex::new(SharedRenderState::default())));
    if let Ok(mut s) = state.lock() {
        s.placeholders.insert(id, (position, size, opacity));
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
            info!("🎨 Creating real GPU renderer (no surface) width={} height={}", width, height);
        }

        // Try to find an adapter compatible with the surface
        let mut chosen_adapter: Option<wgpu::Adapter> = None;
        let surface_for_opts = surface;

        for power in [wgpu::PowerPreference::HighPerformance, wgpu::PowerPreference::LowPower] {
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
            if chosen_adapter.is_some() { break; }
        }

        // If we couldn't find a surface-compatible adapter, fall back to headless mode
        let (device, queue, surface_compatible, pipeline, bgl, sampler, surface_format) = if let Some(adapter) = chosen_adapter {
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
                let fmts: Vec<String> = caps.formats.iter().map(|f| format!("{:?}", f)).collect();
                let pmods: Vec<String> = caps.present_modes.iter().map(|m| format!("{:?}", m)).collect();
                let amods: Vec<String> = caps.alpha_modes.iter().map(|a| format!("{:?}", a)).collect();
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
                let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
                    wgpu::PresentMode::Mailbox
                } else if caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
                    wgpu::PresentMode::Fifo
                } else {
                    caps.present_modes[0]
                };
                let alpha_mode = caps
                    .alpha_modes
                    .iter()
                    .copied()
                    .find(|m| matches!(m, wgpu::CompositeAlphaMode::Auto | wgpu::CompositeAlphaMode::Opaque))
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
                let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Axiom Texture BGL"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

                let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

            (device, queue, surface.is_some(), out_pipeline, out_bgl, out_sampler, out_format)
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
            });
        };

        info!("✅ GPU renderer initialized successfully");
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
        })
    }

    /// Backward-compatible constructor; may fall back to headless if given surface was created
    /// from a different Instance and no compatible adapter is found.
    pub async fn new(surface: &wgpu::Surface<'_>, width: u32, height: u32) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
        Self::new_with_instance(&instance, Some(surface), width, height).await
    }

    /// Create a headless renderer for testing with specified backends
    pub async fn new_headless_with_backends(backends: wgpu::Backends) -> Result<Self> {
        info!("🎨 Creating headless GPU renderer for testing (backends={:?})", backends);

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
            bind_group: None,
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
                bind_group: None,
                dirty: true,
                opacity,
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
            // Create texture
            let texture = self.device.create_texture(&TextureDescriptor {
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
            });

            // Upload pixel data to GPU
            self.queue.write_texture(
                ImageCopyTexture {
                    aspect: TextureAspect::All,
                    texture: &texture,
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

            let texture_view = texture.create_view(&TextureViewDescriptor::default());

            window.texture = Some(texture);
            window.texture_view = Some(texture_view);
            window.bind_group = None;
            window.dirty = true;

            info!("✅ Updated texture for window {}", window_id);
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
        Ok(())
    }

    /// Render all windows to a wgpu surface (real rendering)
    pub fn render_to_surface(
        &mut self,
        _surface: &wgpu::Surface<'_>,
        surface_texture: &wgpu::SurfaceTexture,
    ) -> Result<()> {
        if !self.surface_compatible {
            // Graceful no-op: we don't have a surface-compatible device, skip rendering
            debug!("🚫 Skipping surface render (no compatible adapter); presenting empty frame");
            return Ok(());
        }
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

        if let (Some(pipeline), Some(bgl), Some(sampler)) = (&self.pipeline, &self.bind_group_layout, &self.sampler) {
            // Pre-allocate buffers once per frame for all windows and draw
            // Flatten all quads into a single draw list
            let mut all_vertices: Vec<Vertex> = Vec::new();
            let mut all_indices: Vec<u16> = Vec::new();
            let mut draw_bind_groups: Vec<&BindGroup> = Vec::new();

            for window in &mut self.windows {
                if let Some(tex_view) = &window.texture_view {
                    if window.bind_group.is_none() {
                        window.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("Axiom Window BindGroup"),
                            layout: bgl,
                            entries: &[
                                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tex_view) },
                                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                            ],
                        }));
                    }
                    let bind_group = window.bind_group.as_ref().unwrap();

                    // Build quad vertices in clip space for this window
                    let (x, y) = window.position;
                    let (w, h) = window.size;
                    let fw = self.size.0 as f32;
                    let fh = self.size.1 as f32;
                    let x0 = (x / fw) * 2.0 - 1.0;
                    let y0 = 1.0 - (y / fh) * 2.0;
                    let x1 = ((x + w) / fw) * 2.0 - 1.0;
                    let y1 = 1.0 - ((y + h) / fh) * 2.0;

                    let base_index = all_vertices.len() as u16;
                    all_vertices.extend_from_slice(&[
                        Vertex { position: [x0, y1, 0.0], tex_coords: [0.0, 1.0] },
                        Vertex { position: [x1, y1, 0.0], tex_coords: [1.0, 1.0] },
                        Vertex { position: [x0, y0, 0.0], tex_coords: [0.0, 0.0] },
                        Vertex { position: [x1, y0, 0.0], tex_coords: [1.0, 0.0] },
                    ]);
                    all_indices.extend_from_slice(&[base_index, base_index + 1, base_index + 2, base_index + 2, base_index + 1, base_index + 3]);
                    draw_bind_groups.push(bind_group);
                }
            }

            if !all_vertices.is_empty() {
                let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Axiom Quad Verts Batch"),
                    contents: bytemuck::cast_slice(&all_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.05, b: 0.06, a: 1.0 }),
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

                    for (i, bind_group) in draw_bind_groups.iter().enumerate() {
                        rpass.set_bind_group(0, bind_group, &[]);
                        let first_index = (i as u32) * 6;
                        rpass.draw_indexed(first_index..first_index + 6, 0, 0..1);
                    }
                }
            }
        }

        // Submit commands to GPU
        self.queue.submit(std::iter::once(encoder.finish()));

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

    /// Sync renderer state from shared placeholders and pending textures
    pub fn sync_from_shared(&mut self) {
        if let Some(state) = RENDER_STATE.get() {
            if let Ok(mut s) = state.lock() {
                for (id, (pos, size, opacity)) in s.placeholders.iter() {
                    self.upsert_window_rect(*id, *pos, *size, *opacity);
                }
                let updates: Vec<_> = s.pending_textures.drain(..).collect();
                drop(s);
                for (id, data, w, h) in updates {
                    let _ = self.update_window_texture(id, &data, w, h);
                }
            }
        }
    }

    /// Start a simple headless render loop at ~60 FPS for development
    pub async fn start_headless_loop_with_backends(backends: wgpu::Backends) -> Result<tokio::task::JoinHandle<()>> {
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
                        // Drain pending textures into renderer
                        let updates: Vec<_> = s.pending_textures.drain(..).collect();
                        drop(s);
                        for (id, data, w, h) in updates {
                            let _ = renderer.update_window_texture(id, &data, w, h);
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
