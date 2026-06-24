// Post / composite pass: samples the resolved linear-HDR offscreen scene targets,
// applies ambient-only GTAO, adds bloom, tone-maps, encodes to sRGB, and writes
// the final LDR color into egui's framebuffer behind the chrome. FXAA operates on
// the composed display-space result.

@group(0) @binding(0)
var scene_color: texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler: sampler;

struct PostUniforms {
    // view -> world, for the G-buffer's view-space geom + bent normals.
    inv_view: mat4x4<f32>,
    // view -> clip, to reconstruct view position from the G-buffer depth (n·v).
    proj: mat4x4<f32>,
    inv_resolution: vec2<f32>,
    fxaa_enabled: u32,
    bloom_enabled: u32,
    bloom_intensity: f32,
    gtao_enabled: u32,
    tonemap_enabled: u32,
    tonemap_op: u32,
    // IBL on: gate the bent-normal diffuse re-lighting + split-specular occlusion.
    ibl_enabled: u32,
    // Environment yaw (radians), matching the scene shader's rotation.
    env_yaw: f32,
    // IBL intensity, to recover the ambient albedo proxy for multi-bounce.
    ibl_intensity: f32,
    // Orthographic projection flag (>0.5), for the view-position reconstruction.
    is_ortho: f32,
};
@group(0) @binding(2)
var<uniform> post: PostUniforms;
// Blurred linear-HDR bloom (half-res; the filtering sampler upsamples it). Added
// back over the scene when `bloom_enabled` is set.
@group(0) @binding(3)
var bloom_texture: texture_2d<f32>;
// Blurred GTAO output (Rgba16Float, full-res): view-space bent normal in xyz,
// occlusion in w. Used to attenuate + re-light the scene's ambient when
// `gtao_enabled` is set.
@group(0) @binding(4)
var gtao_texture: texture_2d<f32>;
// Linear HDR diffuse ambient radiance eligible for GTAO attenuation / re-lighting.
@group(0) @binding(5)
var ambient_texture: texture_2d<f32>;
// Linear HDR IBL specular (rgb) + roughness (a), for the bent-normal-aware
// specular occlusion.
@group(0) @binding(6)
var specular_texture: texture_2d<f32>;
// Single-sample GTAO G-buffer: view-space geom normal (xyz) + view Z (w).
@group(0) @binding(7)
var gtao_gbuffer: texture_2d<f32>;
// Diffuse irradiance cube, re-sampled with the bent / geom normal.
@group(0) @binding(8)
var irradiance_cube: texture_cube<f32>;

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

// Rotate an irradiance lookup about world Y by the negative environment yaw, so
// the re-sampled ambient tracks the rotation slider (matching `scene.wgsl`'s
// `env_sample_dir`).
fn env_dir(dir: vec3<f32>) -> vec3<f32> {
    let angle = -post.env_yaw;
    let s = sin(angle);
    let c = cos(angle);
    return vec3<f32>(c * dir.x + s * dir.z, dir.y, -s * dir.x + c * dir.z);
}

// Activision's GTAO multi-bounce: turns the scalar visibility into a colored
// occlusion that lets light bounce back in bright-albedo cavities (a cubic fit per
// channel, clamped to be no darker than the raw visibility).
fn gtao_multibounce(visibility: f32, albedo: vec3<f32>) -> vec3<f32> {
    let a = 2.0404 * albedo - vec3<f32>(0.3324);
    let b = -4.7951 * albedo + vec3<f32>(0.6417);
    let c = 2.7552 * albedo + vec3<f32>(0.6903);
    return max(vec3<f32>(visibility), ((visibility * a + b) * visibility + c) * visibility);
}

