// Axiom Compositor Placeholder Shader
//
// Solid-colored quads for windows that have not yet received an
// SHM commit (no real client texture). Used exclusively by
// `AxiomRenderer::compose_full_frame` to eliminate the
// GL scissor-fallback in the legacy `src/backend/mod.rs` renderer.
//
// Bindings intentionally mirror the textured shader so the same
// `cached_projection_buffer` (binding 0) and per-window
// `cached_uniform_buffer` (binding 1) can be re-used without
// re-uploading. No texture, no sampler — this pipeline draws only
// from uniform data, so the existing `WindowUniforms` struct
// (opacity, border_width, width, height) is enough.

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct ProjectionUniforms {
    projection: mat4x4<f32>,
}

struct WindowUniforms {
    opacity: f32,
    border_width: f32,
    window_width: f32,
    window_height: f32,
    border_color: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: ProjectionUniforms;

@group(0) @binding(1)
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
    let bw = window_uniforms.border_width;
    let ww = window_uniforms.window_width;
    let wh = window_uniforms.window_height;
    let in_border = input.tex_coords.x < bw / ww
        || input.tex_coords.x > 1.0 - bw / ww
        || input.tex_coords.y < bw / wh
        || input.tex_coords.y > 1.0 - bw / wh;

    let border_color = window_uniforms.border_color;
    let body_color = vec4<f32>(0.15, 0.15, 0.18, 1.0);
    let base = select(body_color, border_color, in_border);
    return vec4<f32>(base.rgb, base.a * window_uniforms.opacity);
}
