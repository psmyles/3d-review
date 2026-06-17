// Scene shader for the viewer: one module covering the shaded / unlit /
// wireframe / uv-checker paths (see CLAUDE.md §4). Loaded into `scene.rs` via
// `include_str!`. The `SceneUniforms` / `VertexInput` layouts here must stay in
// lockstep with the `#[repr(C)]` `SceneUniforms` / `SceneVertex` structs in
// `scene.rs` (invariant 11) — reorder a field here and you must reorder it there.

struct SceneUniforms {
    view_projection: mat4x4<f32>,
    // Inverse view-projection, for reconstructing world ray directions in the
    // skybox pass.
    inv_view_projection: mat4x4<f32>,
    render_options: vec4<f32>,
    // World-space camera eye in `xyz` (`w` is padding), for the specular view vector.
    camera_position: vec4<f32>,
    // Image-based lighting: x = IBL enabled (>0.5), y = intensity, z = show
    // background skybox (>0.5), w = prefiltered-cube max mip LOD.
    env_params: vec4<f32>,
    // View matrix (world -> view), for writing the view-space normal + depth into
    // the SSAO G-buffer (MRT location 2).
    view: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: SceneUniforms;

@group(1) @binding(0)
var checker_texture: texture_2d<f32>;
@group(1) @binding(1)
var checker_sampler: sampler;

// Image-based lighting maps (group 2), precomputed in `ibl.rs` from the chosen
// HDR environment. Bound for every scene draw (the shared pipeline layout
// includes this group); only the shaded path and the skybox sample them.
@group(2) @binding(0)
var irradiance_cube: texture_cube<f32>;
@group(2) @binding(1)
var prefilter_cube: texture_cube<f32>;
@group(2) @binding(2)
var brdf_lut: texture_2d<f32>;
@group(2) @binding(3)
var env_cube: texture_cube<f32>;
@group(2) @binding(4)
var ibl_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) vertex_color: vec4<f32>,
    @location(5) smoothness: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) vertex_color: vec4<f32>,
    @location(4) smoothness: f32,
    @location(5) world_position: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.normal = input.normal;
    output.uv = input.uv;
    output.vertex_color = input.vertex_color;
    output.smoothness = input.smoothness;
    output.world_position = input.position;
    return output;
}

// egui hands us a non-sRGB (gamma-space) framebuffer, so the scene shader must
// do its own color management: decode sRGB inputs to linear, light/tone-map in
// linear, then re-encode to sRGB on output. (The checker texture is sampled as
// Rgba8UnormSrgb and is therefore already linear at this point.)
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

