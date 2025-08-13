//! GPU-based Shadow Effects Implementation
//!
//! This module provides realistic shadow rendering for windows:
//! - Drop shadows with soft edges
//! - Inner shadows for depth
//! - Dynamic lighting effects
//! - Performance-optimized shadow maps

use wgpu::{
    Device, Queue, Buffer, Texture, TextureView, RenderPipeline, BindGroup,
    CommandEncoder, TextureFormat, TextureUsages, BufferUsages,
    BufferDescriptor, BindGroupDescriptor, BindGroupEntry,
    RenderPipelineDescriptor, FragmentState, VertexState, PrimitiveState,
    MultisampleState, ColorTargetState, BlendState, ColorWrites,
};
use cgmath::{Vector2, Vector3, Vector4, InnerSpace};
use log::{info, debug};
use anyhow::Result;
use std::sync::Arc;

use super::shaders::{ShaderManager, ShaderType};
use super::ShadowParams;

/// Different types of shadow effects
#[derive(Debug, Clone)]
pub enum ShadowType {
    /// Standard drop shadow
    DropShadow {
        offset: Vector2<f32>,
        blur_radius: f32,
        opacity: f32,
        color: Vector4<f32>,
    },
    /// Inner shadow for depth
    InnerShadow {
        offset: Vector2<f32>,
        blur_radius: f32,
        opacity: f32,
        color: Vector4<f32>,
    },
    /// Dynamic lighting shadow
    DynamicShadow {
        light_position: Vector3<f32>,
        blur_radius: f32,
        opacity: f32,
        color: Vector4<f32>,
    },
}

/// Shadow rendering quality levels
#[derive(Debug, Clone, Copy)]
pub enum ShadowQuality {
    Low,    // Simple shadow with minimal blur
    Medium, // Standard shadow with moderate blur
    High,   // High-quality shadow with extensive blur
    Ultra,  // Maximum quality with advanced features
}

/// GPU-based shadow renderer
pub struct ShadowRenderer {
    device: Arc<Device>,
    queue: Arc<Queue>,
    shader_manager: Arc<ShaderManager>,
    
    // Render pipelines for shadow effects
    drop_shadow_pipeline: Option<RenderPipeline>,
    inner_shadow_pipeline: Option<RenderPipeline>,
    
    // Uniform buffers
    shadow_params_buffer: Buffer,
    
    // Shadow map textures for complex shadows
    shadow_map_texture: Option<Texture>,
    shadow_map_view: Option<TextureView>,
    
    // Current shadow settings
    current_quality: ShadowQuality,
    global_shadow_params: ShadowParams,
    
    // Performance tracking
    last_render_time: std::time::Duration,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowUniforms {
    shadow_offset: [f32; 2],
    shadow_blur: f32,
    shadow_opacity: f32,
    shadow_color: [f32; 4],
    window_size: [f32; 2],
    light_position: [f32; 3],
    shadow_type: u32, // 0: drop, 1: inner, 2: dynamic
}

impl ShadowRenderer {
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        shader_manager: Arc<ShaderManager>,
        initial_params: ShadowParams,
        quality: ShadowQuality,
    ) -> Result<Self> {
        info!("🌟 Initializing GPU Shadow Renderer...");
        
        // Create uniform buffer for shadow parameters
        let shadow_params_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("Shadow Parameters Buffer"),
            size: std::mem::size_of::<ShadowUniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let mut renderer = Self {
            device,
            queue,
            shader_manager,
            drop_shadow_pipeline: None,
            inner_shadow_pipeline: None,
            shadow_params_buffer,
            shadow_map_texture: None,
            shadow_map_view: None,
            current_quality: quality,
            global_shadow_params: initial_params,
            last_render_time: std::time::Duration::from_millis(0),
        };
        
        // Initialize render pipelines
        renderer.create_shadow_pipelines()?;
        
        info!("✅ Shadow Renderer initialized with {:?} quality", quality);
        Ok(renderer)
    }
    
