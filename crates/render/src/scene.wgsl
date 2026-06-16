// Scene shader for the viewer: one module covering the shaded / unlit /
// wireframe / uv-checker paths (see CLAUDE.md §4). Loaded into `scene.rs` via
// `include_str!`. The `SceneUniforms` / `VertexInput` layouts here must stay in
// lockstep with the `#[repr(C)]` `SceneUniforms` / `SceneVertex` structs in
// `scene.rs` (invariant 11) — reorder a field here and you must reorder it there.

struct SceneUniforms {
    view_projection: mat4x4<f32>,
    render_options: vec4<f32>,
    // World-space camera eye in `xyz` (`w` is padding), for the specular view vector.
    camera_position: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: SceneUniforms;

@group(1) @binding(0)
var checker_texture: texture_2d<f32>;
@group(1) @binding(1)
var checker_sampler: sampler;

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

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
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
    if (normal_length_sq < 1e-6) {
        return input.color;
    }

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
        return vec4<f32>(linear_to_srgb(base_color), out_alpha);
    }

    let n = normalize(input.normal);
    let light_dir = normalize(vec3<f32>(0.35, 0.82, 0.44));
    let diffuse = max(dot(n, light_dir), 0.0);
    let hemi_t = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
    // Neutral grey hemisphere (Rec.709 luma of the former bluish sky/ground) so
    // lighting only scales brightness and never shifts hue/saturation.
    let sky = vec3<f32>(0.63, 0.63, 0.63);
    let ground = vec3<f32>(0.11, 0.11, 0.11);
    let hemi = mix(ground, sky, hemi_t);
    let lighting = hemi * 0.55 + vec3<f32>(1.0, 1.0, 1.0) * (0.20 + diffuse * 0.75);
    let lit = base_color * lighting;

    // Smoothness-driven Blinn-Phong specular: the imported material smoothness
    // (glossiness == 1 - roughness) both tightens the highlight (exponent) and
    // scales its strength, so rough surfaces show no glint and smooth ones a
    // sharp one. The view vector comes from the world-space camera eye.
    let smoothness = clamp(input.smoothness, 0.0, 1.0);
    let view_dir = normalize(uniforms.camera_position.xyz - input.world_position);
    let half_dir = normalize(light_dir + view_dir);
    let shininess = exp2(1.0 + smoothness * 10.0);
    let spec = pow(max(dot(n, half_dir), 0.0), shininess) * smoothness * step(0.0, diffuse);
    let mapped = pbr_neutral_tonemap(lit + vec3<f32>(spec));
    return vec4<f32>(linear_to_srgb(mapped), out_alpha);
}
