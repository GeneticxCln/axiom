//! GPU Shader Definitions for Visual Effects
//!
//! This module contains all WGSL shaders used by the effects engine:
//! - Blur effects (Gaussian, dual-pass, bokeh)
//! - Shadow rendering (drop shadows, inner shadows)
//! - Rounded corners with anti-aliasing
//! - Window animations and transformations

use anyhow::Result;
use log::{debug, info};
use wgpu::{ShaderModule, ShaderModuleDescriptor, ShaderSource};

/// Shader types supported by the effects engine
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShaderType {
    /// Basic vertex shader for window rendering
    WindowVertex,
    /// Fragment shader for basic window rendering
    WindowFragment,
    /// Gaussian blur (horizontal pass)
    BlurHorizontal,
    /// Gaussian blur (vertical pass)
    BlurVertical,
    /// Drop shadow rendering
    DropShadow,
    /// Rounded corners with anti-aliasing
    RoundedCorners,
    /// Animation transformations
    AnimationTransform,
}

/// Shader manager for compiling and caching GPU shaders
pub struct ShaderManager {
    device: wgpu::Device,
    compiled_shaders: std::collections::HashMap<ShaderType, ShaderModule>,
}

impl ShaderManager {
    pub fn new(device: wgpu::Device) -> Self {
        info!("🎨 Initializing GPU Shader Manager...");
        Self {
            device,
            compiled_shaders: std::collections::HashMap::new(),
        }
    }

    /// Compile all effects shaders
    pub fn compile_all_shaders(&mut self) -> Result<()> {
        info!("⚡ Compiling Phase 4 visual effects shaders...");

        // Compile window rendering shaders
        self.compile_shader(ShaderType::WindowVertex)?;
        self.compile_shader(ShaderType::WindowFragment)?;

        // Compile blur effect shaders
        self.compile_shader(ShaderType::BlurHorizontal)?;
        self.compile_shader(ShaderType::BlurVertical)?;

        // Compile shadow and corner shaders
        self.compile_shader(ShaderType::DropShadow)?;
        self.compile_shader(ShaderType::RoundedCorners)?;

        // Compile animation shader
        self.compile_shader(ShaderType::AnimationTransform)?;

        info!(
            "✅ Successfully compiled {} shaders",
            self.compiled_shaders.len()
        );
        Ok(())
    }

    /// Get a compiled shader by type
    pub fn get_shader(&self, shader_type: &ShaderType) -> Option<&ShaderModule> {
        self.compiled_shaders.get(shader_type)
    }

    /// Compile a specific shader
    fn compile_shader(&mut self, shader_type: ShaderType) -> Result<()> {
        let source = self.get_shader_source(&shader_type);

        let shader = self.device.create_shader_module(ShaderModuleDescriptor {
            label: Some(&format!("{:?} Shader", shader_type)),
            source: ShaderSource::Wgsl(source.into()),
        });

        self.compiled_shaders.insert(shader_type.clone(), shader);
        debug!("✨ Compiled shader: {:?}", shader_type);

        Ok(())
    }

    /// Get shader source code for a specific shader type
    fn get_shader_source(&self, shader_type: &ShaderType) -> &'static str {
        match shader_type {
            ShaderType::WindowVertex => WINDOW_VERTEX_SHADER,
            ShaderType::WindowFragment => WINDOW_FRAGMENT_SHADER,
            ShaderType::BlurHorizontal => BLUR_HORIZONTAL_SHADER,
            ShaderType::BlurVertical => BLUR_VERTICAL_SHADER,
            ShaderType::DropShadow => DROP_SHADOW_SHADER,
            ShaderType::RoundedCorners => ROUNDED_CORNERS_SHADER,
            ShaderType::AnimationTransform => ANIMATION_TRANSFORM_SHADER,
        }
    }
}

/// Basic window vertex shader - handles positioning and transformations
const WINDOW_VERTEX_SHADER: &str = r#"
// Vertex shader for window rendering with animations
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec2<f32>,
}

struct WindowUniforms {
    transform: mat4x4<f32>,
    scale: f32,
    opacity: f32,
    corner_radius: f32,
    window_size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> window: WindowUniforms;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Apply scale animation
    let scaled_pos = input.position * window.scale;
    
    // Apply transformation matrix (includes position offset, rotation)
    let world_pos = window.transform * vec4<f32>(scaled_pos, 0.0, 1.0);
    
