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

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(0) @binding(1)
var window_texture: texture_2d<f32>;

@group(0) @binding(2)
var window_sampler: sampler;

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
    
    // Apply gamma correction and return
    return vec4<f32>(pow(color.rgb, vec3<f32>(2.2)), color.a);
}
