// 3D Review — THE shader source. One file, 17 programs, both backends.
//
// This is sokol-shdc's annotated GLSL (Vulkan syntax: separate `texture2D`/
// `textureCube` and `sampler` objects, combined at each tap). `scripts/gen-shaders`
// runs shdc over it twice — once for the per-backend HLSL5 + MSL sources, once for
// the sokol_gfx reflection as a Rust module — into `generated/`, which is checked
// in. `build.rs` then compiles *this host's* generated source to committed bytecode
// (`fxc` → DXBC on Windows, `xcrun metal` → `.metallib` on macOS). So nothing
// compiles a shader at run time on either OS, a broken shader is a build error, and
// there is one source instead of two hand-kept twins (`docs/ARCHITECTURE.md`,
// Platform decisions D4/D5).
//
// **Edit this file, re-run the script, commit both.** And note D5's discipline: the
// bytecode is per-host, so a shader edit committed from one OS ships stale bytecode
// for the other until that host rebuilds.
//
// ---------------------------------------------------------------------------
// Binding plan. Written out explicitly on every declaration so that one
// `sg::Bindings` value serves every scene pipeline — the bindings are a value
// re-applied after each `apply_pipeline`, not a per-pass side effect.
//
//   Uniform blocks (per stage — `sg_shader_uniform_block` carries one stage, so
//   `SceneUniforms` is declared twice, once per stage, with the same bytes):
//     0 scene_vs (VS)   1 scene_fs (FS)   2 material (FS)   3 line_params (VS)
//   Other programs number their own blocks from 0.
//
//   Views (textures and storage buffers share one bind space in sokol):
//     0 checker · 1 irradiance · 2 prefilter · 3 brdf · 4 env cube
//     5..11 the seven material slots
//     12..15 the VS deform storage buffers (influences / palette / morph deltas /
//            morph weights)
//     16..17 the line programs' pulled vertices and the wireframe's edge list (VS)
//   Samplers: 0 checker · 1 IBL · 2 material (anisotropic)
//
// A shader declares **only** the slots it actually uses: sokol validates that every
// declared view is bound, so a blanket declaration would force the skybox and the
// line pipelines to bind material textures they never read.
//
// ---------------------------------------------------------------------------
// Rules this file must keep:
//
//  * **Uniform-block members are `float`/`vec2`/`vec3`/`vec4`/`int`/`ivec*`/`mat4`
//    only** — shdc's std140 subset. The old HLSL `uint` flags are `int` here (same
//    bits). No `mat3` *members*: the tone-map matrices are locals built with
//    `mat3(c0, c1, c2)`, which is column-major in GLSL and so is the verbatim
//    equivalent of the old `mat_from_cols` + `mul`.
//  * **Storage structs are std430.** `MorphEntry` is declared as seven scalars, not
//    a `uint` + two `vec3`: a `vec3` member would pad the struct to 48 bytes and
//    shift every entry. It must stay 28.
//  * **Every texture read under non-uniform control flow is `textureLod`** (never
//    plain `texture`), because a per-pixel branch leaves the 2×2 quad's other lanes
//    holding stale registers and the implicit derivative becomes garbage — a
//    flickering mip, not a static wrong one. `fs_main` samples every material slot
//    at the top, under uniform control flow, for exactly this reason.
//  * `gl_FragCoord` is top-left on both backends (`sg_features.origin_top_left`),
//    matching the old `SV_Position` reads in the Tex and GTAO passes.
//  * Depth is **Reversed-Z** (near → 1, far → 0): cleared to 0, compared
//    `GreaterEqual`, with an infinite reversed perspective.
//  * Matrices arrive column-major (`glam::Mat4::to_cols_array_2d`), so `M * v` is
//    the plain matrix × column-vector product — what `mul(M, v)` meant in HLSL.
//    GTAO's element reads flip accordingly: HLSL's `proj[row][col]` is `proj[col][row]`
//    here.

// ===========================================================================
// Shared blocks
// ===========================================================================

// sRGB transfer functions, in one place rather than duplicated across three files
// as they were in HLSL. `max(...)` guards a negative `pow` base — the base is
// non-negative for any real color, so it changes nothing for valid input.
@block srgb
vec3 srgb_to_linear(vec3 c) {
    vec3 lo = c / 12.92;
    vec3 hi = pow(max((c + 0.055) / 1.055, vec3(0.0)), vec3(2.4));
    // `step(c, edge)` is 1 where c <= edge, so `mix(hi, lo, cutoff)` takes the low
    // branch there — the same selection the HLSL `lerp(hi, lo, step(...))` made.
    vec3 cutoff = step(c, vec3(0.04045));
    return mix(hi, lo, cutoff);
}

vec3 linear_to_srgb(vec3 c) {
    vec3 lo = c * 12.92;
    vec3 hi = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    vec3 cutoff = step(c, vec3(0.0031308));
    return mix(hi, lo, cutoff);
}
@end