    out.clip_position = world_pos;
    out.tex_coords = input.tex_coords;
    out.world_position = scaled_pos;
    
    return out;
}
"#;

/// Window fragment shader with rounded corners and opacity
const WINDOW_FRAGMENT_SHADER: &str = r#"
// Fragment shader for window rendering with rounded corners
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec2<f32>,
}

struct WindowUniforms {
    transform: mat4x4<f32>,
    scale: f32,
    opacity: f32,
    corner_radius: f32,
    window_size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> window: WindowUniforms;

@group(0) @binding(1)
var window_texture: texture_2d<f32>;

@group(0) @binding(2)
var window_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the window texture
    let base_color = textureSample(window_texture, window_sampler, input.tex_coords);
    
    // Calculate distance from corners for rounded corner effect
    let pos = input.world_position;
    let half_size = window.window_size * 0.5;
    
    // Distance to nearest corner
    let corner_pos = abs(pos) - (half_size - window.corner_radius);
    let corner_dist = length(max(corner_pos, vec2<f32>(0.0))) - window.corner_radius;
    
    // Smooth anti-aliased alpha for rounded corners
    let alpha_multiplier = 1.0 - smoothstep(-1.0, 1.0, corner_dist);
    
    // Apply opacity animation
    let final_alpha = base_color.a * window.opacity * alpha_multiplier;
    
    return vec4<f32>(base_color.rgb, final_alpha);
}
"#;

/// Horizontal blur pass shader
const BLUR_HORIZONTAL_SHADER: &str = r#"
// Horizontal Gaussian blur pass
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct BlurUniforms {
    radius: f32,
    intensity: f32,
    direction: vec2<f32>,
    texture_size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> blur: BlurUniforms;

@group(0) @binding(1)
var input_texture: texture_2d<f32>;

@group(0) @binding(2)
var input_sampler: sampler;

// Gaussian weights for 9-tap blur
let gaussian_weights = array<f32, 9>(
    0.0947416, 0.118318, 0.0947416,
    0.118318, 0.147761, 0.118318,  
    0.0947416, 0.118318, 0.0947416
);

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let tex_offset = 1.0 / blur.texture_size;
    let step = tex_offset * blur.radius * blur.direction;
    
    var result = vec4<f32>(0.0);
    
    // 9-tap Gaussian blur
    for (var i: i32 = -4; i <= 4; i++) {
        let offset = step * f32(i);
        let sample_coords = input.tex_coords + offset;
        let sample_color = textureSample(input_texture, input_sampler, sample_coords);
        
        result += sample_color * gaussian_weights[i + 4];
    }
    
    return mix(textureSample(input_texture, input_sampler, input.tex_coords), result, blur.intensity);
}
"#;

/// Vertical blur pass shader (similar to horizontal but different direction)
const BLUR_VERTICAL_SHADER: &str = r#"
// Vertical Gaussian blur pass
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct BlurUniforms {
    radius: f32,
    intensity: f32,
    direction: vec2<f32>,
    texture_size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> blur: BlurUniforms;

@group(0) @binding(1)
var input_texture: texture_2d<f32>;

@group(0) @binding(2)
var input_sampler: sampler;

// Gaussian weights for 9-tap blur
let gaussian_weights = array<f32, 9>(
    0.0947416, 0.118318, 0.0947416,
    0.118318, 0.147761, 0.118318,  
    0.0947416, 0.118318, 0.0947416
);

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let tex_offset = 1.0 / blur.texture_size;
    let step = tex_offset * blur.radius * blur.direction;
    
    var result = vec4<f32>(0.0);
    
    // 9-tap Gaussian blur
    for (var i: i32 = -4; i <= 4; i++) {
        let offset = step * f32(i);
        let sample_coords = input.tex_coords + offset;
        let sample_color = textureSample(input_texture, input_sampler, sample_coords);
        
        result += sample_color * gaussian_weights[i + 4];
    }
    
    return mix(textureSample(input_texture, input_sampler, input.tex_coords), result, blur.intensity);
}
"#;

/// Drop shadow shader
const DROP_SHADOW_SHADER: &str = r#"
// Drop shadow rendering shader
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec2<f32>,
}

struct ShadowUniforms {
    shadow_offset: vec2<f32>,
    shadow_blur: f32,
    shadow_opacity: f32,
    shadow_color: vec4<f32>,
    window_size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> shadow: ShadowUniforms;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Calculate distance from window edges for shadow falloff
    let pos = input.world_position + shadow.shadow_offset;
    let half_size = shadow.window_size * 0.5;
    
