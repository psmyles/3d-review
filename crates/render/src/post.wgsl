// Post / composite pass: samples the resolved linear-HDR offscreen scene targets,
// applies ambient-only SSAO, adds bloom, tone-maps, encodes to sRGB, and writes
// the final LDR color into egui's framebuffer behind the chrome. FXAA operates on
// the composed display-space result.

@group(0) @binding(0)
var scene_color: texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler: sampler;

struct PostUniforms {
    inv_resolution: vec2<f32>,
    fxaa_enabled: u32,
    bloom_enabled: u32,
    bloom_intensity: f32,
    ssao_enabled: u32,
    _pad0: f32,
    _pad1: f32,
};
@group(0) @binding(2)
var<uniform> post: PostUniforms;
// Blurred linear-HDR bloom (half-res; the filtering sampler upsamples it). Added
// back over the scene when `bloom_enabled` is set.
@group(0) @binding(3)
var bloom_texture: texture_2d<f32>;
// Blurred SSAO occlusion (R8, full-res). Multiplied into the scene's ambient
// light when `ssao_enabled` is set.
@group(0) @binding(4)
var ssao_texture: texture_2d<f32>;
// Linear HDR ambient radiance eligible for SSAO attenuation.
@group(0) @binding(5)
var ambient_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Single oversized triangle covering the whole framebuffer (the standard
    // fullscreen-triangle trick — no vertex buffer needed).
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];
    var out: VertexOutput;
    out.clip_position = vec4<f32>(corner, 0.0, 1.0);
    // Map clip space to texture UV, flipping Y (texture origin is top-left).
    var uv = corner * 0.5 + vec2<f32>(0.5, 0.5);
    uv.y = 1.0 - uv.y;
    out.uv = uv;
    return out;
}

// Perceptual luma weights (Rec. 601), the standard FXAA edge metric.
const LUMA: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Khronos PBR Neutral tone mapping (Rec.709 linear), ported from
// https://github.com/KhronosGroup/ToneMapping/tree/main/PBR_Neutral
// Rolls off highlights >1.0 gracefully instead of hard per-channel clipping,
// preserving hue with only controlled desaturation near white.
fn pbr_neutral_tonemap(color_in: vec3<f32>) -> vec3<f32> {
    let start_compression = 0.8 - 0.04;
    let desaturation = 0.15;
    var color = color_in;
    let x = min(color.r, min(color.g, color.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    color = color - offset;
    let peak = max(color.r, max(color.g, color.b));
    if (peak < start_compression) { return color; }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    color = color * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, new_peak * vec3<f32>(1.0, 1.0, 1.0), g);
}

fn compose_ldr(uv: vec2<f32>) -> vec3<f32> {
    var lit = textureSample(scene_color, scene_sampler, uv).rgb;
    if (post.ssao_enabled != 0u) {
        let ao = textureSample(ssao_texture, scene_sampler, uv).r;
        let ambient = textureSample(ambient_texture, scene_sampler, uv).rgb;
        lit = max(lit - ambient * (1.0 - ao), vec3<f32>(0.0));
    }
    if (post.bloom_enabled != 0u) {
        let bloom = textureSample(bloom_texture, scene_sampler, uv).rgb;
        lit = lit + bloom * post.bloom_intensity;
    }
    return linear_to_srgb(pbr_neutral_tonemap(lit));
}

// Classic NVIDIA FXAA II ("console" quality) edge-blend. Cheap, dependency-free,
// and a good match for a single full-screen post pass over LDR color.
fn fxaa(uv: vec2<f32>) -> vec3<f32> {
    let span_max = 8.0;
    let reduce_mul = 1.0 / 8.0;
    let reduce_min = 1.0 / 128.0;
    let inv = post.inv_resolution;

    let rgb_nw = compose_ldr(uv + vec2<f32>(-1.0, -1.0) * inv);
    let rgb_ne = compose_ldr(uv + vec2<f32>(1.0, -1.0) * inv);
    let rgb_sw = compose_ldr(uv + vec2<f32>(-1.0, 1.0) * inv);
    let rgb_se = compose_ldr(uv + vec2<f32>(1.0, 1.0) * inv);
    let rgb_m = compose_ldr(uv);

    let luma_nw = dot(rgb_nw, LUMA);
    let luma_ne = dot(rgb_ne, LUMA);
    let luma_sw = dot(rgb_sw, LUMA);
    let luma_se = dot(rgb_se, LUMA);
    let luma_m = dot(rgb_m, LUMA);

    let luma_min = min(luma_m, min(min(luma_nw, luma_ne), min(luma_sw, luma_se)));
    let luma_max = max(luma_m, max(max(luma_nw, luma_ne), max(luma_sw, luma_se)));

    // Edge direction perpendicular to the local luma gradient.
    var dir = vec2<f32>(
        -((luma_nw + luma_ne) - (luma_sw + luma_se)),
        ((luma_nw + luma_sw) - (luma_ne + luma_se)),
    );

    let dir_reduce = max((luma_nw + luma_ne + luma_sw + luma_se) * 0.25 * reduce_mul, reduce_min);
    let rcp_dir_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + dir_reduce);
    dir = clamp(dir * rcp_dir_min, vec2<f32>(-span_max), vec2<f32>(span_max)) * inv;

    // Two-tap inner average and a four-tap wider average; pick the wider one
    // unless it strays outside the local luma range (which would over-blur).
    let rgb_a = 0.5 * (
        compose_ldr(uv + dir * (1.0 / 3.0 - 0.5))
        + compose_ldr(uv + dir * (2.0 / 3.0 - 0.5))
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        compose_ldr(uv + dir * -0.5)
        + compose_ldr(uv + dir * 0.5)
    );

    let luma_b = dot(rgb_b, LUMA);
    if (luma_b < luma_min || luma_b > luma_max) {
        return rgb_a;
    }
    return rgb_b;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var color = compose_ldr(input.uv);
    if (post.fxaa_enabled != 0u) {
        color = fxaa(input.uv);
    }
    return vec4<f32>(color, 1.0);
}
