// Textured quad shader with per-window uniforms and rounded-corner mask
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct WindowUniforms {
    // params = vec4(opacity, corner_radius_px, window_width_px, window_height_px)
    params: vec4<f32>,
    // params2 = vec4(mode, shadow_spread_px, shadow_offset_x, shadow_offset_y)
    // mode: 0.0 = window, 1.0 = shadow
    params2: vec4<f32>,
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
    let opacity = window.params.x;
    let radius = window.params.y;
    let win_size = vec2<f32>(window.params.z, window.params.w);
    let mode = window.params2.x;
    let shadow_spread = window.params2.y;
    let tint = vec3<f32>(window.params2.y, window.params2.z, window.params2.w);

    // Compute signed distance from rounded-rectangle (SDF) in pixel space
    // Transform uv (0..1) to local pixel coordinates centered at 0
    let p = in.uv * win_size - (win_size * 0.5);
    let q = abs(p) - (win_size * 0.5 - vec2<f32>(radius, radius));
    let max_q = max(q, vec2<f32>(0.0, 0.0));
    let dist = length(max_q) - radius;

    if (mode > 2.5) {
        // Solid fill with rounded-rect mask using window.params (opacity, radius, w, h)
        let edge0 = 0.5;
        let edge1 = 1.5;
        let mask = clamp(1.0 - smoothstep(edge0, edge1, dist), 0.0, 1.0);
        return vec4<f32>(tint, opacity * mask);
    } else if (mode > 1.5) {
        // Solid fill mode: draw a tinted rectangle without mask (full rect)
        return vec4<f32>(tint, opacity);
    } else if (mode > 0.5) {
        // Shadow mode: draw outside the rounded-rect with soft falloff over shadow_spread px
        // Only outside region contributes (dist >= 0)
        let outside = step(0.0, dist);
        let falloff = 1.0 - smoothstep(0.0, max(1.0, shadow_spread), dist);
        let alpha = opacity * outside * falloff;
        let color = vec3<f32>(0.0, 0.0, 0.0);
        return vec4<f32>(color, alpha);
    } else {
        // Window mode: sample texture and apply rounded-corner mask (~1px edge AA)
        let base = textureSample(t0, s0, in.uv);
        let edge0 = 0.5;
        let edge1 = 1.5;
        let mask = clamp(1.0 - smoothstep(edge0, edge1, dist), 0.0, 1.0);
        return vec4<f32>(base.rgb, base.a * opacity * mask);
    }
}