    // Distance field for shadow shape
    let edge_dist = length(max(abs(pos) - half_size, vec2<f32>(0.0)));
    
    // Smooth shadow falloff
    let shadow_alpha = 1.0 - smoothstep(0.0, shadow.shadow_blur, edge_dist);
    
    // Final shadow color with opacity
    let final_alpha = shadow_alpha * shadow.shadow_opacity;
    
    return vec4<f32>(shadow.shadow_color.rgb, final_alpha);
}
"#;

/// Rounded corners shader
const ROUNDED_CORNERS_SHADER: &str = r#"
// Anti-aliased rounded corners shader
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec2<f32>,
}

struct CornerUniforms {
    corner_radius: f32,
    window_size: vec2<f32>,
    border_width: f32,
    border_color: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> corner: CornerUniforms;

@group(0) @binding(1)
var window_texture: texture_2d<f32>;

@group(0) @binding(2)
var window_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let base_color = textureSample(window_texture, window_sampler, input.tex_coords);
    
    // Calculate rounded corner mask
    let pos = input.world_position;
    let half_size = corner.window_size * 0.5;
    let corner_pos = abs(pos) - (half_size - corner.corner_radius);
    let corner_dist = length(max(corner_pos, vec2<f32>(0.0))) - corner.corner_radius;
    
    // Anti-aliased alpha
    let alpha = 1.0 - smoothstep(-1.0, 1.0, corner_dist);
    
    // Optional border effect
    let border_alpha = 1.0 - smoothstep(-corner.border_width - 1.0, -corner.border_width + 1.0, corner_dist);
    let border_mask = border_alpha - alpha;
    
    // Mix base color with border color
    let final_color = mix(base_color.rgb, corner.border_color.rgb, border_mask);
    let final_alpha = max(alpha * base_color.a, border_mask * corner.border_color.a);
    
    return vec4<f32>(final_color, final_alpha);
}
"#;

/// Animation transformation vertex shader
const ANIMATION_TRANSFORM_SHADER: &str = r#"
// Advanced animation transformation shader
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_position: vec2<f32>,
}

struct AnimationUniforms {
    transform: mat4x4<f32>,
    scale: vec2<f32>,
    rotation: f32,
    opacity: f32,
    time: f32,
    animation_type: u32, // 0: none, 1: bounce, 2: elastic, 3: spring
}

@group(0) @binding(0)
var<uniform> anim: AnimationUniforms;

// Animation helper functions
fn bounce_ease_out(t: f32) -> f32 {
    if t < 1.0 / 2.75 {
        return 7.5625 * t * t;
    } else if t < 2.0 / 2.75 {
        let t2 = t - 1.5 / 2.75;
        return 7.5625 * t2 * t2 + 0.75;
    } else if t < 2.5 / 2.75 {
        let t2 = t - 2.25 / 2.75;
        return 7.5625 * t2 * t2 + 0.9375;
    } else {
        let t2 = t - 2.625 / 2.75;
        return 7.5625 * t2 * t2 + 0.984375;
    }
}

fn elastic_ease_out(t: f32) -> f32 {
    if t == 0.0 { return 0.0; }
    if t == 1.0 { return 1.0; }
    
    let p = 0.3;
    let s = p / 4.0;
    return pow(2.0, -10.0 * t) * sin((t - s) * (2.0 * 3.14159265) / p) + 1.0;
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Apply scale with animation easing
    var final_scale = anim.scale;
    
    if anim.animation_type == 1u { // Bounce
        let bounce_factor = bounce_ease_out(anim.time);
        final_scale = anim.scale * bounce_factor;
    } else if anim.animation_type == 2u { // Elastic
        let elastic_factor = elastic_ease_out(anim.time);
        final_scale = anim.scale * elastic_factor;
    }
    
    // Apply rotation
    let cos_r = cos(anim.rotation);
    let sin_r = sin(anim.rotation);
    let rotation_matrix = mat2x2<f32>(cos_r, -sin_r, sin_r, cos_r);
    
    // Transform vertex
    let scaled_pos = input.position * final_scale;
    let rotated_pos = rotation_matrix * scaled_pos;
    let world_pos = anim.transform * vec4<f32>(rotated_pos, 0.0, 1.0);
    
    out.clip_position = world_pos;
    out.tex_coords = input.tex_coords;
    out.world_position = rotated_pos;
    
    return out;
}
"#;