// The oversized triangle that covers the whole target in one primitive, with no
// vertex buffer — the vertices come from `gl_VertexIndex`.
@block fullscreen
vec2 fullscreen_corner(int index) {
    vec2 corners[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return corners[index];
}
@end

// How the GTAO depth chain spells "nothing here". Shared by the G-buffer pass that
// writes it, the prefilter that reduces it and the occlusion pass that marches it,
// because all three have to agree or a horizon test finds an occluder in empty space.
//
// Two values mean background, which is why this is a predicate rather than a
// comparison spelled out at each site: a pixel no mesh covered keeps the pass's
// **clear**, which is 0, while an overlay or line fragment writes BACKGROUND_DEPTH
// explicitly (it has to write something, and 0 would read as "on the near plane" —
// an occluder in front of everything).
@block gtao_depth_encoding
const float BACKGROUND_DEPTH = 1e30;

bool is_background_depth(float depth) {
    return !(depth > 0.0 && depth < BACKGROUND_DEPTH);
}
@end

// The scene uniform block, once per stage. Both are the same 272 bytes and are
// filled from the same `SceneUniforms` value; they are two declarations only
// because a sokol uniform block belongs to exactly one stage. The instance names
// (`su` / `sc`) are what lets them share member names in one translation unit.
@block scene_uniforms_vs
layout(binding=0) uniform scene_vs {
    mat4 view_projection;
    mat4 inv_view_projection;
    vec4 render_options;
    // xyz = world-space eye. w > 0.5 enables the GPU deform path.
    vec4 camera_position;
    vec4 env_params;
    vec4 projection_params;
    mat4 view;
    vec4 selection_color;
} su;
@end

@block scene_uniforms_fs
layout(binding=1) uniform scene_fs {
    mat4 view_projection;
    mat4 inv_view_projection;
    // x = shading mode, y = uv-checker on, z = checker tiling, w = vertex-color view
    // (-1 off, else the mode index).
    vec4 render_options;
    vec4 camera_position;
    // x = IBL on, y = intensity, z = show skybox, w = prefilter max mip LOD.
    vec4 env_params;
    // x = orthographic, y = environment yaw (radians), z = buffer-view index (-1
    // off), w = skin-weight heat map active.
    vec4 projection_params;
    mat4 view;
    // rgb = gamma-space highlight, a = flash fade.
    vec4 selection_color;
} sc;
@end

// The varyings `vs_main` produces. Every fragment shader paired with it must
// declare the matching inputs even where it ignores them — the interface is the
// program's, not the shader's — so they are written once here.
@block scene_varyings_out
layout(location=0) out vec3 v_normal;
layout(location=1) out vec2 v_uv;
layout(location=2) out vec4 v_color;
layout(location=3) out vec3 v_world_position;
layout(location=4) out vec4 v_tangent;
@end

@block scene_varyings_in
layout(location=0) in vec3 v_normal;
layout(location=1) in vec2 v_uv;
layout(location=2) in vec4 v_color;
layout(location=3) in vec3 v_world_position;
layout(location=4) in vec4 v_tangent;
@end

// The GPU deform stage: blend shapes, then linear-blend skinning. Every geometry
// pipeline shares `vs_main`, so the mesh, the GTAO G-buffer, the selection flash
// and every mesh-derived overlay deform identically. The structs mirror
// `InfluenceEntry` / `PaletteEntry` / `MorphEntry` in `gpu_types.rs` byte for byte
// (invariant 11).
@block deform
struct InfluenceEntry {
    uint entry;    // palette index
    float weight;
};

// The three rows of an affine 3x4 matrix, spelled as rows rather than a matrix
// type so the storage-buffer packing is unambiguous on both sides.
struct PaletteEntry {
    vec4 r0;
    vec4 r1;
    vec4 r2;
};

// Seven scalars, deliberately: `uint shape; vec3 position; vec3 normal;` would pad
// to 48 bytes under std430 and shift every entry. This stays 28.
struct MorphEntry {
    uint shape;    // index into the weight buffer
    float px;
    float py;
    float pz;
    float nx;
    float ny;
    float nz;
};

// A storage buffer must hold exactly one flexible array of a struct, so the weight
// array is a struct of one float — the same 4 bytes a bare `float[]` would be.
struct MorphWeight {
    float value;
};

layout(binding=12) readonly buffer deform_influences { InfluenceEntry influences[]; };
layout(binding=13) readonly buffer deform_palette { PaletteEntry palette[]; };
layout(binding=14) readonly buffer morph_deltas { MorphEntry morphs[]; };
layout(binding=15) readonly buffer morph_weights { MorphWeight weights[]; };

vec3 palette_point(PaletteEntry m, vec3 p) {
    vec4 h = vec4(p, 1.0);
    return vec3(dot(m.r0, h), dot(m.r1, h), dot(m.r2, h));
}

vec3 palette_direction(PaletteEntry m, vec3 v) {
    return vec3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

// Apply the vertex's blend-shape deltas, then its skinning run — the weighted
// palette blend normalised by the summed weight, exactly as ufbx's own
// `ufbx_get_skin_vertex_matrix` does (a sum within 1e-6 of one is left alone).
// A zero normal (the overlay sentinel) stays zero through both, so overlays keep
// their flat-color path in the fragment shader.
void apply_deform(uvec4 lane, inout vec3 position, inout vec3 normal, inout vec3 tangent) {
    // An overlay vertex derived from a mesh corner (a wireframe edge end) carries
    // the corner's morph lane but a zero normal; the normal delta must not revive
    // it, or the fragment shader would shade the line instead of returning its color.
    bool has_normal = dot(normal, normal) > 1e-12;
    for (uint m = 0u; m < lane.w; m++) {
        MorphEntry delta = morphs[lane.z + m];
        float weight = weights[delta.shape].value;
        position += vec3(delta.px, delta.py, delta.pz) * weight;
        if (has_normal) {
            normal += vec3(delta.nx, delta.ny, delta.nz) * weight;
        }
    }

    if (lane.y == 0u) {
        return;
    }
    vec3 blended_position = vec3(0.0);
    vec3 blended_normal = vec3(0.0);
    vec3 blended_tangent = vec3(0.0);
    float total = 0.0;
    for (uint i = 0u; i < lane.y; i++) {
        InfluenceEntry influence = influences[lane.x + i];
        PaletteEntry entry = palette[influence.entry];
        blended_position += palette_point(entry, position) * influence.weight;
        blended_normal += palette_direction(entry, normal) * influence.weight;
        blended_tangent += palette_direction(entry, tangent) * influence.weight;
        total += influence.weight;
    }
    if (total <= 0.0) {
        return;
    }
    if (abs(total - 1.0) > 1e-6) {
        float rcp_total = 1.0 / total;
        blended_position *= rcp_total;
        blended_normal *= rcp_total;
        blended_tangent *= rcp_total;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}
@end

// ===========================================================================
// Scene — the shared mesh/line vertex shader
// ===========================================================================

@vs vs_main
@include_block scene_uniforms_vs
@include_block deform
@include_block scene_varyings_out

layout(location=0) in vec3 in_position;
layout(location=1) in vec3 in_normal;
layout(location=2) in vec2 in_uv;
layout(location=3) in vec4 in_tangent;
layout(location=4) in vec4 in_color;
// The deform lane: xy = first + count of this vertex's influence run, zw = first +
// count of its blend-shape delta run. All zero for geometry that never deforms.
layout(location=5) in uvec4 in_deform;

void main() {
    vec3 position = in_position;
    vec3 normal = in_normal;
    vec3 tangent = in_tangent.xyz;
    if (su.camera_position.w > 0.5 && (in_deform.y | in_deform.w) != 0u) {
        apply_deform(in_deform, position, normal, tangent);
        // Rigid bones keep the length; a blend of several can shrink it. Renormalise
        // only a real normal — the overlay sentinel must stay exactly zero.
        float normal_length_sq = dot(normal, normal);
        if (normal_length_sq > 1e-12) {
            normal *= inversesqrt(normal_length_sq);
        }
        float tangent_length_sq = dot(tangent, tangent);
        if (tangent_length_sq > 1e-12) {
            tangent *= inversesqrt(tangent_length_sq);
        }
    }
    gl_Position = su.view_projection * vec4(position, 1.0);
    v_normal = normal;
    v_uv = in_uv;
    v_color = in_color;
    v_world_position = position;
    v_tangent = vec4(tangent, in_tangent.w);
}
@end

// ===========================================================================
// Scene — the full per-material shaded / unlit / checker / vertex-color path
// ===========================================================================

@fs fs_main
@include_block scene_uniforms_fs
@include_block scene_varyings_in
@include_block srgb

// Per-material parameters. `params` = (metallic, roughness, slot_flags bitfield,
// alpha mode). `channels0` selects the channel (0 R … 3 A) feeding slots 0..3;
// `channels1` does the same for slots 4..6 with `w` the alpha cutoff. `flags.x` is
// the roughness workflow (0 roughness, 1 smoothness).
layout(binding=2) uniform material {
    vec4 mat_base_color;
    vec4 mat_emissive;
    vec4 mat_params;
    vec4 mat_channels0;
    vec4 mat_channels1;
    vec4 mat_flags;
};

layout(binding=0) uniform texture2D checker_texture;
layout(binding=1) uniform textureCube irradiance_cube;
layout(binding=2) uniform textureCube prefilter_cube;
layout(binding=3) uniform texture2D brdf_lut;
// Slot 4 is the env cube — only the skybox samples it, so it is not declared here.
layout(binding=5) uniform texture2D base_color_tex;
layout(binding=6) uniform texture2D normal_tex;
layout(binding=7) uniform texture2D roughness_tex;
layout(binding=8) uniform texture2D metallic_tex;
layout(binding=9) uniform texture2D ao_tex;
layout(binding=10) uniform texture2D emissive_tex;
layout(binding=11) uniform texture2D opacity_tex;

layout(binding=0) uniform sampler checker_sampler;
layout(binding=1) uniform sampler ibl_sampler;
layout(binding=2) uniform sampler material_sampler;

// MRT: location 0 is the linear-HDR radiance the composite tone-maps; location 1
// is the AO-eligible diffuse ambient radiance GTAO may attenuate. Overlays write a
// zero ambient *color* with the overlay alpha so they never get AO-darkened.
layout(location=0) out vec4 frag_color;
layout(location=1) out vec4 frag_ambient;

// Skin-weight heat map: the unlit-region base color (linear, ~sRGB 0.24 — dark
// enough to stay out of the ramp's way, light enough to read against black) and
// the ambient floor under its headlight, so a face turned away from the camera is
// still visibly *there*.
const vec3 SKIN_WEIGHT_BASE = vec3(0.047, 0.047, 0.052);
const float SKIN_WEIGHT_AMBIENT = 0.28;

// Select one channel of a sampled texel by index (0 R, 1 G, 2 B, 3 A), so a packed
// map can route any channel into a scalar property.
float select_channel(vec4 texel, float index) {
    if (index < 0.5) { return texel.r; }
    if (index < 1.5) { return texel.g; }
    if (index < 2.5) { return texel.b; }
    return texel.a;
}

// Perturb the geometric normal `n` by a tangent-space normal-map sample, using the
// vertex tangent (xyz) + handedness (w) to build the TBN basis. Geometry is
// world-baked, so the tangent is already world-space (no model matrix).
vec3 apply_normal_map(vec3 n, vec4 tangent, vec3 sample_rgb) {
    vec3 geo_n = normalize(n);
    // Gram-Schmidt orthonormalize the tangent against the normal.
    vec3 t = normalize(tangent.xyz - geo_n * dot(geo_n, tangent.xyz));
    if (dot(t, t) < 1e-8) {
        return geo_n;
    }
    vec3 b = cross(geo_n, t) * tangent.w;
    vec3 m = sample_rgb * 2.0 - 1.0;
    return normalize(t * m.x + b * m.y + geo_n * m.z);
}

// Rotate an environment sample direction about the world Y axis by the negative of
// the configured environment yaw, so increasing the yaw spins the whole
// environment. Applied to every cube lookup (irradiance, prefiltered specular,
// skybox) so the IBL and the background rotate together rigidly.
vec3 env_sample_dir(vec3 dir) {
    float angle = -sc.projection_params.y;
    float s = sin(angle);
    float c = cos(angle);
    return vec3(c * dir.x + s * dir.z, dir.y, -s * dir.x + c * dir.z);
}

// Fresnel-Schlick with a roughness term, so rough surfaces don't over-brighten at
// grazing angles (the IBL ambient form).
vec3 fresnel_schlick_roughness(float cos_theta, vec3 f0, float roughness) {
    vec3 inv_rough = vec3(1.0 - roughness);
    return f0 + (max(inv_rough, f0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

struct ShadingResult {
    vec3 color;
    vec3 ambient;
};

// Environment-lit metallic-roughness PBR for the shaded path. Returns linear
// radiance and the diffuse ambient term GTAO may attenuate (tone mapping happens
// in the composite).
ShadingResult shade_ibl(vec3 albedo, vec3 world_normal, vec3 world_pos, float roughness, float metallic) {
    vec3 n = normalize(world_normal);
    vec3 v = normalize(sc.camera_position.xyz - world_pos);
    vec3 r = reflect(-v, n);
    float n_dot_v = max(dot(n, v), 1e-4);
    vec3 f0 = mix(vec3(0.04), albedo, metallic);

    // Diffuse: cosine-convolved irradiance modulated by albedo, yaw-rotated.
    vec3 irradiance = textureLod(samplerCube(irradiance_cube, ibl_sampler), env_sample_dir(n), 0.0).rgb;
    vec3 diffuse = irradiance * albedo;

    // Specular: split-sum — prefiltered env at the roughness mip, scaled by the
    // BRDF LUT (scale + bias). Same yaw rotation applied to the reflection lookup.
    float max_lod = sc.env_params.w;
    vec3 prefiltered = textureLod(samplerCube(prefilter_cube, ibl_sampler), env_sample_dir(r), roughness * max_lod).rgb;
    vec2 brdf = textureLod(sampler2D(brdf_lut, ibl_sampler), vec2(n_dot_v, roughness), 0.0).rg;
    vec3 fresnel = fresnel_schlick_roughness(n_dot_v, f0, roughness);
    vec3 specular = prefiltered * (fresnel * brdf.x + brdf.y);

    vec3 kd = (vec3(1.0) - fresnel) * (1.0 - metallic);
    vec3 ambient = kd * diffuse * sc.env_params.y;

    ShadingResult result;
    result.color = ambient + specular * sc.env_params.y;
    result.ambient = ambient;
    return result;
}

void main() {
    frag_ambient = vec4(0.0);

    float shading_mode = sc.render_options.x;
    bool uv_checker_enabled = sc.render_options.y > 0.5;
    float tiling = max(sc.render_options.z, 1.0);
    // Vertex-color view: -1 off, else mode (0 RGB, 1 Alpha, 2 RGB+A).
    float vertex_color_mode = sc.render_options.w;
    // FBX authors UVs bottom-left (V up); texture sampling is top-left (V down), so
    // flip V once here at the single UV→texel boundary for the checker + every slot.
    vec2 tex_uv = vec2(v_uv.x, 1.0 - v_uv.y);
    vec4 checker = texture(sampler2D(checker_texture, checker_sampler), tex_uv * tiling);
    // Sample every material slot at the top, under uniform control flow — see the
    // derivative rule in the header.
    vec4 tex_base = texture(sampler2D(base_color_tex, material_sampler), tex_uv);
    vec4 tex_normal = texture(sampler2D(normal_tex, material_sampler), tex_uv);
    vec4 tex_roughness = texture(sampler2D(roughness_tex, material_sampler), tex_uv);
    vec4 tex_metallic = texture(sampler2D(metallic_tex, material_sampler), tex_uv);
    vec4 tex_ao = texture(sampler2D(ao_tex, material_sampler), tex_uv);
    vec4 tex_emissive = texture(sampler2D(emissive_tex, material_sampler), tex_uv);
    vec4 tex_opacity = texture(sampler2D(opacity_tex, material_sampler), tex_uv);
    uint slot_flags = uint(mat_params.z);
    float normal_length_sq = dot(v_normal, v_normal);

    // Grid / wireframe / normal lines carry a zero normal — they always render
    // their own vertex color. Their ambient output carries the overlay alpha with
    // zero color so the composite's GTAO darkens only the mesh beneath them.
    if (normal_length_sq < 1e-6) {
        frag_color = vec4(srgb_to_linear(v_color.rgb), v_color.a);
        frag_ambient = vec4(0.0, 0.0, 0.0, v_color.a);
        return;
    }

    // --- Skin-weight heat map -----------------------------------------------
    // A dark matte Lambert base with the influence ramp tinted over it. Painting
    // the ramp flat (the way the buffer views do) would be unreadable here: an
    // uninfluenced region is near-black, so on a dark background the silhouette
    // disappears entirely. Shading a neutral base keeps the form legible while the
    // hue still means the weight.
    //
    // `v_color.rgb` is the ramp color, `.a` the influence fraction, both baked per
    // vertex by `geometry::skin`. Like the buffer views this path runs with the
    // composite in passthrough, so it emits display-space (sRGB) pixels itself.
    if (sc.projection_params.w > 0.5) {
        float weight = clamp(v_color.a, 0.0, 1.0);
        vec3 albedo = mix(SKIN_WEIGHT_BASE, srgb_to_linear(v_color.rgb), weight);
        // A camera headlight rather than a scene light: the model reads from every
        // orbit angle, and the ambient floor keeps grazing faces off pure black.
        vec3 shading_normal = normalize(v_normal);
        vec3 to_eye = normalize(sc.camera_position.xyz - v_world_position);
        float lambert = clamp(dot(shading_normal, to_eye), 0.0, 1.0);
        float shade = SKIN_WEIGHT_AMBIENT + (1.0 - SKIN_WEIGHT_AMBIENT) * lambert;
        frag_color = vec4(linear_to_srgb(albedo * shade), 1.0);
        frag_ambient = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    // Which material slots carry a texture (the slot-flags bitfield in params.z).
    bool has_base = (slot_flags & 1u) != 0u;
    bool has_normal = (slot_flags & 2u) != 0u;
    bool has_roughness = (slot_flags & 4u) != 0u;
    bool has_metallic = (slot_flags & 8u) != 0u;
    bool has_ao = (slot_flags & 16u) != 0u;
    bool has_emissive = (slot_flags & 32u) != 0u;
    bool has_opacity = (slot_flags & 64u) != 0u;

    // Work in linear space. The material base color is stored linear in the uniform;
    // the base-color texture (sRGB format → already linear here) modulates it. The
    // checker sample is already linear (sRGB texture format).
    vec3 base_color = mat_base_color.rgb;
    if (has_base) {
        if (mat_channels0.x > 3.5) {
            base_color = base_color * tex_base.rgb;
        } else {
            base_color = base_color * select_channel(tex_base, mat_channels0.x);
        }
    }
    float out_alpha = mat_base_color.a;
    if (has_opacity) {
        out_alpha = out_alpha * select_channel(tex_opacity, mat_channels1.z);
    }

    // --- Buffer-inspection view: show one shading input flat for data inspection.
    // `projection_params.z` is the buffer index (-1 when off). Bypasses lighting and
    // tone mapping — the composite passes this straight to the backbuffer — so the
    // shown pixel *is* the value (color buffers sRGB-encoded for display, data
    // buffers written raw). Ambient is zeroed (opaque) so GTAO never darkens it.
    // Reads only texels already sampled above plus the uniform, so no sample sits
    // inside a branch.
    float buffer_view = sc.projection_params.z;
    if (buffer_view >= 0.0) {
        frag_ambient = vec4(0.0, 0.0, 0.0, 1.0);
        vec3 result = vec3(0.0);
        if (buffer_view < 0.5) {
            // Base Color: final albedo, sRGB-encoded for display.
            result = linear_to_srgb(base_color);
        } else if (buffer_view < 1.5) {
            // Normal (World): final shading normal with the normal map applied.
            vec3 wn = normalize(v_normal);
            if (has_normal) {
                wn = apply_normal_map(v_normal, v_tangent, tex_normal.rgb);
            }
            result = wn * 0.5 + 0.5;
        } else if (buffer_view < 2.5) {
            // Normal Map (Tangent): the raw authored texels (linear data, direct).
            result = tex_normal.rgb;
        } else if (buffer_view < 3.5) {
            // Geometric Normal: the interpolated vertex normal, no map.
            result = normalize(v_normal) * 0.5 + 0.5;
        } else if (buffer_view < 4.5) {
            // Tangent: the vertex tangent remapped, dimmed where handedness is -1 so
            // a flipped (mirrored-UV) basis reads darker.
            float handed = v_tangent.w < 0.0 ? 0.5 : 1.0;
            result = (normalize(v_tangent.xyz) * 0.5 + 0.5) * handed;
        } else if (buffer_view < 5.5) {
            // Roughness / Smoothness: final scalar (factor combined with the
            // channel-routed map). Under the Smoothness workflow the surface's
            // authored quantity is smoothness, so show 1 - roughness; the Buffers
            // panel relabels this buffer "Smoothness" to match.
            float r = mat_params.y;
            if (has_roughness) {
                float sample_r = select_channel(tex_roughness, mat_channels0.z);
                if (mat_flags.x > 0.5) {
                    r = 1.0 - (1.0 - r) * sample_r;
                } else {
                    r = r * sample_r;
                }
            }
            if (mat_flags.x > 0.5) {
                r = 1.0 - r;
            }
            result = vec3(clamp(r, 0.0, 1.0));
        } else if (buffer_view < 6.5) {
            // Metallic: final scalar.
            float m = mat_params.x;
            if (has_metallic) {
                m = m * select_channel(tex_metallic, mat_channels0.w);
            }
            result = vec3(clamp(m, 0.0, 1.0));
        } else if (buffer_view < 7.5) {
            // Ambient Occlusion: the channel-routed AO map (1 when unbound).
            float ao = 1.0;
            if (has_ao) {
                ao = select_channel(tex_ao, mat_channels1.x);
            }
            result = vec3(clamp(ao, 0.0, 1.0));
        } else if (buffer_view < 8.5) {
            // Emission: final emissive radiance, sRGB-encoded for display.
            vec3 em = mat_emissive.rgb;
            if (has_emissive) {
                vec3 factor = mat_emissive.rgb;
                if (all(lessThanEqual(factor, vec3(0.0)))) { factor = vec3(1.0); }
                if (mat_channels1.y > 3.5) {
                    em = tex_emissive.rgb * factor;
                } else {
                    em = vec3(select_channel(tex_emissive, mat_channels1.y)) * factor;
                }
            }
            result = linear_to_srgb(em);
        } else if (buffer_view < 9.5) {
            // Opacity: final alpha (base-color alpha × opacity map).
            result = vec3(clamp(out_alpha, 0.0, 1.0));
        } else {
            // UV: UV0 as R = U, G = V.
            result = vec3(v_uv.x, v_uv.y, 0.0);
        }
        frag_color = vec4(result, 1.0);
        return;
    }

    if (uv_checker_enabled) {
        base_color = checker.rgb;
    }
    // The vertex-color view replaces the surface material (overriding the checker if
    // both are on). RGB is gamma-space like the material color; alpha is a raw 0..1
    // scalar shown as linear grey.
    if (vertex_color_mode >= 0.0) {
        if (vertex_color_mode < 0.5) {
            base_color = srgb_to_linear(v_color.rgb);
            out_alpha = 1.0;
        } else if (vertex_color_mode < 1.5) {
            base_color = vec3(v_color.a);
            out_alpha = 1.0;
        } else {
            base_color = srgb_to_linear(v_color.rgb);
            out_alpha = v_color.a;
        }
    }

    // Alpha-clip cutout (material flag): drop fully-transparent fragments before any
    // shading. Blend mode keeps the fragment and lets the MRT alpha-blend it.
    float alpha_mode = mat_params.w;
    if (alpha_mode > 1.5 && out_alpha < mat_channels1.w) {
        discard;
    }

    if (shading_mode < 1.5) {
        // Unlit: flat emissive material color.
        frag_color = vec4(base_color, out_alpha);
        return;
    }

    // The shading normal: the geometric normal, optionally perturbed by a
    // tangent-space normal map.
    vec3 world_normal = normalize(v_normal);
    if (has_normal) {
        world_normal = apply_normal_map(v_normal, v_tangent, tex_normal.rgb);
    }
    vec3 n = world_normal;
    // Metallic + roughness from the uniform, modulated by their (channel-routed)
    // textures when bound.
    float metallic = mat_params.x;
    if (has_metallic) {
        metallic = metallic * select_channel(tex_metallic, mat_channels0.w);
    }
    metallic = clamp(metallic, 0.0, 1.0);
    float roughness_value = mat_params.y;
    if (has_roughness) {
        float sample_r = select_channel(tex_roughness, mat_channels0.z);
        if (mat_flags.x > 0.5) {
            // Smoothness workflow: combine in smoothness space (Unity-style),
            // final = slider * map, so a slider of 1 uses the map as authored.
            float smoothness_scalar = 1.0 - roughness_value;
            roughness_value = 1.0 - smoothness_scalar * sample_r;
        } else {
            // Roughness workflow: roughnessFactor * roughness_map (glTF convention).
            roughness_value = roughness_value * sample_r;
        }
    }
    // Ambient-occlusion factor (channel-routed), darkening only the ambient term.
    float ao = 1.0;
    if (has_ao) {
        ao = select_channel(tex_ao, mat_channels1.x);
    }

    vec3 color_linear;
    vec3 ambient_linear;
    if (sc.env_params.x > 0.5) {
        // Image-based lighting (the default Shaded look). Roughness clamped away
        // from a perfect mirror so the lowest mip still reads as a surface.
        float roughness = clamp(roughness_value, 0.04, 1.0);
        ShadingResult shaded = shade_ibl(base_color, world_normal, v_world_position, roughness, metallic);
        color_linear = shaded.color;
        ambient_linear = shaded.ambient;
    } else {
        // Analytic fallback: the neutral-grey hemisphere + Blinn-Phong specular used
        // before IBL. Smoothness is the complement of the (textured) roughness.
        float smoothness = clamp(1.0 - roughness_value, 0.0, 1.0);
        vec3 light_dir = normalize(vec3(0.35, 0.82, 0.44));
        float diffuse = max(dot(n, light_dir), 0.0);
        float hemi_t = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
        vec3 sky = vec3(0.63, 0.63, 0.63);
        vec3 ground = vec3(0.11, 0.11, 0.11);
        vec3 hemi = mix(ground, sky, hemi_t);
        vec3 ambient_light = hemi * 0.55 + vec3(1.0, 1.0, 1.0) * 0.20;
        vec3 direct_light = vec3(1.0, 1.0, 1.0) * (diffuse * 0.75);
        vec3 lighting = ambient_light + direct_light;
        vec3 lit = base_color * lighting;

        vec3 view_dir = normalize(sc.camera_position.xyz - v_world_position);
        vec3 half_dir = normalize(light_dir + view_dir);
        float shininess = exp2(1.0 + smoothness * 10.0);
        float spec = pow(max(dot(n, half_dir), 0.0), shininess) * smoothness * step(0.0, diffuse);
        color_linear = lit + vec3(spec);
        ambient_linear = base_color * ambient_light;
    }

    // Ambient occlusion darkens only the ambient term: remove the un-occluded
    // ambient from the lit color and add back the occluded fraction, leaving direct
    // + specular light untouched (matching the composite's GTAO compose).
    color_linear = color_linear + ambient_linear * (ao - 1.0);
    ambient_linear = ambient_linear * ao;

    // Emissive adds on top of the lit result (linear). Default emissive is black; an
    // emissive texture (when bound) modulates it.
    vec3 emissive = mat_emissive.rgb;
    if (has_emissive) {
        vec3 factor = mat_emissive.rgb;
        if (all(lessThanEqual(factor, vec3(0.0)))) {
            factor = vec3(1.0);
        }
        if (mat_channels1.y > 3.5) {
            emissive = tex_emissive.rgb * factor;
        } else {
            emissive = vec3(select_channel(tex_emissive, mat_channels1.y)) * factor;
        }
    }
    color_linear = color_linear + emissive;

    frag_color = vec4(color_linear, out_alpha);
    frag_ambient = vec4(ambient_linear, out_alpha);
}
@end

// ===========================================================================
// Scene — the selection flash
// ===========================================================================

// A flat color fill over the selected triangles (the solo index buffer redrawn
// over the mesh). Ignores the mesh's lighting / material — it emits the uniform
// highlight color tinted by the flash fade. Writes zero ambient *color* with the
// fade alpha so it masks (rather than GTAO-darkens) the mesh ambient beneath, like
// the other overlays.
@fs fs_selection
@include_block scene_uniforms_fs
@include_block scene_varyings_in
@include_block srgb

layout(location=0) out vec4 frag_color;
layout(location=1) out vec4 frag_ambient;

void main() {
    float fade = sc.selection_color.a;
    frag_color = vec4(srgb_to_linear(sc.selection_color.rgb), fade);
    frag_ambient = vec4(0.0, 0.0, 0.0, fade);
}
@end

// ===========================================================================
// Scene — line overlays, as screen-space quads
// ===========================================================================

// Every line the viewer draws — the model wireframe, the grid, the bounding box, the
// normal lines, the seams, the pivot, the skeleton's outlines and the UV view's lines —
// is widened into a quad of a chosen width in pixels, with its coverage faded out
// analytically across the last half pixel on each side. Hardware lines are one
// *device* pixel wide whatever the display, and each API rasterizes them by its own
// rules — Direct3D 11 draws them ~1.4 px wide under MSAA, Metal 1 px — so the same
// wireframe looked twice as heavy on one OS as the other, and half as heavy again on a
// Retina screen. A quad is the same shape on every backend at every density.
//
// Because the edges smooth themselves, lines need no MSAA. With it on, the renderer
// draws them in a single-sample pass over the *resolved* scene instead of in the
// multisampled one (`SceneGpu::record_line_pass`): a dense wireframe covers a great
// many pixels, and inside the MSAA pass every one of them was a blend per sample.
//
// There is no vertex input. A draw is six vertices per line, and the shader *pulls*
// what it needs from the line's vertex buffer, bound as a storage buffer: the line's
// two ends, both deformed through the same `apply_deform` as the mesh under them. The
// two programs differ only in where a line's ends and colour come from:
//
//   line  a line list: ends `2i` and `2i + 1`, the colour carried by the vertices
//   wire  the model wireframe: ends named by an edge list over the *mesh's own*
//         vertex buffer, so the geometry is read rather than copied (invariant 1),
//         and the colour from `selection_color` — which is what lets the Opt ghost
//         reuse it with its own tint

// The line programs' own block: what widening a line needs that the scene block lacks.
@block line_uniforms
layout(binding=3) uniform line_params {
    // x / y = the scene target's size in pixels, z = the line's width in pixels,
    // w spare.
    vec4 params;
} lu;
@end

// A mesh vertex pulled from a storage view of a vertex buffer, and its position
// deformed exactly as `vs_main` deforms it: what every program that takes no vertex
// input reads its geometry through (the lines, the wireframe, the overdraw views).
// Needs `scene_uniforms_vs` and `deform` included before it.
@block pulled_vertex
// `SceneVertex` as a storage buffer sees it: twenty scalars, deliberately — a `vec3`
// member would align to 16 bytes under std430 and stride the buffer at 96 rather than
// the vertex buffer's 80. `gpu_types.rs` pins this to `SceneVertex`.
struct LineVertex {
    float px;
    float py;
    float pz;
    float nx;
    float ny;
    float nz;
    float u;
    float v;
    float tx;
    float ty;
    float tz;
    float tw;
    float r;
    float g;
    float b;
    float a;
    uint d0;
    uint d1;
    uint d2;
    uint d3;
};

layout(binding=16) readonly buffer line_vertices { LineVertex line_vertex[]; };

// A pulled vertex's position, deformed exactly as `vs_main` deforms it.
vec3 line_corner_position(uint vertex) {
    LineVertex corner = line_vertex[vertex];
    vec3 position = vec3(corner.px, corner.py, corner.pz);
    uvec4 lane = uvec4(corner.d0, corner.d1, corner.d2, corner.d3);
    if (su.camera_position.w > 0.5 && (lane.y | lane.w) != 0u) {
        // Only the position is wanted. A zero normal is the overlay sentinel, so the
        // blend-shape normal deltas are skipped rather than revived.
        vec3 normal = vec3(0.0);
        vec3 tangent = vec3(0.0);
        apply_deform(lane, position, normal, tangent);
    }
    return position;
}

vec4 line_corner_color(uint vertex) {
    LineVertex corner = line_vertex[vertex];
    return vec4(corner.r, corner.g, corner.b, corner.a);
}

@end

// What both line vertex shaders share: the quad built from a line's two clip-space
// ends. Needs `scene_uniforms_vs`, `deform`, `pulled_vertex` and `line_uniforms`
// included before it.
@block line_quad
// Across the line, along it, its on-screen length, and its half width — all in
// pixels, and all interpolated in screen space: the quad is a rectangle on screen, so
// a perspective-correct interpolation would bend the coverage ramp.
layout(location=0) noperspective out vec4 v_line;
// Gamma-space rgb + alpha, as the vertices and `selection_color` both carry it.
layout(location=1) out vec4 v_color;

// Corner `corner_index` (0..5) of a line's quad, as two triangles: x picks the line's
// end (0 = first, 1 = second), y its side. Unsigned, like the arithmetic that finds
// it: fxc warns on a signed integer divide, and `/WX` makes that a build error.
vec2 line_quad_corner(uint corner_index) {
    vec2 corners[6] = vec2[6](
        vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    return corners[corner_index];
}

// Write `gl_Position` and `v_line` for corner `corner_index` of the quad over the line
// from `first` to `second`, both in clip space.
void emit_line_quad(vec4 first, vec4 second, uint corner_index) {
    vec2 corner = line_quad_corner(corner_index);

    // Clip the line to the near plane before anything divides by w: an end behind
    // the eye would otherwise land on the far side of the screen and drag the quad
    // across it. Reversed-Z puts the near plane at z = w (perspective and
    // orthographic alike), with everything in front of it at z < w.
    float first_in = first.w - first.z;
    float second_in = second.w - second.z;
    if (first_in < 0.0 && second_in < 0.0) {
        // Wholly behind the near plane: a degenerate quad outside the clip volume.
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        v_line = vec4(0.0);
        return;
    }
    if (first_in < 0.0) {
        first = mix(first, second, first_in / (first_in - second_in));
    } else if (second_in < 0.0) {
        second = mix(second, first, second_in / (second_in - first_in));
    }

    vec2 half_size = 0.5 * lu.params.xy;
    vec2 first_px = first.xy / first.w * half_size;
    vec2 second_px = second.xy / second.w * half_size;
    vec2 along = second_px - first_px;
    float length_px = length(along);
    // A line seen end-on still gets a square of its own width, in any orientation.
    vec2 direction = length_px > 1e-4 ? along / length_px : vec2(1.0, 0.0);
    vec2 across = vec2(-direction.y, direction.x);

    // The quad reaches half a pixel past the line on every side, which is where the
    // fragment shader's coverage ramp reaches zero: a pixel whose centre lies any
    // further out would be shaded only to be discarded by its alpha. A line under a
    // pixel wide is drawn one pixel wide and faded instead (see `fs_line`), so the
    // quad never shrinks below that.
    float half_width = 0.5 * lu.params.z;
    float reach = max(half_width, 0.5) + 0.5;
    float end_sign = corner.x * 2.0 - 1.0;
    vec4 position = corner.x < 0.5 ? first : second;
    vec2 offset_px = across * (corner.y * reach) + direction * (end_sign * reach);
    position.xy += offset_px / half_size * position.w;
    gl_Position = position;
    v_line = vec4(
        corner.y * reach,
        corner.x < 0.5 ? -reach : length_px + reach,
        length_px,
        half_width);
}
@end

// A line list: line `i` runs from vertex `2i` to vertex `2i + 1`.
@vs vs_line
@include_block scene_uniforms_vs
@include_block deform
@include_block pulled_vertex
@include_block line_uniforms
@include_block line_quad

void main() {
    uint vertex = uint(gl_VertexIndex);
    uint line = vertex / 6u;
    uint first_vertex = 2u * line;
    vec4 first = su.view_projection * vec4(line_corner_position(first_vertex), 1.0);
    vec4 second = su.view_projection * vec4(line_corner_position(first_vertex + 1u), 1.0);
    uint corner_index = vertex - line * 6u;
    emit_line_quad(first, second, corner_index);
    v_color = line_corner_color(first_vertex + uint(line_quad_corner(corner_index).x));
}
@end

// The model wireframe: edge `e` runs between the mesh vertices named by entries `2e`
// and `2e + 1` of the edge list.
@vs vs_wire
@include_block scene_uniforms_vs
@include_block deform
@include_block pulled_vertex
@include_block line_uniforms
@include_block line_quad

// One corner index of the edge list. A struct of one `uint` because a storage buffer
// must hold an array of a struct.
struct LineIndex {
    uint index;
};

layout(binding=17) readonly buffer line_indices { LineIndex line_index[]; };

void main() {
    uint vertex = uint(gl_VertexIndex);
    uint edge = vertex / 6u;
    vec4 first = su.view_projection
        * vec4(line_corner_position(line_index[2u * edge].index), 1.0);
    vec4 second = su.view_projection
        * vec4(line_corner_position(line_index[2u * edge + 1u].index), 1.0);
    emit_line_quad(first, second, vertex - edge * 6u);
    v_color = su.selection_color;
}
@end

// The line's coverage of this pixel: a one-pixel box filter of a band `width` pixels
// wide, with a square cap of half that width past each end so lines meeting at a
// vertex join without a notch. Writes zero ambient *colour* with the line's alpha, so
// it masks (rather than GTAO-darkens) the ambient beneath, like the other overlays.
@fs fs_line
@include_block srgb

layout(location=0) noperspective in vec4 v_line;
layout(location=1) in vec4 v_color;

layout(location=0) out vec4 frag_color;
layout(location=1) out vec4 frag_ambient;

void main() {
    float half_width = max(v_line.w, 0.5);
    // Narrower than a pixel: kept one pixel wide and faded by the shortfall, so a
    // thin line dims evenly rather than breaking into dashes.
    float thinness = min(v_line.w * 2.0, 1.0);
    float across = abs(v_line.x);
    float past_end = max(-v_line.y, v_line.y - v_line.z);
    float coverage = clamp(half_width + 0.5 - across, 0.0, 1.0)
        * clamp(half_width + 0.5 - past_end, 0.0, 1.0)
        * thinness;
    float alpha = v_color.a * coverage;
    frag_color = vec4(srgb_to_linear(v_color.rgb), alpha);
    frag_ambient = vec4(0.0, 0.0, 0.0, alpha);
}
@end

// ===========================================================================
// Scene — the GTAO G-buffer
// ===========================================================================

// A mesh-only pass writing the view-space normal (xyz) + the linear view-space Z
// (w) into a single-sample target the GTAO occlusion pass reads. Single-sample (no
// MSAA) so geometry-edge normals/depths aren't averaged by a resolve before the
// occlusion + denoiser read them.
//
// Location 1 is the same depth again as a *positive* distance in its own
// single-channel target, which is what the prefilter chain reduces. It is written
// here rather than copied out afterwards because this pass already has it, and a
// second fullscreen pass to move it would cost a full-resolution read and write for
// nothing.
@fs fs_gtao_gbuffer
@include_block scene_uniforms_fs
@include_block scene_varyings_in
@include_block gtao_depth_encoding

layout(location=0) out vec4 frag_gbuffer;
layout(location=1) out float frag_depth;

void main() {
    float normal_length_sq = dot(v_normal, v_normal);
    // Overlays / lines (zero normal) write a zero G-buffer entry → GTAO treats them
    // as background and applies no occlusion there.
    if (normal_length_sq < 1e-6) {
        frag_gbuffer = vec4(0.0);
        frag_depth = BACKGROUND_DEPTH;
        return;
    }
    // View Z is negative in front of the camera; GTAO treats zero as background.
    vec4 view_pos = sc.view * vec4(v_world_position, 1.0);
    vec3 view_normal = normalize((sc.view * vec4(v_normal, 0.0)).xyz);
    frag_gbuffer = vec4(view_normal, view_pos.z);
    frag_depth = -view_pos.z;
}
@end

// ===========================================================================
// Scene — the skybox background
// ===========================================================================

// A fullscreen triangle whose per-pixel world ray direction is reconstructed from
// the inverse view-projection, sampled into the env cube. Drawn first in the
// offscreen pass (depth test always, no depth write) so geometry composites over it.
@vs vs_skybox
@include_block fullscreen

layout(location=0) out vec2 v_ndc;

void main() {
    vec2 xy = fullscreen_corner(gl_VertexIndex);
    // Reversed-Z far depth is 0; depth write is off and the pipeline always passes,
    // so this is only for completeness.
    gl_Position = vec4(xy, 0.0, 1.0);
    v_ndc = xy;
}
@end

@fs fs_skybox
@include_block scene_uniforms_fs

layout(location=0) in vec2 v_ndc;

layout(binding=4) uniform textureCube env_cube;
layout(binding=1) uniform sampler ibl_sampler;

layout(location=0) out vec4 frag_color;
layout(location=1) out vec4 frag_ambient;

void main() {
    // Perspective uses an infinite reversed projection, so unproject a finite
    // near-plane point (reversed-Z near depth = 1). Orthographic keeps a finite far
    // plane and unprojects at depth 0 to preserve the viewport-varying direction.
    float unproject_depth = (sc.projection_params.x > 0.5) ? 0.0 : 1.0;
    vec4 world = sc.inv_view_projection * vec4(v_ndc, unproject_depth, 1.0);
    vec3 world_pos = world.xyz / world.w;
    vec3 dir = normalize(world_pos - sc.camera_position.xyz);
    // Yaw-rotate the skybox lookup so the background spins with the IBL. Duplicated
    // rather than shared with `fs_main`'s copy because this shader declares neither
    // the material block nor the IBL maps it would otherwise pull in.
    float angle = -sc.projection_params.y;
    float s = sin(angle);
    float c = cos(angle);
    vec3 env_dir = vec3(c * dir.x + s * dir.z, dir.y, -s * dir.x + c * dir.z);
    // Clamp the intensity-scaled sample to f16 max so a bright sun saturates to
    // white rather than overflowing to inf/NaN in the HDR target.
    vec3 raw = textureLod(samplerCube(env_cube, ibl_sampler), env_dir, 0.0).rgb * sc.env_params.y;
    frag_color = vec4(min(raw, vec3(65504.0)), 1.0);
    // The sky is background: no ambient target, so GTAO never darkens it.
    frag_ambient = vec4(0.0);
}
@end

// ===========================================================================
// Fullscreen vertex shaders
// ===========================================================================

// The composite and both GTAO passes: a fullscreen triangle carrying the target uv.
@vs vs_fullscreen
@include_block fullscreen

layout(location=0) out vec2 v_uv;

void main() {
    vec2 corner = fullscreen_corner(gl_VertexIndex);
    gl_Position = vec4(corner, 0.0, 1.0);
    vec2 uv = corner * 0.5 + vec2(0.5);
    uv.y = 1.0 - uv.y; // texture origin is top-left
    v_uv = uv;
}
@end

// The Tex viewport: the same triangle with no varyings — both its fragment paths
// work from `gl_FragCoord` (framebuffer pixels), not a normalized uv.
@vs vs_fullscreen_bare
@include_block fullscreen

void main() {
    gl_Position = vec4(fullscreen_corner(gl_VertexIndex), 0.0, 1.0);
}
@end

// ===========================================================================
// GTAO
// ===========================================================================

// Ground-truth ambient occlusion, in view space: the G-buffer stores the view-space
// normal (xyz) and the linear view-space Z (w, negative in front of the camera).
// For each slice direction the shader marches the screen-space horizon both ways,
// then integrates the cosine-weighted visible arc, averaging over slices.
@block gtao_common
@include_block gtao_depth_encoding
layout(binding=0) uniform gtao_params {
    // View → clip projection: projects sample points to screen, and its terms
    // reconstruct view-space position from depth.
    mat4 proj;
    // x = sample radius (view units), y = intensity (power on visibility),
    // z = thin-occluder compensation (0..1), w = target width in pixels.
    vec4 params;
    // x = 1.0 when the projection is orthographic, y = slice count,
    // z = steps per slice, w = target height in pixels.
    vec4 config;
    // The temporal state of the accumulation (`scene/ao_accum.rs`). x = this
    // frame's extra slice rotation and y = its extra step offset, both in [0,1);
    // together they walk the 6×4 Jimenez pattern set so 24 accumulated frames cover
    // every rotation/offset exactly once. z = the weight this frame's estimate gets
    // when blended into the history — 1/(n+1) for a running mean, and exactly 1.0
    // on the frame a reset happened, which is what tells the occlusion shader not to
    // read the history at all. w is spare.
    vec4 temporal;
};

layout(binding=0) uniform texture2D gbuffer;
layout(binding=0) uniform sampler gtao_sampler;

const float PI = 3.14159265359;
const float HALF_PI = 1.57079632679;

// The G-buffer / AO target size in pixels. sokol has no `GetDimensions`, so the two
// otherwise-unused `w` slots of the params/config rows carry it.
vec2 target_dims() {
    return vec2(max(params.w, 1.0), max(config.w, 1.0));
}

// Reconstruct view-space position from a pixel's uv + its stored view-space Z.
// Matrices are column-major, so an element that HLSL read as `proj[row][col]` is
// `proj[col][row]` here.
vec3 reconstruct_view_pos(vec2 uv, float view_z) {
    vec2 ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    if (config.x > 0.5) {
        // Orthographic: ndc = P00*x + P03 (x = (ndc - P03) / P00); Z is independent.
        float x = (ndc.x - proj[3][0]) / proj[0][0];
        float y = (ndc.y - proj[3][1]) / proj[1][1];
        return vec3(x, y, view_z);
    }
    // Perspective: ndc.x = P00 * x / (-view_z)  =>  x = ndc.x * (-view_z) / P00.
    float x = ndc.x * (-view_z) / proj[0][0];
    float y = ndc.y * (-view_z) / proj[1][1];
    return vec3(x, y, view_z);
}

// Project a view-space point to texture uv via the projection matrix.
vec2 project_to_uv(vec3 view_pos) {
    vec4 clip = proj * vec4(view_pos, 1.0);
    vec2 ndc = clip.xy;
    if (config.x <= 0.5) {
        ndc = clip.xy / clip.w;
    }
    return vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// The size in view units of one pixel at `view_z`, which is what turns a screen-space
// tolerance into a world-space one. Used by the denoiser so its edge stopping is the
// same strictness on a close-up and on a distant object, rather than scaling with the
// scene or the AO radius the way the old blur's depth sigma did.
float view_pixel_size(float view_z) {
    float height = max(config.w, 1.0);
    // Perspective: the view-space height of the frustum at this depth is
    // 2|z| / P11, spread over `height` pixels. Orthographic: the same without the
    // depth term, since the frustum is a box.
    float extent = (config.x > 0.5) ? 2.0 : 2.0 * abs(view_z);
    return extent / (max(abs(proj[1][1]), 1e-6) * height);
}
@end

// The horizon search: a GLSL translation of `XeGTAO_MainPass` from Intel's XeGTAO
// (MIT, Copyright (C) 2016-2021, Intel Corporation - see
// docs/THIRD-PARTY-NOTICES.md), without its bent normals. Three things distinguish it
// from a textbook GTAO march, and each is one of the reasons the old pass was noisy:
//
//   * **Steps are distributed as the square of the fraction**, so the first tap sits
//     ~1.3 px away and the last at the full radius. Linear spacing over a radius that
//     is metres wide on a room interior put *every* tap tens of pixels out: contact
//     occlusion was missed entirely, and the four step offsets of the dither block
//     each found a different set of thin occluders, which is what drew the streaks.
//   * **Far taps read a prefiltered depth level** instead of full-resolution depth,
//     so a tap that stands for a 16-px neighbourhood is compared against that whole
//     neighbourhood's depth rather than against one aliased texel of a chair leg.
//   * **The horizon falls off smoothly** toward the slice's low horizon over the
//     outer 61.5 % of the radius, rather than an occluder inside the radius counting
//     fully and one just outside not at all.
@fs fs_gtao
@include_block gtao_common

layout(binding=1) uniform texture2D depth_mip0;
layout(binding=2) uniform texture2D depth_mip1;
layout(binding=3) uniform texture2D depth_mip2;
layout(binding=4) uniform texture2D depth_mip3;
layout(binding=5) uniform texture2D depth_mip4;
layout(binding=6) uniform texture2D ao_history;

layout(location=0) in vec2 v_uv;
layout(location=0) out float frag_ao;

// Below this the sample is the centre pixel again and carries no information.
const float PIXEL_TOO_CLOSE = 1.3;
// XeGTAO's depth-MIP sampling offset: `log2(offset_px) - 3.3` picks the level, so a
// tap ~10 px out reads level 0 and one ~160 px out reads level 4.
const float DEPTH_MIP_OFFSET = 3.3;
const float DEPTH_MIP_MAX = 4.0;
// The outer fraction of the radius over which an occluder fades out.
const float FALLOFF_RANGE_FRACTION = 0.615;
// Golden ratio, for decorrelating each (slice, step) pair's offset from the others.
const float GOLDEN = 0.6180339887498949;

// Jimenez 2016 (Activision GTAO) structured 4x4 spatial dither. Returns the slice
// rotation in x and the step offset in y, both in [0,1). Adjacent pixels get evenly
// spread rotations/offsets, so the denoiser averages complementary slice directions
// over a small block and resolves to a clean result at low sample counts instead of
// leaving high-variance speckle.
vec2 spatial_dither(uvec2 pix) {
    float rot = (1.0 / 16.0) * float((((pix.x + pix.y) & 3u) << 2u) + (pix.x & 3u));
    float offset = (1.0 / 4.0) * float((pix.y - pix.x) & 3u);
    return vec2(rot, offset);
}

// One tap's depth, from the prefilter level that matches how far out it is.
//
// Five separate bindings selected by an integer compare, rather than one image with
// five mips: sokol refuses to bind an image as a texture in the pass that attaches it
// (`VALIDATE_ABND_TEXTURE_BINDING_VS_COLOR_ATTACHMENT`), so the chain cannot live in
// one image that each prefilter pass writes a level of. It also keeps this portable —
// a dynamically indexed texture array is the construct MSL is least happy with.
// One assigned variable rather than a return per branch: fxc's flow analysis does not
// see that a chain of early returns covers every path and rejects the function under
// `/WX` as "potentially uninitialized".
float sample_depth_mip(vec2 uv, int level) {
    float depth = 0.0;
    if (level <= 0) {
        depth = textureLod(sampler2D(depth_mip0, gtao_sampler), uv, 0.0).r;
    } else if (level == 1) {
        depth = textureLod(sampler2D(depth_mip1, gtao_sampler), uv, 0.0).r;
    } else if (level == 2) {
        depth = textureLod(sampler2D(depth_mip2, gtao_sampler), uv, 0.0).r;
    } else if (level == 3) {
        depth = textureLod(sampler2D(depth_mip3, gtao_sampler), uv, 0.0).r;
    } else {
        depth = textureLod(sampler2D(depth_mip4, gtao_sampler), uv, 0.0).r;
    }
    return depth;
}

void main() {
    vec4 g = textureLod(sampler2D(gbuffer, gtao_sampler), v_uv, 0.0);
    vec3 raw_normal = g.xyz;
    float view_z = g.w;
    // Background (overlays / skybox / cleared frame write 0): no occlusion, and no
    // history blend either — an unoccluded pixel is unoccluded on every frame.
    if (dot(raw_normal, raw_normal) < 0.25 || view_z >= -1e-4) {
        frag_ao = 1.0;
        return;
    }

    vec3 n = normalize(raw_normal);
    vec3 p = reconstruct_view_pos(v_uv, view_z);
    vec3 v = normalize(-p);

    float radius = max(params.x, 1e-4);
    float intensity = params.y;
    float thin_occluder = clamp(params.z, 0.0, 1.0);
    uint slice_count = max(uint(config.y), 1u);
    uint step_count = max(uint(config.z), 1u);

    vec2 dims = target_dims();
    vec2 inv_dims = 1.0 / dims;
    // Screen radius (px): the UV span of `radius` view units along view X at this
    // depth. Derived by projecting rather than from a depth ratio so the
    // orthographic camera needs no separate case.
    vec2 edge_uv = project_to_uv(p + vec3(radius, 0.0, 0.0));
    float radius_px = abs(edge_uv.x - v_uv.x) * dims.x;

    // Under a pixel of screen radius there is nothing to march: every tap would land
    // back on the centre. The old code clamped to 1 px and marched anyway, which
    // spent the full loop to produce noise.
    if (radius_px < PIXEL_TOO_CLOSE) {
        frag_ao = 1.0;
        return;
    }
    float min_s = PIXEL_TOO_CLOSE / radius_px;

    // Smooth radius falloff (XeGTAO): an occluder is counted in full up to
    // `falloff_from` and fades to the slice's low horizon by `radius`.
    float falloff_range = FALLOFF_RANGE_FRACTION * radius;
    float falloff_from = radius * (1.0 - FALLOFF_RANGE_FRACTION);
    float falloff_mul = -1.0 / falloff_range;
    float falloff_add = falloff_from / falloff_range + 1.0;

    // The 4x4 spatial pattern, rotated and offset by this frame's place in the
    // temporal sequence. While the view moves the sequence sits at entry 0, so a
    // moving frame carries the plain spatial pattern and nothing shimmers; while it
    // is still, 24 frames walk every rotation/offset and average out.
    vec2 dither = spatial_dither(uvec2(uint(gl_FragCoord.x), uint(gl_FragCoord.y)));
    float noise_slice = fract(dither.x + temporal.x);
    float noise_sample = fract(dither.y + temporal.y);

    // A tiny lift at small screen radii, so AO fades out with distance rather than
    // switching off at the early-out above.
    float visibility = clamp((10.0 - radius_px) / 100.0, 0.0, 1.0) * 0.5;

    for (uint s = 0u; s < slice_count; s++) {
        float phi = (float(s) + noise_slice) * PI / float(slice_count);
        float cos_phi = cos(phi);
        float sin_phi = sin(phi);
        // Screen y points down while view y points up, so the screen direction takes
        // the opposite sine of the view-space one it stands for.
        vec2 omega_screen = vec2(cos_phi, -sin_phi);
        vec3 direction = vec3(cos_phi, sin_phi, 0.0);

        // The slice plane: spanned by the view vector and the part of `direction`
        // perpendicular to it. Built from the slice angle alone — the old code read a
        // G-buffer neighbour 2 px away to get it, which at a silhouette is a different
        // surface (or background) and flipped the plane between adjacent pixels.
        vec3 ortho_direction = direction - dot(direction, v) * v;
        // `normalize` of a zero vector is NaN, and one NaN here reaches the AO
        // buffer — where the denoise kernel spreads it and the history blend makes
        // it permanent. The cross product vanishes only when the slice direction is
        // parallel to the view vector, which needs a point ~90° off axis, but the
        // cost of the guard is one compare and the cost of being wrong is a viewport
        // that goes black and stays black.
        vec3 axis_raw = cross(ortho_direction, v);
        float axis_len_sq = dot(axis_raw, axis_raw);
        if (axis_len_sq < 1e-12) {
            continue;
        }
        vec3 axis = axis_raw * inversesqrt(axis_len_sq);
        vec3 proj_n = n - axis * dot(n, axis);
        float proj_len = length(proj_n);
        if (proj_len < 1e-6) {
            continue;
        }
        float sign_n = sign(dot(proj_n, ortho_direction));
        float cos_n = clamp(dot(proj_n, v) / proj_len, 0.0, 1.0);
        float angle_n = sign_n * acos(cos_n);

        // The horizons start at the edges of the hemisphere around the normal, so a
        // slice with no occluder integrates to full visibility.
        float low_horizon_cos0 = cos(angle_n + HALF_PI);
        float low_horizon_cos1 = cos(angle_n - HALF_PI);
        float horizon_cos0 = low_horizon_cos0;
        float horizon_cos1 = low_horizon_cos1;

        for (uint t = 0u; t < step_count; t++) {
            // Decorrelate each (slice, step) from every other, so the step offsets
            // don't line up into a pattern across the dither block.
            float step_noise = fract(noise_sample + float(s + t * slice_count) * GOLDEN);
            float f = (float(t) + step_noise) / float(step_count);
            // Squared: dense near the pixel, sparse out at the radius.
            f = f * f + min_s;

            vec2 offset = f * omega_screen * radius_px;
            float offset_px = length(offset);
            int level = int(clamp(log2(max(offset_px, 1.0)) - DEPTH_MIP_OFFSET, 0.0, DEPTH_MIP_MAX) + 0.5);
            // Snap to whole pixels so a tap reads one texel rather than straddling two.
            vec2 offset_uv = round(offset) * inv_dims;

            vec2 uv0 = v_uv + offset_uv;
            vec2 uv1 = v_uv - offset_uv;
            float depth0 = sample_depth_mip(uv0, level);
            float depth1 = sample_depth_mip(uv1, level);

            float shc0 = low_horizon_cos0;
            if (!is_background_depth(depth0)) {
                vec3 delta = reconstruct_view_pos(uv0, -depth0) - p;
                float dist = length(delta);
                if (dist > 1e-6) {
                    float w = clamp(dist * falloff_mul + falloff_add, 0.0, 1.0);
                    shc0 = mix(low_horizon_cos0, dot(delta, v) / dist, w);
                }
            }
            float shc1 = low_horizon_cos1;
            if (!is_background_depth(depth1)) {
                vec3 delta = reconstruct_view_pos(uv1, -depth1) - p;
                float dist = length(delta);
                if (dist > 1e-6) {
                    float w = clamp(dist * falloff_mul + falloff_add, 0.0, 1.0);
                    shc1 = mix(low_horizon_cos1, dot(delta, v) / dist, w);
                }
            }

            // Thin-occluder compensation (the `Thickness` knob). A horizon that only
            // ever climbs assumes every occluder is infinitely deep, so a railing or a
            // chair leg shadows everything behind it. At 1 the horizon follows the
            // last sample back down; at 0 occluders are solid.
            float raised0 = max(horizon_cos0, shc0);
            horizon_cos0 = (horizon_cos0 > shc0) ? mix(raised0, shc0, thin_occluder) : raised0;
            float raised1 = max(horizon_cos1, shc1);
            horizon_cos1 = (horizon_cos1 > shc1) ? mix(raised1, shc1, thin_occluder) : raised1;
        }

        // A normal nearly edge-on to the slice contributes almost nothing, which
        // makes its estimate pure noise; nudging the weight toward 1 trades a little
        // bias for a lot of variance.
        proj_len = mix(proj_len, 1.0, 0.05);

        float h0 = -acos(clamp(horizon_cos1, -1.0, 1.0));
        float h1 = acos(clamp(horizon_cos0, -1.0, 1.0));
        float sin_n = sin(angle_n);
        float arc0 = 0.25 * (cos_n + 2.0 * h0 * sin_n - cos(2.0 * h0 - angle_n));
        float arc1 = 0.25 * (cos_n + 2.0 * h1 * sin_n - cos(2.0 * h1 - angle_n));
        visibility += proj_len * (arc0 + arc1);
    }

    visibility = clamp(visibility / float(slice_count), 0.0, 1.0);
    // Intensity sharpens the falloff (1 = ground truth, >1 darkens, 0 disables).
    //
    // The floor on the base is not cosmetic. HLSL computes `pow(x, y)` as
    // `exp2(y * log2(x))`, so `pow(0.0, 0.0)` is `0 * -inf` — NaN. Visibility is
    // exactly 0 wherever every slice was skipped, and Intensity 0 is a documented
    // setting the slider bottoms out at, so that pair is reachable by dragging one
    // control to its end. The NaN would then spread through the denoise kernel and,
    // because the history blend folds it back in every frame, never wash out: the
    // viewport goes black and stays black. `1e-6` changes no other result — at any
    // positive intensity it still darkens to ~0, and at intensity 0 it correctly
    // gives 1.
    float ao = pow(max(visibility, 1e-6), max(intensity, 0.0));

    // Blend into the running mean. The branch is load-bearing rather than tidiness:
    // a freshly created R16F target holds whatever was in that memory, and a `mix`
    // with weight 1 over a NaN is still NaN — so the reset frame must not read the
    // history at all.
    if (temporal.z >= 1.0) {
        frag_ao = ao;
    } else {
        float history = textureLod(sampler2D(ao_history, gtao_sampler), v_uv, 0.0).r;
        // A visibility term outside [0,1] is not one, and NaN fails both compares.
        // Falling back to this frame's estimate is what lets the buffer heal: an
        // accumulating history has no other way to drop a bad value, since every
        // later frame only ever blends *with* it.
        bool usable = history >= 0.0 && history <= 1.0;
        frag_ao = usable ? mix(history, ao, temporal.z) : ao;
    }
}
@end

// One level of the GTAO depth prefilter chain: a 2x2 reduction of the level above.
//
// Not a box filter. A plain average across a silhouette invents a depth halfway
// between the foreground and the background, and the occlusion pass then finds an
// occluder floating in empty space — the speckle that used to ring thin geometry.
// This is XeGTAO's weighted filter (`XeGTAO_DepthMIPFilter`, MIT, Copyright (C)
// 2016-2021, Intel Corporation), biased toward the *farthest* of the four, so a
// mixed neighbourhood resolves to the background it mostly is.
@fs fs_gtao_depth_mip
@include_block gtao_depth_encoding

layout(binding=0) uniform gtao_mip_params {
    // x = spare, y / z = the source level's size in pixels, w = the AO radius in
    // view units (the filter's falloff is scaled to it, as the occlusion pass's is).
    vec4 mip;
};

layout(binding=0) uniform texture2D depth_src;
layout(binding=0) uniform sampler gtao_sampler;

layout(location=0) out float frag_depth;

void main() {
    vec2 src_dims = max(mip.yz, vec2(1.0));
    vec2 base = floor(gl_FragCoord.xy) * 2.0;

    float depths[4];
    for (int i = 0; i < 4; i++) {
        vec2 texel = min(base + vec2(float(i & 1), float(i >> 1)), src_dims - vec2(1.0));
        float d = textureLod(sampler2D(depth_src, gtao_sampler), (texel + vec2(0.5)) / src_dims, 0.0).r;
        depths[i] = is_background_depth(d) ? BACKGROUND_DEPTH : d;
    }

    float farthest = max(max(depths[0], depths[1]), max(depths[2], depths[3]));
    // All four background: stay background rather than averaging 1e30 four ways.
    if (farthest >= BACKGROUND_DEPTH) {
        frag_depth = BACKGROUND_DEPTH;
        return;
    }

    // The same falloff shape the occlusion pass uses, at the scale one texel of this
    // level stands for. A sample within `from` of the farthest counts in full and one
    // a full `range` in front of it not at all, so the near surface of a silhouette
    // drops out instead of dragging the average forward.
    float radius = max(mip.w, 1e-4) * 0.75 * 1.457;
    float range = 0.615 * radius;
    float from = radius * (1.0 - 0.615);
    float mul = -1.0 / range;
    float add = from / range + 1.0;

    float sum = 0.0;
    float weight_sum = 0.0;
    for (int i = 0; i < 4; i++) {
        float d = min(depths[i], farthest);
        // The farthest sample always weighs 1, so the sum can never be zero.
        float w = clamp((farthest - d) * mul + add, 0.0, 1.0);
        sum += d * w;
        weight_sum += w;
    }
    frag_depth = sum / max(weight_sum, 1e-6);
}
@end

// 5x5 edge-aware denoise over the AO, run once, twice or three times by quality.
//
// Two weights replace what the old blur got wrong. Its depth weight compared raw view
// Z against a sigma scaled by the AO *radius*, so a small radius made the kernel
// reject nearly every neighbour — exactly the setting whose input is noisiest — and a
// surface seen at a glancing angle rejected its own neighbours because their Z
// legitimately differs. This measures distance from the centre pixel's tangent
// **plane** instead, which a flat surface satisfies at any angle, in a tolerance of a
// couple of pixels' own view-space size, which is scale- and depth-independent. And
// its normal weight cut off hard at dot < 0.75, throwing away most of the kernel on
// any curved surface; this one falls off smoothly instead, so a curve keeps its
// support and only a genuine crease loses it.
@fs fs_gtao_denoise
@include_block gtao_common

layout(binding=1) uniform texture2D ao_in;

layout(location=0) in vec2 v_uv;
layout(location=0) out float frag_ao;

// The tangent-plane tolerance, in multiples of a pixel's own view-space size.
const float PLANE_SIGMA_PX = 2.0;
// Falloff of the spatial term, in pixels.
const float SPATIAL_SIGMA = 1.5;
// Sharpness of the normal term. 8 keeps a 30° difference at about half weight and a
// 90° crease at none.
const float NORMAL_POWER = 8.0;

void main() {
    vec4 center_g = textureLod(sampler2D(gbuffer, gtao_sampler), v_uv, 0.0);
    vec3 center_n = center_g.xyz;
    float center_z = center_g.w;
    if (dot(center_n, center_n) < 0.25 || center_z >= -1e-4) {
        frag_ao = 1.0;
        return;
    }

    vec2 texel = 1.0 / target_dims();
    vec3 n0 = normalize(center_n);
    vec3 p0 = reconstruct_view_pos(v_uv, center_z);
    float plane_sigma = max(PLANE_SIGMA_PX * view_pixel_size(center_z), 1e-6);
    float sum = 0.0;
    float weight_sum = 0.0;
    for (int x = -2; x <= 2; x++) {
        for (int y = -2; y <= 2; y++) {
            vec2 pixel_offset = vec2(float(x), float(y));
            vec2 uv = v_uv + pixel_offset * texel;
            vec4 g = textureLod(sampler2D(gbuffer, gtao_sampler), uv, 0.0);
            if (dot(g.xyz, g.xyz) < 0.25 || g.w >= -1e-4) {
                continue;
            }

            // Distance from the centre pixel's tangent plane: zero across a flat
            // surface however steeply it is seen, large across a step.
            float plane_distance = dot(n0, reconstruct_view_pos(uv, g.w) - p0);
            float plane_weight =
                exp(-(plane_distance * plane_distance) / (2.0 * plane_sigma * plane_sigma));
            float normal_weight = pow(max(dot(n0, normalize(g.xyz)), 0.0), NORMAL_POWER);
            float spatial_weight =
                exp(-dot(pixel_offset, pixel_offset) / (2.0 * SPATIAL_SIGMA * SPATIAL_SIGMA));
            float weight = spatial_weight * plane_weight * normal_weight;
            sum += textureLod(sampler2D(ao_in, gtao_sampler), uv, 0.0).r * weight;
            weight_sum += weight;
        }
    }
    if (weight_sum <= 1e-5) {
        frag_ao = textureLod(sampler2D(ao_in, gtao_sampler), v_uv, 0.0).r;
        return;
    }
    frag_ao = sum / weight_sum;
}
@end

// ===========================================================================
// Composite
// ===========================================================================

// Samples the offscreen linear-HDR scene targets, applies ambient-only GTAO, tone
// maps, encodes to sRGB, and writes the final LDR color to the swapchain.
@fs fs_post
@include_block srgb

layout(binding=0) uniform post_params {
    // The four scalars below fill exactly one 16-byte std140 slot; adding or removing
    // any one silently shifts `bg_top` unless the Rust struct moves in lockstep, so
    // keep the count a multiple of four. `int` rather than the old HLSL `uint`
    // because shdc's uniform-block subset has no unsigned type — same bits.
    int gtao_enabled;
    int tonemap_enabled;
    int tonemap_op;
    // When non-zero, blit the (already display-ready) scene color straight out — no
    // GTAO, tone map or sRGB encode. Set for the buffer-inspection view, whose scene
    // shader emits faithful final pixels itself.
    int passthrough;
    // Viewport background fill, in display (sRGB) space (`xyz`; `w` unused). The
    // composite paints `mix(bg_top, bg_bottom, v)` wherever the scene's coverage is
    // below 1, so the chosen color is the exact displayed value (after, not before,
    // tone mapping). A flat preset sets both equal; the gradient sets distinct ends.
    vec4 bg_top;
    vec4 bg_bottom;
};

layout(binding=0) uniform texture2D scene_color;
layout(binding=1) uniform texture2D gtao_texture;    // blurred GTAO occlusion (R8)
layout(binding=2) uniform texture2D ambient_texture; // linear-HDR ambient radiance
layout(binding=0) uniform sampler scene_sampler;

layout(location=0) in vec2 v_uv;
layout(location=0) out vec4 frag_color;

// Khronos PBR Neutral tone mapping (Rec.709 linear). A port of the Khronos Group's
// reference implementation, KhronosGroup/ToneMapping `PBR_Neutral/pbrNeutral.glsl`
// (Apache-2.0) - see docs/THIRD-PARTY-NOTICES.md.
vec3 pbr_neutral_tonemap(vec3 color_in) {
    const float start_compression = 0.8 - 0.04;
    const float desaturation = 0.15;
    vec3 color = color_in;
    float x = min(color.r, min(color.g, color.b));
    float offset = (x < 0.08) ? (x - 6.25 * x * x) : 0.04;
    color = color - offset;
    float peak = max(color.r, max(color.g, color.b));
    if (peak < start_compression) {
        return color;
    }
    float d = 1.0 - start_compression;
    float new_peak = 1.0 - d * d / (peak + d - start_compression);
    color = color * (new_peak / peak);
    float g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, new_peak * vec3(1.0), g);
}

vec3 reinhard_tonemap(vec3 color) {
    return color / (vec3(1.0) + color);
}

// The fitted ACES curve (Stephen Hill's RRT + ODT fit, via MJP's BakingLab) as
// three.js ships it in `ACESFilmicToneMapping` - including its 1/0.6 exposure lift
// for a brighter viewing environment. Ported from three.js (MIT, (c) 2010-2026
// three.js authors) - see docs/THIRD-PARTY-NOTICES.md.
vec3 aces_rrt_odt_fit(vec3 v) {
    vec3 a = v * (v + 0.0245786) - 0.000090537;
    vec3 b = v * (0.983729 * v + 0.432951) + 0.238081;
    return a / b;
}

// GLSL's `mat3(c0, c1, c2)` takes *columns*, so these are the verbatim equivalent of
// the old HLSL `mat_from_cols` helper plus `mul(M, v)`.
vec3 aces_tonemap(vec3 color_in) {
    mat3 input_mat = mat3(
        vec3(0.59719, 0.07600, 0.02840),
        vec3(0.35458, 0.90834, 0.13383),
        vec3(0.04823, 0.01566, 0.83777));
    mat3 output_mat = mat3(
        vec3(1.60475, -0.10208, -0.00327),
        vec3(-0.53108, 1.10813, -0.07276),
        vec3(-0.07367, -0.00605, 1.07602));
    vec3 color = color_in / 0.6;
    color = input_mat * color;
    color = aces_rrt_odt_fit(color);
    color = output_mat * color;
    return clamp(color, 0.0, 1.0);
}

// AgX, ported from three.js's `AgXToneMapping` (MIT, (c) 2010-2026 three.js authors),
// itself after Filament's implementation of Blender's AgX and Benjamin Wrensch's
// polynomial fit of the default contrast curve - see docs/THIRD-PARTY-NOTICES.md.
vec3 agx_contrast_approx(vec3 x) {
    vec3 x2 = x * x;
    vec3 x4 = x2 * x2;
    return 15.5 * x4 * x2
        - 40.14 * x4 * x
        + 31.96 * x4
        - 6.868 * x2 * x
        + 0.4298 * x2
        + 0.1191 * x
        - 0.00232;
}

vec3 agx_tonemap(vec3 color_in) {
    mat3 srgb_to_rec2020 = mat3(
        vec3(0.6274, 0.0691, 0.0164),
        vec3(0.3293, 0.9195, 0.0880),
        vec3(0.0433, 0.0113, 0.8956));
    mat3 rec2020_to_srgb = mat3(
        vec3(1.6605, -0.1246, -0.0182),
        vec3(-0.5876, 1.1329, -0.1006),
        vec3(-0.0728, -0.0083, 1.1187));
    mat3 inset = mat3(
        vec3(0.856627153315983, 0.137318972929847, 0.11189821299995),
        vec3(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        vec3(0.0482516061458583, 0.101439036467562, 0.811302368396859));
    mat3 outset = mat3(
        vec3(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        vec3(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
        vec3(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405));
    const float min_ev = -12.47393;
    const float max_ev = 4.026069;

    vec3 color = srgb_to_rec2020 * color_in;
    color = inset * color;
    color = max(color, 1e-10);
    color = log2(color);
    color = (color - min_ev) / (max_ev - min_ev);
    color = clamp(color, 0.0, 1.0);
    color = agx_contrast_approx(color);
    color = outset * color;
    color = pow(max(color, vec3(0.0)), vec3(2.2));
    color = rec2020_to_srgb * color;
    return clamp(color, 0.0, 1.0);
}

// Select the operator (indices match `TonemapOperator::shader_index`). When tone
// mapping is off the linear radiance passes straight to the sRGB encode.
vec3 apply_tonemap(vec3 color) {
    if (tonemap_enabled == 0) {
        return color;
    }
    switch (tonemap_op) {
        case 0: return pbr_neutral_tonemap(color);
        case 1: return color;
        case 2: return reinhard_tonemap(color);
        case 3: return aces_tonemap(color);
        case 4: return agx_tonemap(color);
        default: return pbr_neutral_tonemap(color);
    }
}

void main() {
    vec4 scene = texture(sampler2D(scene_color, scene_sampler), v_uv);
    vec3 lit = scene.rgb;
    // Coverage = the resolved scene alpha (1 where opaque geometry / the skybox
    // wrote, 0 over the cleared background, fractional at MSAA silhouette edges and
    // through transparent surfaces). The MSAA resolve premultiplies the color by
    // coverage, so divide it back out to recover the surface color before tone
    // mapping, then composite over the background by coverage — giving clean
    // anti-aliased edges against any background.
    float coverage = scene.a;
    vec3 surface = lit / max(coverage, 1e-4);
    // Vertical background gradient in display space (uv.y = 0 at the top).
    vec3 bg = mix(bg_top.rgb, bg_bottom.rgb, clamp(v_uv.y, 0.0, 1.0));
    // Buffer-inspection view: the scene shader already wrote final display pixels,
    // so pass them through unchanged (faithful — the shown value is the data),
    // composited over the chosen background where no geometry covers the pixel.
    if (passthrough != 0) {
        frag_color = vec4(mix(bg, surface, coverage), 1.0);
        return;
    }
    if (gtao_enabled != 0) {
        float ao = texture(sampler2D(gtao_texture, scene_sampler), v_uv).r;
        vec3 ambient = texture(sampler2D(ambient_texture, scene_sampler), v_uv).rgb;
        lit = max(lit - ambient * (1.0 - ao), vec3(0.0));
        surface = lit / max(coverage, 1e-4);
    }
    vec3 scene_srgb = linear_to_srgb(apply_tonemap(surface));
    frag_color = vec4(mix(bg, scene_srgb, coverage), 1.0);
}
@end

// ===========================================================================
// Tex viewport
// ===========================================================================

// One pooled texture drawn channel-isolated with pan/zoom from the uniform, plus the
// transparency checkerboard behind it. Deliberately outside the scene MRT / tone-map
// path so the displayed texel equals the stored texel: the source is uploaded as
// plain UNORM, so the sample returns the raw stored bytes for every channel alike.
@block tex_common
layout(binding=0) uniform tex_params {
    // Image rectangle in framebuffer pixels: top-left corner + size. A fragment's
    // image uv is (frag_pixel - img_min) / img_size.
    vec2 img_min;
    vec2 img_size;
    // 0 = RGB (alpha composites over the background), 1..4 = R/G/B/A isolated as
    // opaque greyscale.
    int channel;
    // 1 when the framebuffer encodes linear→sRGB on write (0 for our UNORM
    // swapchain).
    int target_srgb;
    // Checker cell size in framebuffer pixels.
    float checker_cell;
    int pad;
    // Checker colors (gamma-space, written verbatim).
    vec4 bg_light;
    vec4 bg_dark;
};
@end

@fs fs_tex_image
@include_block tex_common
@include_block srgb

layout(binding=0) uniform texture2D tex;
layout(binding=0) uniform sampler samp;

layout(location=0) out vec4 frag_color;

void main() {
    vec2 uv = (gl_FragCoord.xy - img_min) / img_size;
    // Sample unconditionally (uniform control flow) so the implicit-derivative LOD
    // is valid, then discard fragments outside the image so the background shows.
    vec4 raw = texture(sampler2D(tex, samp), uv);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        discard;
    }

    vec3 rgb;
    float alpha = 1.0;
    switch (channel) {
        case 0:
            rgb = raw.rgb;
            alpha = raw.a;
            break;
        case 1:
            rgb = vec3(raw.r);
            break;
        case 2:
            rgb = vec3(raw.g);
            break;
        case 3:
            rgb = vec3(raw.b);
            break;
        default:
            rgb = vec3(raw.a);
            break;
    }

    // `rgb` holds the stored source bytes (UNORM sample, no decode). On an sRGB
    // framebuffer the GPU would encode linear→sRGB on write, so pre-apply the
    // inverse; our UNORM swapchain stores verbatim (`target_srgb` is 0).
    vec3 out_rgb = (target_srgb == 1) ? srgb_to_linear(rgb) : rgb;
    frag_color = vec4(out_rgb, alpha);
}
@end

@fs fs_tex_checker
@include_block tex_common

layout(location=0) out vec4 frag_color;

void main() {
    float cell = max(checker_cell, 1.0);
    vec2 c = floor(gl_FragCoord.xy / cell);
    float parity = mod(c.x + c.y, 2.0);
    frag_color = (parity < 0.5) ? bg_light : bg_dark;
}
@end

// ===========================================================================
// egui
// ===========================================================================

// The egui chrome, drawn last into the swapchain pass. egui hands us premultiplied
// gamma-space colors and a texture atlas in the same space, so the fragment is a
// plain `texture * color` with no gamma conversion — the contract egui-directx11
// relied on, kept because our swapchain is plain UNORM (D20).
@vs vs_egui
layout(binding=0) uniform egui_params {
    // Logical screen size in points; the vertex positions arrive in the same space.
    vec2 screen_size;
    vec2 egui_pad;
};

layout(location=0) in vec2 in_pos;
layout(location=1) in vec2 in_uv;
layout(location=2) in vec4 in_color;

layout(location=0) out vec2 v_uv;
layout(location=1) out vec4 v_color;

void main() {
    gl_Position = vec4(
        2.0 * in_pos.x / screen_size.x - 1.0,
        1.0 - 2.0 * in_pos.y / screen_size.y,
        0.0,
        1.0);
    v_uv = in_uv;
    v_color = in_color;
}
@end

@fs fs_egui
layout(binding=0) uniform texture2D egui_texture;
layout(binding=0) uniform sampler egui_sampler;

layout(location=0) in vec2 v_uv;
layout(location=1) in vec4 v_color;

layout(location=0) out vec4 frag_color;

void main() {
    frag_color = v_color * texture(sampler2D(egui_texture, egui_sampler), v_uv);
}
@end

// ===========================================================================
// IBL precompute (bake only)
// ===========================================================================

// Four passes driven by the offline bake tool: project the loaded equirectangular
// HDR onto a cubemap, cosine-convolve it into a diffuse irradiance cube,
// GGX-importance-sample it into the mip chain of a prefiltered specular cube (one
// roughness per mip), and integrate the split-sum BRDF response into a 2D LUT.
// These run once per environment at bake time, never at run time.
@vs vs_ibl
@include_block fullscreen

// Per-face / per-pass parameters. `forward/right/up` are the cube face's basis: a
// fullscreen-triangle position (clip xy in -1..1) maps to the world direction
// `forward + x*right + y*up`. `params.x` carries the prefilter roughness.
layout(binding=0) uniform face_params {
    vec4 forward;
    vec4 right;
    vec4 up;
    vec4 params;
};

// World-space direction for this fragment (cube passes).
layout(location=0) out vec3 v_local_dir;
// 0..1 screen coordinate (BRDF LUT pass): x = N·V, y = roughness.
layout(location=1) out vec2 v_uv;

void main() {
    vec2 xy = fullscreen_corner(gl_VertexIndex);
    gl_Position = vec4(xy, 0.0, 1.0);
    v_local_dir = forward.xyz + xy.x * right.xyz + xy.y * up.xyz;
    // Flip Y so texel row t (top = 0) maps to roughness = t for the BRDF LUT.
    v_uv = vec2(xy.x * 0.5 + 0.5, 1.0 - (xy.y * 0.5 + 0.5));
}
@end

@block ibl_common
// Binding 1, not 0: a uniform block belongs to one stage, and `vs_ibl` already
// holds slot 0 — the same split the scene block makes between `scene_vs` and
// `scene_fs`. Both carry the identical 64 bytes.
layout(binding=1) uniform face_params_fs {
    vec4 forward;
    vec4 right;
    vec4 up;
    vec4 params;
} face;

layout(location=0) in vec3 v_local_dir;
layout(location=1) in vec2 v_uv;

const float PI = 3.14159265359;

// Ceiling for sampled environment radiance in the convolution passes. The equirect
// upload is already clamped to finite f16, but a small, very bright source texel (a
// sun in a high-range HDR) still dominates the cosine / GGX integral, leaving
// fireflies in the diffuse irradiance and the specular mips.
const float IBL_RADIANCE_CLAMP = 64.0;

// Van der Corput / Hammersley low-discrepancy sequence for importance sampling.
float radical_inverse_vdc(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10;
}

vec2 hammersley(uint i, uint n) {
    return vec2(float(i) / float(n), radical_inverse_vdc(i));
}

// Sample a GGX half-vector around `n` for a given roughness, from a 2D random.
vec3 importance_sample_ggx(vec2 xi, vec3 n, float roughness) {
    float a = roughness * roughness;
    float phi = 2.0 * PI * xi.x;
    float cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    float sin_theta = sqrt(1.0 - cos_theta * cos_theta);

    vec3 h = vec3(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);

    vec3 up_axis = vec3(0.0, 0.0, 1.0);
    if (abs(n.z) > 0.999) {
        up_axis = vec3(1.0, 0.0, 0.0);
    }
    vec3 tangent = normalize(cross(up_axis, n));
    vec3 bitangent = cross(n, tangent);
    return normalize(tangent * h.x + bitangent * h.y + n * h.z);
}
@end

@fs fs_ibl_equirect
@include_block ibl_common

layout(binding=0) uniform texture2D src_equirect;
layout(binding=0) uniform sampler src_sampler;

layout(location=0) out vec4 frag_color;

// Map a unit direction to equirectangular uv: u around the equator, v from the top
// (+Y) down. Matches the orientation the HDR is uploaded with (row 0 = top).
vec2 dir_to_equirect_uv(vec3 dir) {
    vec3 n = normalize(dir);
    float u = atan(n.z, n.x) / (2.0 * PI) + 0.5;
    float v = acos(clamp(n.y, -1.0, 1.0)) / PI;
    return vec2(u, v);
}

void main() {
    vec2 uv = dir_to_equirect_uv(v_local_dir);
    vec3 color = textureLod(sampler2D(src_equirect, src_sampler), uv, 0.0).rgb;
    frag_color = vec4(color, 1.0);
}
@end

@fs fs_ibl_irradiance
@include_block ibl_common

layout(binding=1) uniform textureCube src_cube;
layout(binding=0) uniform sampler src_sampler;

layout(location=0) out vec4 frag_color;

void main() {
    vec3 normal = normalize(v_local_dir);

    // Build a tangent frame around the surface normal.
    vec3 up_axis = vec3(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) {
        up_axis = vec3(0.0, 0.0, 1.0);
    }
    vec3 tangent = normalize(cross(up_axis, normal));
    vec3 bitangent = cross(normal, tangent);

    vec3 irradiance = vec3(0.0);
    float samples = 0.0;
    const float sample_delta = 0.05;
    for (float phi = 0.0; phi < 2.0 * PI; phi += sample_delta) {
        for (float theta = 0.0; theta < 0.5 * PI; theta += sample_delta) {
            // Spherical → tangent-space → world.
            vec3 tangent_sample = vec3(
                sin(theta) * cos(phi),
                sin(theta) * sin(phi),
                cos(theta));
            vec3 sample_vec = tangent_sample.x * tangent
                + tangent_sample.y * bitangent
                + tangent_sample.z * normal;
            vec3 radiance = min(
                textureLod(samplerCube(src_cube, src_sampler), sample_vec, 0.0).rgb,
                vec3(IBL_RADIANCE_CLAMP));
            irradiance += radiance * cos(theta) * sin(theta);
            samples += 1.0;
        }
    }
    irradiance = PI * irradiance / max(samples, 1.0);
    frag_color = vec4(irradiance, 1.0);
}
@end

@fs fs_ibl_prefilter
@include_block ibl_common

layout(binding=1) uniform textureCube src_cube;
layout(binding=0) uniform sampler src_sampler;

layout(location=0) out vec4 frag_color;

void main() {
    vec3 normal = normalize(v_local_dir);
    vec3 view_dir = normal;
    float roughness = face.params.x;

    uint sample_count = 64u;
    vec3 prefiltered = vec3(0.0);
    float total_weight = 0.0;
    for (uint i = 0u; i < sample_count; i++) {
        vec2 xi = hammersley(i, sample_count);
        vec3 h = importance_sample_ggx(xi, normal, roughness);
        vec3 l = normalize(2.0 * dot(view_dir, h) * h - view_dir);
        float n_dot_l = max(dot(normal, l), 0.0);
        if (n_dot_l > 0.0) {
            vec3 radiance = min(
                textureLod(samplerCube(src_cube, src_sampler), l, 0.0).rgb,
                vec3(IBL_RADIANCE_CLAMP));
            prefiltered += radiance * n_dot_l;
            total_weight += n_dot_l;
        }
    }
    if (total_weight > 0.0) {
        prefiltered = prefiltered / total_weight;
    }
    frag_color = vec4(prefiltered, 1.0);
}
@end

@fs fs_ibl_brdf
@include_block ibl_common

layout(location=0) out vec2 frag_brdf;

// Smith geometry term for IBL (k uses the IBL remap, not the direct-lighting one).
float geometry_schlick_ggx(float n_dot_v, float roughness) {
    float k = (roughness * roughness) / 2.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

float geometry_smith(float n_dot_v, float n_dot_l, float roughness) {
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

void main() {
    float n_dot_v = max(v_uv.x, 1e-4);
    float roughness = v_uv.y;

    vec3 view = vec3(sqrt(1.0 - n_dot_v * n_dot_v), 0.0, n_dot_v);
    vec3 normal = vec3(0.0, 0.0, 1.0);

    float a = 0.0;
    float b = 0.0;
    uint sample_count = 512u;
    for (uint i = 0u; i < sample_count; i++) {
        vec2 xi = hammersley(i, sample_count);
        vec3 h = importance_sample_ggx(xi, normal, roughness);
        vec3 l = normalize(2.0 * dot(view, h) * h - view);

        float n_dot_l = max(l.z, 0.0);
        float n_dot_h = max(h.z, 0.0);
        float v_dot_h = max(dot(view, h), 0.0);
        if (n_dot_l > 0.0) {
            float g = geometry_smith(n_dot_v, n_dot_l, roughness);
            float g_vis = (g * v_dot_h) / (n_dot_h * n_dot_v);
            float fc = pow(max(1.0 - v_dot_h, 0.0), 5.0);
            a += (1.0 - fc) * g_vis;
            b += fc * g_vis;
        }
    }
    frag_brdf = vec2(a, b) / float(sample_count);
}
@end

// ===========================================================================
// Aud: the overdraw views
// ===========================================================================
//
// Both count into a single-sample R16F target with additive blending, drawing every
// visible triangle as three pulled vertices: entry `3t + k` of the index list names
// corner `k` of triangle `t` in the mesh's own vertex buffer, so the geometry is read
// rather than copied (invariant 1) and deforms as the mesh does.
//
//   overdraw_count  1 per layer covering a pixel, no depth test: how many surfaces
//                   a pixel is shaded for (also the quad view's depth prepass)
//   quad_overdraw   per visible pixel, 4 / k, where k is how many of its 2x2 quad's
//                   pixel centres its own triangle covers: the shading a GPU spends
//                   on helper lanes, since a quad is always shaded whole
//   overdraw_ramp   the count, as a colour, into the view's rectangle

@block overdraw_indices
// One corner index of the triangle list. A struct of one `uint` because a storage
// buffer must hold an array of a struct; named as the wireframe's edge list is, since
// it is bound the same way.
struct LineIndex {
    uint index;
};

layout(binding=17) readonly buffer line_indices { LineIndex line_index[]; };
@end

@vs vs_overdraw
@include_block scene_uniforms_vs
@include_block deform
@include_block pulled_vertex
@include_block overdraw_indices

void main() {
    uint corner = line_index[uint(gl_VertexIndex)].index;
    gl_Position = su.view_projection * vec4(line_corner_position(corner), 1.0);
}
@end

@fs fs_overdraw_count
layout(location=0) out vec4 frag_count;

void main() {
    frag_count = vec4(1.0, 0.0, 0.0, 1.0);
}
@end

@vs vs_quad_overdraw
@include_block scene_uniforms_vs
@include_block deform
@include_block pulled_vertex
@include_block overdraw_indices

// The triangle's three corners in clip space (x, y, w), the same for every fragment
// of it: what lets a fragment test pixels its triangle does not cover.
layout(location=0) flat out vec3 v_corner_a;
layout(location=1) flat out vec3 v_corner_b;
layout(location=2) flat out vec3 v_corner_c;

void main() {
    uint vertex = uint(gl_VertexIndex);
    uint triangle = vertex / 3u;
    vec4 a = su.view_projection * vec4(line_corner_position(line_index[3u * triangle].index), 1.0);
    vec4 b = su.view_projection * vec4(line_corner_position(line_index[3u * triangle + 1u].index), 1.0);
    vec4 c = su.view_projection * vec4(line_corner_position(line_index[3u * triangle + 2u].index), 1.0);
    uint corner = vertex - triangle * 3u;
    gl_Position = corner == 0u ? a : (corner == 1u ? b : c);
    v_corner_a = a.xyw;
    v_corner_b = b.xyw;
    v_corner_c = c.xyw;
}
@end

@fs fs_quad_overdraw
// The count target's size, which turns a pixel into NDC. Its own block, and its own
// slot: a uniform block belongs to one stage, and the line block is the vertex
// stage's.
layout(binding=4) uniform quad_params {
    // x / y = the count target's size in pixels, z / w spare.
    vec4 params;
} qu;

layout(location=0) flat in vec3 v_corner_a;
layout(location=1) flat in vec3 v_corner_b;
layout(location=2) flat in vec3 v_corner_c;

layout(location=0) out vec4 frag_count;

void main() {
    // Homogeneous 2D edge functions (Olano and Greer): with the corners as the
    // columns of M = [a b c], the adjugate's rows are the three edge functions, and a
    // point (x, y, 1) in NDC is inside exactly when each has the sign of det(M) —
    // with no divide by w, so a corner behind the eye needs no clipping first.
    vec3 e0 = cross(v_corner_b, v_corner_c);
    vec3 e1 = cross(v_corner_c, v_corner_a);
    vec3 e2 = cross(v_corner_a, v_corner_b);
    float det = dot(v_corner_a, e0);
    vec2 size = max(qu.params.xy, vec2(1.0));
    // The fragment's 2x2 quad, from its pixel (top-left origin, as `gl_FragCoord` is
    // on every backend this ships on).
    vec2 pixel = floor(gl_FragCoord.xy);
    vec2 quad = pixel - mod(pixel, vec2(2.0));
    float covered = 0.0;
    for (int i = 0; i < 4; i++) {
        vec2 centre = quad + vec2(float(i & 1), float(i >> 1)) + vec2(0.5);
        vec3 p = vec3(centre.x / size.x * 2.0 - 1.0, 1.0 - centre.y / size.y * 2.0, 1.0);
        vec3 edges = vec3(dot(e0, p), dot(e1, p), dot(e2, p)) * sign(det);
        covered += (edges.x >= 0.0 && edges.y >= 0.0 && edges.z >= 0.0) ? 1.0 : 0.0;
    }
    // An edge-on triangle (det 0) cannot say; it is counted as a full quad. The
    // fragment's own pixel is covered by construction, so k is at least 1.
    float k = abs(det) > 1e-12 ? max(covered, 1.0) : 4.0;
    frag_count = vec4(4.0 / k, 0.0, 0.0, 1.0);
}
@end

@fs fs_overdraw_ramp
layout(binding=0) uniform overdraw_params {
    // The view's rectangle in framebuffer pixels: top-left corner + size.
    vec4 rect;
    // x = the count shown as the ramp's top, y = the count shown as its bottom.
    vec4 range;
    // The ramp's four stops, low to high (gamma rgb), and the colour for no surface.
    vec4 stop0;
    vec4 stop1;
    vec4 stop2;
    vec4 stop3;
    vec4 empty;
};

layout(binding=0) uniform texture2D count_tex;
layout(binding=0) uniform sampler count_smp;

layout(location=0) out vec4 frag_color;

void main() {
    vec2 uv = (gl_FragCoord.xy - rect.xy) / max(rect.zw, vec2(1.0));
    float count = textureLod(sampler2D(count_tex, count_smp), uv, 0.0).r;
    if (count <= 0.0) {
        frag_color = vec4(empty.rgb, 1.0);
        return;
    }
    float t = clamp((count - range.y) / max(range.x - range.y, 1e-3), 0.0, 1.0) * 3.0;
    vec3 rgb = t < 1.0 ? mix(stop0.rgb, stop1.rgb, t)
        : (t < 2.0 ? mix(stop1.rgb, stop2.rgb, t - 1.0) : mix(stop2.rgb, stop3.rgb, t - 2.0));
    frag_color = vec4(rgb, 1.0);
}
@end

// ===========================================================================
// Programs
// ===========================================================================

// The mesh, the GTAO G-buffer and the selection flash share `vs_main`; the two line
// programs pull their vertices instead (see `line_quad`) but deform them through the
// same `apply_deform`, so every mesh-derived overlay moves with the mesh.
@program mesh vs_main fs_main
@program selection vs_main fs_selection
@program gtao_gbuffer vs_main fs_gtao_gbuffer
@program line vs_line fs_line
@program wire vs_wire fs_line

@program skybox vs_skybox fs_skybox

@program gtao vs_fullscreen fs_gtao
@program gtao_denoise vs_fullscreen fs_gtao_denoise
// Works from `gl_FragCoord` rather than a uv: it reduces a level twice its own size,
// so it addresses source *texels* directly instead of resampling a shared uv.
@program gtao_depth_mip vs_fullscreen_bare fs_gtao_depth_mip
@program post vs_fullscreen fs_post

@program tex_image vs_fullscreen_bare fs_tex_image
@program tex_checker vs_fullscreen_bare fs_tex_checker

@program egui vs_egui fs_egui

@program ibl_equirect vs_ibl fs_ibl_equirect
@program ibl_irradiance vs_ibl fs_ibl_irradiance
@program ibl_prefilter vs_ibl fs_ibl_prefilter
@program ibl_brdf vs_ibl fs_ibl_brdf

@program overdraw_count vs_overdraw fs_overdraw_count
@program quad_overdraw vs_quad_overdraw fs_quad_overdraw
@program overdraw_ramp vs_fullscreen_bare fs_overdraw_ramp