    /// Create render pipelines for shadow effects
    fn create_shadow_pipelines(&mut self) -> Result<()> {
        debug!("🔧 Creating shadow render pipelines...");
        
        // Get compiled shaders
        let shadow_shader = self.shader_manager.get_shader(&ShaderType::DropShadow)
            .ok_or_else(|| anyhow::anyhow!("Drop shadow shader not found"))?;
        
        // Create bind group layout for shadow uniforms
        let bind_group_layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shadow Bind Group Layout"),
            entries: &[
                // Shadow uniforms
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
            ],
        });
        
        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shadow Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        
        // Drop shadow pipeline
        self.drop_shadow_pipeline = Some(self.device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Drop Shadow Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: shadow_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: shadow_shader,
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
        }));
        
        debug!("✅ Shadow pipelines created successfully");
        Ok(())
    }
    
    /// Render drop shadow for a window
    pub fn render_drop_shadow(
        &mut self,
        encoder: &mut CommandEncoder,
        output_texture: &TextureView,
        window_position: Vector2<f32>,
        window_size: Vector2<f32>,
        shadow_params: &ShadowParams,
    ) -> Result<()> {
        if !shadow_params.enabled {
            return Ok(());
        }
        
        let start_time = std::time::Instant::now();
        
        // Calculate shadow parameters based on quality
        let (blur_radius, sample_count) = match self.current_quality {
            ShadowQuality::Low => (shadow_params.blur_radius * 0.5, 4),
            ShadowQuality::Medium => (shadow_params.blur_radius, 8),
            ShadowQuality::High => (shadow_params.blur_radius * 1.2, 16),
            ShadowQuality::Ultra => (shadow_params.blur_radius * 1.5, 32),
        };
        
        // Update uniform buffer with shadow parameters
        let uniforms = ShadowUniforms {
            shadow_offset: [shadow_params.offset.0, shadow_params.offset.1],
            shadow_blur: blur_radius,
            shadow_opacity: shadow_params.opacity,
            shadow_color: shadow_params.color,
            window_size: [window_size.x, window_size.y],
            light_position: [0.0, -200.0, 100.0], // Default light position
            shadow_type: 0, // Drop shadow
        };
        
        self.queue.write_buffer(&self.shadow_params_buffer, 0, bytemuck::cast_slice(&[uniforms]));
        
        // Create bind group for this render
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("Drop Shadow Bind Group"),
            layout: &self.drop_shadow_pipeline.as_ref().unwrap().get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.shadow_params_buffer.as_entire_binding(),
                },
            ],
        });
        
        // Render pass for shadow
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Drop Shadow Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_texture,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // Don't clear - we're adding to existing scene
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        
        render_pass.set_pipeline(self.drop_shadow_pipeline.as_ref().unwrap());
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..6, 0..1); // Two triangles for quad
        
        self.last_render_time = start_time.elapsed();
        
        debug!("🌟 Rendered drop shadow: blur={:.1}, opacity={:.1}, time={:.2}ms",
               blur_radius, shadow_params.opacity, self.last_render_time.as_secs_f64() * 1000.0);
        
        Ok(())
    }
    
    /// Render multiple shadows efficiently in a batch
    pub fn render_shadow_batch(
        &mut self,
        encoder: &mut CommandEncoder,
        output_texture: &TextureView,
        shadow_data: &[(Vector2<f32>, Vector2<f32>, ShadowParams)], // (position, size, params)
    ) -> Result<()> {
        let start_time = std::time::Instant::now();
        
        // Process each shadow individually to avoid bind group lifetime issues
        for (_position, size, shadow_params) in shadow_data {
            if !shadow_params.enabled {
                continue;
            }
            
            // Update uniforms for this shadow
            let blur_radius = match self.current_quality {
                ShadowQuality::Low => shadow_params.blur_radius * 0.5,
                ShadowQuality::Medium => shadow_params.blur_radius,
                ShadowQuality::High => shadow_params.blur_radius * 1.2,
                ShadowQuality::Ultra => shadow_params.blur_radius * 1.5,
            };
            
            let uniforms = ShadowUniforms {
                shadow_offset: [shadow_params.offset.0, shadow_params.offset.1],
                shadow_blur: blur_radius,
                shadow_opacity: shadow_params.opacity,
                shadow_color: shadow_params.color,
                window_size: [size.x, size.y],
                light_position: [0.0, -200.0, 100.0],
                shadow_type: 0,
            };
            
            self.queue.write_buffer(&self.shadow_params_buffer, 0, bytemuck::cast_slice(&[uniforms]));
            
            // Create bind group for this shadow
            let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("Batch Shadow Bind Group"),
                layout: &self.drop_shadow_pipeline.as_ref().unwrap().get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self.shadow_params_buffer.as_entire_binding(),
                    },
                ],
            });
            
            // Individual render pass for each shadow
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Shadow Batch Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: output_texture,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                
                render_pass.set_pipeline(self.drop_shadow_pipeline.as_ref().unwrap());
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }
        
        self.last_render_time = start_time.elapsed();
        
        debug!("🌟 Rendered {} shadows in batch, time={:.2}ms",
               shadow_data.len(), self.last_render_time.as_secs_f64() * 1000.0);
        
        Ok(())
    }
    
    /// Render dynamic shadow based on light position
    pub fn render_dynamic_shadow(
        &mut self,
        encoder: &mut CommandEncoder,
        output_texture: &TextureView,
        window_position: Vector2<f32>,
        window_size: Vector2<f32>,
        light_position: Vector3<f32>,
        shadow_params: &ShadowParams,
    ) -> Result<()> {
        if !shadow_params.enabled {
            return Ok(());
        }
        
        // Calculate shadow offset based on light position
        let light_direction = Vector2::new(
            light_position.x - window_position.x,
            light_position.y - window_position.y,
        );
        
        // Normalize and scale for shadow offset
        let shadow_distance = 20.0 * (100.0 / light_position.z.max(50.0));
        let shadow_offset = if light_direction.magnitude() > 0.0 {
            let normalized = light_direction / light_direction.magnitude();
            Vector2::new(-normalized.x * shadow_distance, -normalized.y * shadow_distance)
        } else {
            Vector2::new(0.0, shadow_distance) // Default downward shadow
        };
        
        // Calculate blur based on distance from light
        let distance_factor = (light_position.z / 200.0).min(2.0);
        let dynamic_blur = shadow_params.blur_radius * distance_factor;
        
        // Update uniforms for dynamic shadow
        let uniforms = ShadowUniforms {
            shadow_offset: [shadow_offset.x, shadow_offset.y],
            shadow_blur: dynamic_blur,
            shadow_opacity: shadow_params.opacity * (1.0 / distance_factor.max(0.5)),
            shadow_color: shadow_params.color,
            window_size: [window_size.x, window_size.y],
            light_position: [light_position.x, light_position.y, light_position.z],
            shadow_type: 2, // Dynamic shadow
        };
        
        self.queue.write_buffer(&self.shadow_params_buffer, 0, bytemuck::cast_slice(&[uniforms]));
        
        // Create bind group
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("Dynamic Shadow Bind Group"),
            layout: &self.drop_shadow_pipeline.as_ref().unwrap().get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: self.shadow_params_buffer.as_entire_binding(),
                },
            ],
        });
        
        // Render pass
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Dynamic Shadow Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_texture,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        
        render_pass.set_pipeline(self.drop_shadow_pipeline.as_ref().unwrap());
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..6, 0..1);
        
        debug!("🌟 Rendered dynamic shadow: offset=({:.1}, {:.1}), blur={:.1}",
               shadow_offset.x, shadow_offset.y, dynamic_blur);
        
        Ok(())
    }
    
    /// Update shadow quality level
    pub fn set_shadow_quality(&mut self, quality: ShadowQuality) {
        if std::mem::discriminant(&self.current_quality) != std::mem::discriminant(&quality) {
            info!("🎛️ Updated shadow quality: {:?} -> {:?}", self.current_quality, quality);
            self.current_quality = quality;
        }
    }
    
    /// Update global shadow parameters
    pub fn update_global_shadow_params(&mut self, params: ShadowParams) {
        self.global_shadow_params = params;
        debug!("🔄 Updated global shadow parameters");
    }
    
    /// Get performance statistics
    pub fn get_performance_stats(&self) -> (std::time::Duration, ShadowQuality) {
        (self.last_render_time, self.current_quality)
    }
    
    /// Create a shadow map texture for advanced shadow techniques
    fn ensure_shadow_map(&mut self, size: Vector2<u32>) -> Result<()> {
        let needs_creation = self.shadow_map_texture.as_ref()
            .map(|texture| texture.width() != size.x || texture.height() != size.y)
            .unwrap_or(true);
        
        if needs_creation && matches!(self.current_quality, ShadowQuality::Ultra) {
            debug!("🗺️ Creating shadow map texture: {}x{}", size.x, size.y);
            
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Shadow Map Texture"),
                size: wgpu::Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            
            self.shadow_map_texture = Some(texture);
            self.shadow_map_view = Some(view);
        }
        
        Ok(())
    }
    
    /// Optimize shadow parameters based on performance
    pub fn optimize_for_performance(&mut self, frame_time: std::time::Duration, target_time: std::time::Duration) {
        if frame_time > target_time * 2 {
            // Performance is poor, reduce quality
            match self.current_quality {
                ShadowQuality::Ultra => self.set_shadow_quality(ShadowQuality::High),
                ShadowQuality::High => self.set_shadow_quality(ShadowQuality::Medium),
                ShadowQuality::Medium => self.set_shadow_quality(ShadowQuality::Low),
                ShadowQuality::Low => {
                    // Already at lowest quality, reduce shadow opacity
                    if self.global_shadow_params.opacity > 0.3 {
                        self.global_shadow_params.opacity *= 0.9;
                        debug!("⚡ Reduced shadow opacity to {:.2} for performance", 
                               self.global_shadow_params.opacity);
                    }
                }
            }
        } else if frame_time < target_time / 2 {
            // Performance is good, can increase quality
            match self.current_quality {
                ShadowQuality::Low => self.set_shadow_quality(ShadowQuality::Medium),
                ShadowQuality::Medium => self.set_shadow_quality(ShadowQuality::High),
                ShadowQuality::High => self.set_shadow_quality(ShadowQuality::Ultra),
                ShadowQuality::Ultra => {
                    // Already at highest quality, restore full opacity if needed
                    if self.global_shadow_params.opacity < 1.0 {
                        self.global_shadow_params.opacity = (self.global_shadow_params.opacity * 1.1).min(1.0);
                    }
                }
            }
        }
    }
}
