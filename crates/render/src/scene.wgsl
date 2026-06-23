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
    // Projection metadata: x = orthographic projection (>0.5), y/z/w unused.
    projection_params: vec4<f32>,
    // View matrix (world -> view), for writing the view-space normal + depth into
    // the separate SSAO G-buffer pass.
    view: mat4x4<f32>,
    // Selection-flash highlight: rgb is the gamma-space highlight color, a is the
    // flash fade (1 at the start of a selection flash down to 0). Read only by
    // `fs_selection`; zero alpha while nothing is flashing.
    selection_color: vec4<f32>,
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

// Per-material parameters (bind group 3). Must match the `#[repr(C)]`
// `MaterialUniform` in `material.rs` (invariant 11). The mesh sources its base
// color / metallic / roughness / emissive from here; `base_color`/`emissive` are
// linear. `params` = (metallic, roughness, slot_flags bitfield, alpha mode).
// `channels0` selects the channel (0 R … 3 A) feeding slots 0..3; `channels1`
// does the same for slots 4..6 with `w` carrying the alpha cutoff.
struct MaterialUniform {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,
    channels0: vec4<f32>,
    channels1: vec4<f32>,
};

@group(3) @binding(0)
var<uniform> material: MaterialUniform;
// The seven material texture slots + their shared sampler. The same cached view
// may be bound to several slots (packed maps); the shader picks the configured
// channel per scalar property. Unassigned slots bind a neutral 1×1 fallback and
// are gated off by the slot-flags bitfield in `params.z`.
@group(3) @binding(1)
var base_color_tex: texture_2d<f32>;
@group(3) @binding(2)
var normal_tex: texture_2d<f32>;
@group(3) @binding(3)
var roughness_tex: texture_2d<f32>;
@group(3) @binding(4)
var metallic_tex: texture_2d<f32>;
@group(3) @binding(5)
var ao_tex: texture_2d<f32>;
@group(3) @binding(6)
var emissive_tex: texture_2d<f32>;
@group(3) @binding(7)
var opacity_tex: texture_2d<f32>;
@group(3) @binding(8)
var material_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // World-space tangent (xyz) + handedness sign (w), for normal mapping. Zeroed
    // on overlay/line/UV-fill geometry (those never sample a normal map).
    @location(3) tangent: vec4<f32>,
    // The DCC vertex-color attribute on the mesh; on overlay/line/UV-fill geometry
    // (zero normal) this instead carries the flat color the overlay path returns.
    @location(4) vertex_color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) vertex_color: vec4<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) tangent: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.uv = input.uv;
    output.vertex_color = input.vertex_color;
    output.world_position = input.position;
    output.tangent = input.tangent;
    return output;
}

// Select one channel of a sampled texel by index (0 R, 1 G, 2 B, 3 A), so a packed
// map can route any channel into a scalar property.
fn select_channel(texel: vec4<f32>, index: f32) -> f32 {
    if (index < 0.5) { return texel.r; }
    if (index < 1.5) { return texel.g; }
    if (index < 2.5) { return texel.b; }
    return texel.a;
}

// Perturb the geometric normal `n` by a tangent-space normal-map sample, using the
// vertex tangent (xyz) + handedness (w) to build the TBN basis. Geometry is
// world-baked, so the tangent is already world-space (no model matrix).
fn apply_normal_map(n: vec3<f32>, tangent: vec4<f32>, sample_rgb: vec3<f32>) -> vec3<f32> {
    let geo_n = normalize(n);
    // Gram-Schmidt orthonormalize the tangent against the normal.
    let t = normalize(tangent.xyz - geo_n * dot(geo_n, tangent.xyz));
    if (dot(t, t) < 1e-8) {
        return geo_n;
    }
    let b = cross(geo_n, t) * tangent.w;
    let m = sample_rgb * 2.0 - vec3<f32>(1.0);
    return normalize(t * m.x + b * m.y + geo_n * m.z);
}

