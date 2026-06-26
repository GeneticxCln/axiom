// Axiom Compositor WGSL Shader
// Real GPU shader for compositing windows

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct Uniforms {
    projection: mat4x4<f32>,
}

struct WindowUniforms {
    opacity: f32,
    border_width: f32,
    window_width: f32,
    window_height: f32,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(0) @binding(1)
var window_texture: texture_2d<f32>;

@group(0) @binding(2)
var window_sampler: sampler;

@group(0) @binding(3)
var<uniform> window_uniforms: WindowUniforms;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = uniforms.projection * vec4<f32>(input.position, 1.0);
    output.tex_coords = input.tex_coords;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the window texture
    let color = textureSample(window_texture, window_sampler, input.tex_coords);
    
    // Compute border region (in normalized texture coordinates)
    let bw = window_uniforms.border_width;
    let ww = window_uniforms.window_width;
    let wh = window_uniforms.window_height;

    // Border color: semi-transparent dark border
    let border_color = vec4<f32>(0.15, 0.15, 0.15, 0.9);

    // Check if fragment is in border region
    let in_border = input.tex_coords.x < bw / ww
        || input.tex_coords.x > 1.0 - bw / ww
        || input.tex_coords.y < bw / wh
        || input.tex_coords.y > 1.0 - bw / wh;

    let opacity = window_uniforms.opacity;

    if (in_border) {
        return vec4<f32>(border_color.rgb, border_color.a * opacity);
    } else {
        // Apply gamma correction and opacity
        return vec4<f32>(pow(color.rgb, vec3<f32>(2.2)), color.a * opacity);
    }
}
