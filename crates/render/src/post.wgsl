// Post / composite pass: samples the resolved linear-HDR offscreen scene targets,
// applies ambient-only GTAO, tone-maps, encodes to sRGB, and writes the final LDR
// color into egui's framebuffer behind the chrome.

@group(0) @binding(0)
var scene_color: texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler: sampler;

struct PostUniforms {
    gtao_enabled: u32,
    tonemap_enabled: u32,
    tonemap_op: u32,
};
@group(0) @binding(2)
var<uniform> post: PostUniforms;
// Blurred GTAO occlusion (R8, full-res). Multiplied into the scene's ambient
// light when `gtao_enabled` is set.
@group(0) @binding(3)
var gtao_texture: texture_2d<f32>;
// Linear HDR ambient radiance eligible for GTAO attenuation.
@group(0) @binding(4)
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

// Reinhard: the classic `c / (1 + c)` roll-off, applied per channel. Returns
// linear radiance (sRGB encoding happens afterward).
fn reinhard_tonemap(color: vec3<f32>) -> vec3<f32> {
    return color / (vec3<f32>(1.0) + color);
}

// ACES filmic (Stephen Hill's fit of the RRT+ODT, as used by three.js/Filament).
// Operates in the ACEScg primaries via the input/output matrices and returns
// linear sRGB radiance for the downstream sRGB encode.
fn aces_rrt_odt_fit(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.432951) + 0.238081;
    return a / b;
}

fn aces_tonemap(color_in: vec3<f32>) -> vec3<f32> {
    // Column-major (matches the matrix * vector convention).
    let input_mat = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    let output_mat = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    var color = color_in / 0.6;
    color = input_mat * color;
    color = aces_rrt_odt_fit(color);
    color = output_mat * color;
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

// AgX (Troy Sobotka's curve, Benjamin Wrensch's minimal fit, three.js port).
// Returns linear sRGB radiance for the downstream sRGB encode.
fn agx_contrast_approx(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2
        - 40.14 * x4 * x
        + 31.96 * x4
        - 6.868 * x2 * x
        + 0.4298 * x2
        + 0.1191 * x
        - 0.00232;
}

fn agx_tonemap(color_in: vec3<f32>) -> vec3<f32> {
    let srgb_to_rec2020 = mat3x3<f32>(
        vec3<f32>(0.6274, 0.0691, 0.0164),
        vec3<f32>(0.3293, 0.9195, 0.0880),
        vec3<f32>(0.0433, 0.0113, 0.8956),
    );
    let rec2020_to_srgb = mat3x3<f32>(
        vec3<f32>(1.6605, -0.1246, -0.0182),
        vec3<f32>(-0.5876, 1.1329, -0.1006),
        vec3<f32>(-0.0728, -0.0083, 1.1187),
    );
    let inset = mat3x3<f32>(
        vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
        vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        vec3<f32>(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
        vec3<f32>(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;

    var color = srgb_to_rec2020 * color_in;
    color = inset * color;
    color = max(color, vec3<f32>(1e-10));
    color = log2(color);
    color = (color - min_ev) / (max_ev - min_ev);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    color = agx_contrast_approx(color);
    color = outset * color;
    // Undo the 2.2 display gamma baked into the curve, returning linear so the
    // shared sRGB encode below is correct.
    color = pow(max(color, vec3<f32>(0.0)), vec3<f32>(2.2));
    color = rec2020_to_srgb * color;
    return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Select the tone-map operator (indices match `TonemapOperator::shader_index`).
// When tone mapping is disabled the linear radiance passes straight through to
// the sRGB encode (equivalent to the Linear operator).
fn apply_tonemap(color: vec3<f32>) -> vec3<f32> {
    if (post.tonemap_enabled == 0u) {
        return color;
    }
    switch post.tonemap_op {
        case 0u: { return pbr_neutral_tonemap(color); }
        case 1u: { return color; }
        case 2u: { return reinhard_tonemap(color); }
        case 3u: { return aces_tonemap(color); }
        case 4u: { return agx_tonemap(color); }
        default: { return pbr_neutral_tonemap(color); }
    }
}

fn compose_ldr(uv: vec2<f32>) -> vec3<f32> {
    var lit = textureSample(scene_color, scene_sampler, uv).rgb;
    if (post.gtao_enabled != 0u) {
        let ao = textureSample(gtao_texture, scene_sampler, uv).r;
        let ambient = textureSample(ambient_texture, scene_sampler, uv).rgb;
        lit = max(lit - ambient * (1.0 - ao), vec3<f32>(0.0));
    }
    return linear_to_srgb(apply_tonemap(lit));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(compose_ldr(input.uv), 1.0);
}
