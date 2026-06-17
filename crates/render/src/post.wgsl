// Post / composite pass: samples the resolved offscreen scene color and writes it
// into egui's framebuffer, behind the chrome (CLAUDE.md render roadmap, Phase 1),
// optionally running FXAA first (Phase 2).
//
// The scene shader still owns lighting, tone mapping and the sRGB encode, so the
// offscreen target already holds final display-space color and this pass either
// blits it straight through (byte-for-byte what egui produced when the scene drew
// directly into its pass) or runs an FXAA edge-blend over it. FXAA operates on
// display-space luma, which is exactly the space it was designed for. This is the
// seam where tone mapping / bloom move once the scene renders in linear HDR (see
// `targets.rs` SCENE_HDR_FORMAT).

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

// Classic NVIDIA FXAA II ("console" quality) edge-blend. Cheap, dependency-free,
// and a good match for a single full-screen post pass over LDR color.
fn fxaa(uv: vec2<f32>) -> vec3<f32> {
    let span_max = 8.0;
    let reduce_mul = 1.0 / 8.0;
    let reduce_min = 1.0 / 128.0;
    let inv = post.inv_resolution;

    let rgb_nw = textureSample(scene_color, scene_sampler, uv + vec2<f32>(-1.0, -1.0) * inv).rgb;
    let rgb_ne = textureSample(scene_color, scene_sampler, uv + vec2<f32>(1.0, -1.0) * inv).rgb;
    let rgb_sw = textureSample(scene_color, scene_sampler, uv + vec2<f32>(-1.0, 1.0) * inv).rgb;
    let rgb_se = textureSample(scene_color, scene_sampler, uv + vec2<f32>(1.0, 1.0) * inv).rgb;
    let rgb_m = textureSample(scene_color, scene_sampler, uv).rgb;

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
        textureSample(scene_color, scene_sampler, uv + dir * (1.0 / 3.0 - 0.5)).rgb
        + textureSample(scene_color, scene_sampler, uv + dir * (2.0 / 3.0 - 0.5)).rgb
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        textureSample(scene_color, scene_sampler, uv + dir * -0.5).rgb
        + textureSample(scene_color, scene_sampler, uv + dir * 0.5).rgb
    );

    let luma_b = dot(rgb_b, LUMA);
    if (luma_b < luma_min || luma_b > luma_max) {
        return rgb_a;
    }
    return rgb_b;
}

// sRGB transfer functions, so bloom can be added in linear light. The scene
// color is display-space (sRGB-encoded by the scene shader); bloom is linear HDR.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // The display-space scene color (FXAA'd or straight). When both bloom and SSAO
    // are off this is returned untouched, so the composite stays byte-for-byte what
    // it was before those effects existed.
    var color = textureSample(scene_color, scene_sampler, input.uv).rgb;
    if (post.fxaa_enabled != 0u) {
        color = fxaa(input.uv);
    }

    let bloom_on = post.bloom_enabled != 0u;
    let ssao_on = post.ssao_enabled != 0u;
    if (bloom_on || ssao_on) {
        // Work in linear light: darken by ambient occlusion, then add the blurred
        // bloom (which is not occluded — highlights still glow), then re-encode.
        var lit = srgb_to_linear(color);
        if (ssao_on) {
            let ao = textureSample(ssao_texture, scene_sampler, input.uv).r;
            lit = lit * ao;
        }
        if (bloom_on) {
            let bloom = textureSample(bloom_texture, scene_sampler, input.uv).rgb;
            lit = lit + bloom * post.bloom_intensity;
        }
        return vec4<f32>(linear_to_srgb(lit), 1.0);
    }

    return vec4<f32>(color, 1.0);
}