// Lagarde/Frostbite specular occlusion: roughness- and view-aware so creases
// occlude specular without flattening grazing reflections.
fn specular_occlusion(n_dot_v: f32, ao: f32, roughness: f32) -> f32 {
    return clamp(pow(n_dot_v + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao, 0.0, 1.0);
}

// Reconstruct view-space position from a pixel's uv + its stored view Z, mirroring
// the GTAO shader (perspective divides by depth; orthographic is linear in ndc).
fn reconstruct_view_pos(uv: vec2<f32>, view_z: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    if (post.is_ortho > 0.5) {
        let x = (ndc.x - post.proj[3][0]) / post.proj[0][0];
        let y = (ndc.y - post.proj[3][1]) / post.proj[1][1];
        return vec3<f32>(x, y, view_z);
    }
    let x = ndc.x * (-view_z) / post.proj[0][0];
    let y = ndc.y * (-view_z) / post.proj[1][1];
    return vec3<f32>(x, y, view_z);
}

fn compose_ldr(uv: vec2<f32>) -> vec3<f32> {
    var lit = textureSample(scene_color, scene_sampler, uv).rgb;
    // Sample every 2D source at the top level (uniform control flow) so the
    // branches below stay valid; the cube uses explicit-LOD sampling, which is
    // allowed in non-uniform flow.
    let gt = textureSample(gtao_texture, scene_sampler, uv);
    let gbuf = textureSample(gtao_gbuffer, scene_sampler, uv);
    let ambient = textureSample(ambient_texture, scene_sampler, uv).rgb;
    let spec = textureSample(specular_texture, scene_sampler, uv);
    let bloom = textureSample(bloom_texture, scene_sampler, uv).rgb;

    // GTAO only touches foreground mesh pixels (the G-buffer carries a unit normal
    // and a negative view Z there; background wrote zero).
    let is_foreground = dot(gbuf.xyz, gbuf.xyz) > 0.25 && gbuf.w < -1e-4;
    if (post.gtao_enabled != 0u && is_foreground) {
        let ao = clamp(gt.w, 0.0, 1.0);
        if (post.ibl_enabled != 0u) {
            let geom_v = normalize(gbuf.xyz);
            var bent_v = gt.xyz;
            if (dot(bent_v, bent_v) < 1e-6) {
                bent_v = geom_v;
            } else {
                bent_v = normalize(bent_v);
            }
            // View-space normals -> world for the cube lookup.
            let geom_w = normalize((post.inv_view * vec4<f32>(geom_v, 0.0)).xyz);
            let bent_w = normalize((post.inv_view * vec4<f32>(bent_v, 0.0)).xyz);
            let irr_geom = textureSampleLevel(irradiance_cube, scene_sampler, env_dir(geom_w), 0.0).rgb;
            let irr_bent = textureSampleLevel(irradiance_cube, scene_sampler, env_dir(bent_w), 0.0).rgb;
            let eps = vec3<f32>(1e-4);
            // The diffuse ambient already encodes kd*albedo*intensity*irr_geom, so
            // dividing recovers an albedo proxy for the colored multi-bounce, and
            // the irradiance ratio swaps the geom-normal lighting for the bent one.
            let albedo = clamp(ambient / max(irr_geom * post.ibl_intensity, eps), vec3<f32>(0.0), vec3<f32>(1.0));
            let mb = gtao_multibounce(ao, albedo);
            let amb_bent = ambient * (irr_bent / max(irr_geom, eps)) * mb;

            // Bent-normal-aware specular occlusion on the split specular target.
            let p = reconstruct_view_pos(uv, gbuf.w);
            var view_v = vec3<f32>(0.0, 0.0, 1.0);
            if (post.is_ortho <= 0.5) {
                view_v = normalize(-p);
            }
            let n_dot_v = clamp(dot(geom_v, view_v), 1e-4, 1.0);
            let so = specular_occlusion(n_dot_v, ao, spec.a);

            // Additive correction over the full radiance (location 0): swap the
            // geom-normal ambient for the bent-normal one and occlude the specular,
            // leaving direct + emissive light untouched.
            lit = max(lit - ambient + amb_bent - spec.rgb * (1.0 - so), vec3<f32>(0.0));
        } else {
            // IBL off (analytic ambient): the original scalar attenuation.
            lit = max(lit - ambient * (1.0 - ao), vec3<f32>(0.0));
        }
    }
    if (post.bloom_enabled != 0u) {
        lit = lit + bloom * post.bloom_intensity;
    }
    return linear_to_srgb(apply_tonemap(lit));
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