// Fresnel-Schlick with a roughness term, so rough surfaces don't over-brighten
// at grazing angles (the IBL ambient form).
fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    let inv_rough = vec3<f32>(1.0 - roughness);
    return f0 + (max(inv_rough, f0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Environment-lit metallic-roughness PBR for the shaded path. `albedo` is linear.
// The imported material carries no metallic channel (CLAUDE.md Phase 3 decision),
// so the surface is treated as a dielectric and roughness comes from the material
// smoothness. Returns linear radiance (tone mapping happens at the end of fs_main).
fn shade_ibl(albedo: vec3<f32>, world_normal: vec3<f32>, world_pos: vec3<f32>, roughness: f32) -> vec3<f32> {
    let metallic = 0.0;
    let n = normalize(world_normal);
    let v = normalize(uniforms.camera_position.xyz - world_pos);
    let r = reflect(-v, n);
    let n_dot_v = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // Diffuse: cosine-convolved irradiance modulated by albedo.
    let irradiance = textureSampleLevel(irradiance_cube, ibl_sampler, n, 0.0).rgb;
    let diffuse = irradiance * albedo;

    // Specular: split-sum — prefiltered env at the roughness mip, scaled by the
    // BRDF LUT (scale + bias).
    let max_lod = uniforms.env_params.w;
    let prefiltered = textureSampleLevel(prefilter_cube, ibl_sampler, r, roughness * max_lod).rgb;
    let brdf = textureSampleLevel(brdf_lut, ibl_sampler, vec2<f32>(n_dot_v, roughness), 0.0).rg;
    let fresnel = fresnel_schlick_roughness(n_dot_v, f0, roughness);
    let specular = prefiltered * (fresnel * brdf.x + brdf.y);

    let kd = (vec3<f32>(1.0) - fresnel) * (1.0 - metallic);
    return (kd * diffuse + specular) * uniforms.env_params.y;
}

// Scene fragment output (MRT): location 0 is the display-space color the
// composite shows (tone-mapped + sRGB-encoded here, exactly as before bloom);
// location 1 is the linear pre-tone-map HDR radiance bloom thresholds; location 2
// is the SSAO G-buffer (view-space normal in xyz, view-space Z in w). Overlays
// write 0 to locations 1 and 2 so grid / wireframe / normal lines never glow and
// never generate ambient occlusion.
struct FragOutput {
    @location(0) color: vec4<f32>,
    @location(1) bloom: vec4<f32>,
    @location(2) gbuffer: vec4<f32>,
};

@fragment
fn fs_main(input: VertexOutput) -> FragOutput {
    var out: FragOutput;
    out.bloom = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.gbuffer = vec4<f32>(0.0, 0.0, 0.0, 0.0);

    let shading_mode = uniforms.render_options.x;
    let uv_checker_enabled = uniforms.render_options.y > 0.5;
    let tiling = max(uniforms.render_options.z, 1.0);
    // Vertex-color view: -1 off, else mode (0 RGB, 1 Alpha, 2 RGB+A).
    let vertex_color_mode = uniforms.render_options.w;
    // Sample at top level so it stays in uniform control flow.
    let checker = textureSample(checker_texture, checker_sampler, input.uv * tiling);
    let normal_length_sq = dot(input.normal, input.normal);

    // Grid / wireframe / normal lines carry a zero normal — they always render
    // their own vertex color and never pick up the checker / vertex-color tint.
    // They emit no bloom (`out.bloom` stays 0), so overlays never glow.
    if (normal_length_sq < 1e-6) {
        out.color = input.color;
        return out;
    }

    // Real geometry: write the view-space normal + linear view Z into the SSAO
    // G-buffer (both the unlit and shaded paths below carry it). View Z is negative
    // in front of the camera; SSAO treats the zero left by overlays/background as
    // unoccluded.
    let view_pos = uniforms.view * vec4<f32>(input.world_position, 1.0);
    let view_normal = normalize((uniforms.view * vec4<f32>(input.normal, 0.0)).xyz);
    out.gbuffer = vec4<f32>(view_normal, view_pos.z);

    // Work in linear space. The material color is authored in sRGB/gamma space;
    // the checker sample is already linear (sRGB texture format).
    var base_color = srgb_to_linear(input.color.rgb);
    var out_alpha = input.color.a;
    if (uv_checker_enabled) {
        base_color = checker.rgb;
    }
    // The vertex-color view replaces the surface material (overriding the checker
    // if both are on). RGB is gamma-space like the material color; alpha is a raw
    // 0..1 scalar shown as linear grey.
    if (vertex_color_mode >= 0.0) {
        if (vertex_color_mode < 0.5) {
            base_color = srgb_to_linear(input.vertex_color.rgb);
            out_alpha = 1.0;
        } else if (vertex_color_mode < 1.5) {
            base_color = vec3<f32>(input.vertex_color.a);
            out_alpha = 1.0;
        } else {
            base_color = srgb_to_linear(input.vertex_color.rgb);
            out_alpha = input.vertex_color.a;
        }
    }

    if (shading_mode < 1.5) {
        // Unlit: flat emissive material color. Carry its linear value to the bloom
        // target so a bright unlit surface can glow past the threshold.
        out.color = vec4<f32>(linear_to_srgb(base_color), out_alpha);
        out.bloom = vec4<f32>(base_color, 1.0);
        return out;
    }

    let n = normalize(input.normal);
    let smoothness = clamp(input.smoothness, 0.0, 1.0);

    var color_linear: vec3<f32>;
    if (uniforms.env_params.x > 0.5) {
        // Image-based lighting (the default Shaded look): metallic-roughness PBR
        // sampling the precomputed environment maps. Roughness from the material
        // smoothness, clamped away from a perfect mirror so the lowest mip still
        // reads as a surface.
        let roughness = clamp(1.0 - smoothness, 0.04, 1.0);
        color_linear = shade_ibl(base_color, input.normal, input.world_position, roughness);
    } else {
        // Analytic fallback: the neutral-grey hemisphere + Blinn-Phong specular
        // used before IBL. Kept so disabling IBL restores the previous look.
        let light_dir = normalize(vec3<f32>(0.35, 0.82, 0.44));
        let diffuse = max(dot(n, light_dir), 0.0);
        let hemi_t = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
        let sky = vec3<f32>(0.63, 0.63, 0.63);
        let ground = vec3<f32>(0.11, 0.11, 0.11);
        let hemi = mix(ground, sky, hemi_t);
        let lighting = hemi * 0.55 + vec3<f32>(1.0, 1.0, 1.0) * (0.20 + diffuse * 0.75);
        let lit = base_color * lighting;

        let view_dir = normalize(uniforms.camera_position.xyz - input.world_position);
        let half_dir = normalize(light_dir + view_dir);
        let shininess = exp2(1.0 + smoothness * 10.0);
        let spec = pow(max(dot(n, half_dir), 0.0), shininess) * smoothness * step(0.0, diffuse);
        color_linear = lit + vec3<f32>(spec);
    }

    let mapped = pbr_neutral_tonemap(color_linear);
    out.color = vec4<f32>(linear_to_srgb(mapped), out_alpha);
    // Bloom reads the pre-tone-map linear radiance, so bright reflections /
    // highlights above the threshold glow.
    out.bloom = vec4<f32>(color_linear, 1.0);
    return out;
}

// --- Skybox: draw the environment cubemap as the viewport background. -------
// A fullscreen triangle whose per-pixel world ray direction is reconstructed
// from the inverse view-projection, sampled into the env cube. Drawn first in
// the offscreen pass (depth-test always, no depth write) so scene geometry
// composites over it.
struct SkyOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_skybox(@builtin(vertex_index) vertex_index: u32) -> SkyOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let xy = positions[vertex_index];
    var out: SkyOutput;
    // z = 1 keeps the skybox at the far plane; depth write is off so this is only
    // for completeness.
    out.clip_position = vec4<f32>(xy, 1.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_skybox(input: SkyOutput) -> FragOutput {
    // Unproject a far-plane point to world space, then form the ray from the eye.
    let world = uniforms.inv_view_projection * vec4<f32>(input.ndc, 1.0, 1.0);
    let world_pos = world.xyz / world.w;
    let dir = normalize(world_pos - uniforms.camera_position.xyz);
    let color = textureSampleLevel(env_cube, ibl_sampler, dir, 0.0).rgb * uniforms.env_params.y;
    let mapped = pbr_neutral_tonemap(color);
    var out: FragOutput;
    out.color = vec4<f32>(linear_to_srgb(mapped), 1.0);
    // Full linear HDR sky into the bloom target — bright sky regions glow.
    out.bloom = vec4<f32>(color, 1.0);
    // The sky is background: zero G-buffer so SSAO leaves it unoccluded.
    out.gbuffer = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    return out;
}