// Decode sRGB-authored vertex colors into the scene's linear HDR working space.
// Tone mapping and final sRGB encoding happen once in `post.wgsl`. The checker
// texture is sampled as Rgba8UnormSrgb and is therefore already linear here.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// Fresnel-Schlick with a roughness term, so rough surfaces don't over-brighten
// at grazing angles (the IBL ambient form).
fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    let inv_rough = vec3<f32>(1.0 - roughness);
    return f0 + (max(inv_rough, f0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

struct ShadingResult {
    color: vec3<f32>,
    ambient: vec3<f32>,
};

// Environment-lit metallic-roughness PBR for the shaded path. `albedo`, `metallic`
// and `roughness` come from the per-material uniform (bind group 3). Returns linear
// radiance and the diffuse ambient term SSAO may attenuate (tone mapping happens in
// post).
fn shade_ibl(albedo: vec3<f32>, world_normal: vec3<f32>, world_pos: vec3<f32>, roughness: f32, metallic: f32) -> ShadingResult {
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
    let ambient = kd * diffuse * uniforms.env_params.y;
    return ShadingResult(ambient + specular * uniforms.env_params.y, ambient);
}

// Scene fragment output (MRT): location 0 is the linear HDR color the composite
// tone-maps; location 1 is the linear HDR radiance bloom thresholds;
// location 2 is the linear ambient radiance SSAO may attenuate. Overlays write 0
// alpha to locations 1 and 2 so grid / wireframe / normal lines never glow and
// never darken.
struct FragOutput {
    @location(0) color: vec4<f32>,
    @location(1) bloom: vec4<f32>,
    @location(2) ambient: vec4<f32>,
};

@fragment
fn fs_main(input: VertexOutput) -> FragOutput {
    var out: FragOutput;
    out.bloom = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    out.ambient = vec4<f32>(0.0, 0.0, 0.0, 0.0);

    let shading_mode = uniforms.render_options.x;
    let uv_checker_enabled = uniforms.render_options.y > 0.5;
    let tiling = max(uniforms.render_options.z, 1.0);
    // Vertex-color view: -1 off, else mode (0 RGB, 1 Alpha, 2 RGB+A).
    let vertex_color_mode = uniforms.render_options.w;
    // FBX (and the DCC tools that feed it) author UVs with a bottom-left origin
    // (V up), but wgpu samples textures from a top-left origin (V down), so the
    // raw UV reads textures vertically mirrored. Flip V once here, at the single
    // UV→texel boundary, for both the checker and every material slot — the model
    // geometry's UVs (and the 2D UV viewport, which positions by `input.position`)
    // stay untouched, so nothing downstream is mirrored twice.
    let tex_uv = vec2<f32>(input.uv.x, 1.0 - input.uv.y);
    // Sample at top level so it stays in uniform control flow.
    let checker = textureSample(checker_texture, checker_sampler, tex_uv * tiling);
    // Sample every material slot at the top level (uniform control flow), so the
    // branches below can use them without re-sampling in non-uniform flow. Slots
    // are gated by the slot-flags bitfield; an unbound slot reads its 1×1 fallback.
    let tex_base = textureSample(base_color_tex, material_sampler, tex_uv);
    let tex_normal = textureSample(normal_tex, material_sampler, tex_uv);
    let tex_roughness = textureSample(roughness_tex, material_sampler, tex_uv);
    let tex_metallic = textureSample(metallic_tex, material_sampler, tex_uv);
    let tex_ao = textureSample(ao_tex, material_sampler, tex_uv);
    let tex_emissive = textureSample(emissive_tex, material_sampler, tex_uv);
    let tex_opacity = textureSample(opacity_tex, material_sampler, tex_uv);
    let slot_flags = u32(material.params.z);
    let normal_length_sq = dot(input.normal, input.normal);

    // Grid / wireframe / normal lines carry a zero normal — they always render
    // their own vertex color and never pick up the checker / vertex-color tint.
    // They emit no bloom. Their ambient output has zero color but the overlay
    // alpha, masking the mesh ambient beneath them so post AO only darkens the
    // still-visible mesh fraction, not the overlay color itself.
    if (normal_length_sq < 1e-6) {
        out.color = vec4<f32>(srgb_to_linear(input.vertex_color.rgb), input.vertex_color.a);
        out.ambient = vec4<f32>(0.0, 0.0, 0.0, input.vertex_color.a);
        return out;
    }

    // Which material slots carry a texture (the slot-flags bitfield in params.z).
    let has_base = (slot_flags & 1u) != 0u;
    let has_normal = (slot_flags & 2u) != 0u;
    let has_roughness = (slot_flags & 4u) != 0u;
    let has_metallic = (slot_flags & 8u) != 0u;
    let has_ao = (slot_flags & 16u) != 0u;
    let has_emissive = (slot_flags & 32u) != 0u;
    let has_opacity = (slot_flags & 64u) != 0u;

    // Work in linear space. The material base color is stored linear in the uniform
    // (no decode); the base-color texture (sRGB format → already linear here)
    // modulates it. The checker sample is already linear (sRGB texture format).
    var base_color = material.base_color.rgb;
    if (has_base) {
        base_color = base_color * tex_base.rgb;
    }
    var out_alpha = material.base_color.a;
    if (has_opacity) {
        out_alpha = out_alpha * select_channel(tex_opacity, material.channels1.z);
    }
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

    // Alpha-clip cutout (material flag): drop fully-transparent fragments before
    // any shading. Blend mode keeps the fragment and lets the MRT alpha-blend it.
    let alpha_mode = material.params.w;
    if (alpha_mode > 1.5 && out_alpha < material.channels1.w) {
        discard;
    }

    if (shading_mode < 1.5) {
        // Unlit: flat emissive material color. Carry its linear value to the bloom
        // target so a bright unlit surface can glow past the threshold.
        out.color = vec4<f32>(base_color, out_alpha);
        out.bloom = vec4<f32>(base_color, 1.0);
        return out;
    }

    // The shading normal: the geometric normal, optionally perturbed by a
    // tangent-space normal map.
    var world_normal = normalize(input.normal);
    if (has_normal) {
        world_normal = apply_normal_map(input.normal, input.tangent, tex_normal.rgb);
    }
    let n = world_normal;
    // Metallic + roughness from the uniform, modulated by their (channel-routed)
    // textures when bound.
    var metallic = material.params.x;
    if (has_metallic) {
        metallic = metallic * select_channel(tex_metallic, material.channels0.w);
    }
    metallic = clamp(metallic, 0.0, 1.0);
    var roughness_value = material.params.y;
    if (has_roughness) {
        roughness_value = roughness_value * select_channel(tex_roughness, material.channels0.z);
    }
    // Ambient-occlusion factor (channel-routed), darkening only the ambient term.
    var ao = 1.0;
    if (has_ao) {
        ao = select_channel(tex_ao, material.channels1.x);
    }

    var color_linear: vec3<f32>;
    var ambient_linear: vec3<f32>;
    if (uniforms.env_params.x > 0.5) {
        // Image-based lighting (the default Shaded look): metallic-roughness PBR
        // sampling the precomputed environment maps. Roughness clamped away from a
        // perfect mirror so the lowest mip still reads as a surface.
        let roughness = clamp(roughness_value, 0.04, 1.0);
        let shaded = shade_ibl(base_color, world_normal, input.world_position, roughness, metallic);
        color_linear = shaded.color;
        ambient_linear = shaded.ambient;
    } else {
        // Analytic fallback: the neutral-grey hemisphere + Blinn-Phong specular
        // used before IBL. Kept so disabling IBL restores the previous look.
        // Smoothness (glossiness) is the complement of the (textured) roughness.
        let smoothness = clamp(1.0 - roughness_value, 0.0, 1.0);
        let light_dir = normalize(vec3<f32>(0.35, 0.82, 0.44));
        let diffuse = max(dot(n, light_dir), 0.0);
        let hemi_t = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
        let sky = vec3<f32>(0.63, 0.63, 0.63);
        let ground = vec3<f32>(0.11, 0.11, 0.11);
        let hemi = mix(ground, sky, hemi_t);
        let ambient_light = hemi * 0.55 + vec3<f32>(1.0, 1.0, 1.0) * 0.20;
        let direct_light = vec3<f32>(1.0, 1.0, 1.0) * (diffuse * 0.75);
        let lighting = ambient_light + direct_light;
        let lit = base_color * lighting;

        let view_dir = normalize(uniforms.camera_position.xyz - input.world_position);
        let half_dir = normalize(light_dir + view_dir);
        let shininess = exp2(1.0 + smoothness * 10.0);
        let spec = pow(max(dot(n, half_dir), 0.0), shininess) * smoothness * step(0.0, diffuse);
        color_linear = lit + vec3<f32>(spec);
        ambient_linear = base_color * ambient_light;
    }

    // Ambient occlusion darkens only the ambient term: remove the un-occluded
    // ambient from the lit color and add back the occluded fraction, leaving direct
    // + specular light untouched (matching the post SSAO compose).
    color_linear = color_linear + ambient_linear * (ao - 1.0);
    ambient_linear = ambient_linear * ao;

    // Emissive adds on top of the lit result (linear), and contributes to bloom so
    // a bright emissive surface can glow. Default emissive is black (no effect);
    // an emissive texture (when bound) modulates it.
    var emissive = material.emissive.rgb;
    if (has_emissive) {
        // An emissive map with the default (black) factor still shows at full
        // strength; a non-zero factor tints it.
        var factor = material.emissive.rgb;
        if (all(factor <= vec3<f32>(0.0))) {
            factor = vec3<f32>(1.0);
        }
        emissive = tex_emissive.rgb * factor;
    }
    color_linear = color_linear + emissive;

    out.color = vec4<f32>(color_linear, out_alpha);
    out.bloom = vec4<f32>(color_linear, 1.0);
    out.ambient = vec4<f32>(ambient_linear, out_alpha);
    return out;
}

// Selection flash: a flat color fill over the selected triangles (the solo index
// buffer, drawn over the mesh vertices). Ignores the mesh's lighting and material
// entirely — it emits the uniform highlight color tinted by the flash fade alpha.
// Writes zero bloom so the flash never glows, and zero ambient *color* with the
// fade alpha so it masks (rather than darkens via SSAO) the mesh ambient beneath,
// matching the other overlays. As `selection_color.a` reaches 0 the fill blends
// fully away.
@fragment
fn fs_selection(input: VertexOutput) -> FragOutput {
    var out: FragOutput;
    let fade = uniforms.selection_color.a;
    out.color = vec4<f32>(srgb_to_linear(uniforms.selection_color.rgb), fade);
    out.bloom = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    out.ambient = vec4<f32>(0.0, 0.0, 0.0, fade);
    return out;
}

@fragment
fn fs_ssao_gbuffer(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal_length_sq = dot(input.normal, input.normal);
    if (normal_length_sq < 1e-6) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // View Z is negative in front of the camera; SSAO treats zero as background.
    let view_pos = uniforms.view * vec4<f32>(input.world_position, 1.0);
    let view_normal = normalize((uniforms.view * vec4<f32>(input.normal, 0.0)).xyz);
    return vec4<f32>(view_normal, view_pos.z);
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
    // Reversed-Z far depth is 0; depth write is off and the pipeline always
    // passes, so this is only for completeness.
    out.clip_position = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_skybox(input: SkyOutput) -> FragOutput {
    // Perspective uses an infinite reversed projection, so unproject a finite
    // near-plane point. Orthographic keeps a finite far plane and uses that point
    // to preserve the existing viewport-varying background direction.
    let unproject_depth = select(1.0, 0.0, uniforms.projection_params.x > 0.5);
    let world = uniforms.inv_view_projection * vec4<f32>(input.ndc, unproject_depth, 1.0);
    let world_pos = world.xyz / world.w;
    let dir = normalize(world_pos - uniforms.camera_position.xyz);
    let color = textureSampleLevel(env_cube, ibl_sampler, dir, 0.0).rgb * uniforms.env_params.y;
    var out: FragOutput;
    out.color = vec4<f32>(color, 1.0);
    out.bloom = vec4<f32>(color, 1.0);
    // The sky is background: no ambient target, so SSAO never darkens it.
    out.ambient = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    return out;
}
