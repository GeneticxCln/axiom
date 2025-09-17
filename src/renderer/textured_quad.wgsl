// Minimal textured quad shader with per-window uniforms
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct WindowUniforms {
    // params = vec4(opacity, corner_radius_px, window_width, window_height)
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> window: WindowUniforms;
@group(0) @binding(1)
var t0: texture_2d<f32>;
@group(0) @binding(2)
var s0: sampler;

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(input.pos, 1.0);
    out.uv = input.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSample(t0, s0, in.uv);
    let opacity = window.params.x;
    // Corner radius is available as window.params.y (px), width=window.params.z, height=window.params.w (px)
    return vec4<f32>(base.rgb, base.a * opacity);
}
