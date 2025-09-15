// Minimal textured quad shader
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(input.pos, 1.0);
    out.uv = input.uv;
    return out;
}

@group(0) @binding(0)
var t0: texture_2d<f32>;
@group(0) @binding(1)
var s0: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let color = textureSample(t0, s0, in.uv);
    return color;
}
