// Scene shader (HLSL / Shader Model 5.0). This is the sole scene shader (the
// wgpu→D3D11 migration is complete; no WGSL remains in-tree). It is compiled
// offline to DXBC by `fxc` (see `build.rs`) and shipped as bytecode (no runtime
// shader compilation).
//
// Entry points: `vs_main` (shared mesh/line vertex shader), `fs_main` (the full
// per-material PBR / IBL / checker / vertex-color / unlit / alpha path), `fs_line`
// (the flat-color grid + line overlays), and `vs_skybox`/`fs_skybox` (the
// environment background). The GTAO G-buffer + selection fill land in later phases.
//
// Conventions that must hold (see the migration plan's HLSL guardrails):
//  * The `SceneUniforms` cbuffer (b0) is byte-for-byte the `#[repr(C)]`
//    `SceneUniforms` in `scene/gpu_types.rs`; the `MaterialUniform` cbuffer (b1) is
//    byte-for-byte `material::MaterialUniform` (invariant 11). Every field is a
//    16-byte-aligned `float4`/`float4x4`, so HLSL's cbuffer packing matches the
//    Rust layout with no extra padding.
//  * Matrices are uploaded column-major (`glam::Mat4::to_cols_array_2d`) and read
//    with HLSL's default `column_major` packing, so `mul(M, v)` is the standard
//    matrix × column-vector product — the same math as WGSL's `M * v`.
//  * Depth is Reversed-Z (near → 1, far → 0) in the D3D [0,1] clip range, matching
//    `glam`'s `*_reverse_rh` projections; the depth-stencil state clears to 0 and
//    compares `GREATER_EQUAL`.
//  * The `VsInput` semantics line up with the `ID3D11InputLayout` built in
//    `rhi::pipeline` from the `#[repr(C)]` `SceneVertex`.
//
// Register plan (manual b#/t#/s# assignment, all space0 — see the migration plan):
//   b0 SceneUniforms (VS+PS) · b1 MaterialUniform (PS)
//   t0 checker (s0) · t1 irradiance / t2 prefilter / t3 brdf / t4 env cube (s1 IBL)
//   t5..t11 the seven material slots (s2 material aniso sampler)

cbuffer SceneUniforms : register(b0)
{
    float4x4 view_projection;
    float4x4 inv_view_projection;
    float4   render_options;
    float4   camera_position;
    float4   env_params;
    float4   projection_params;
    float4x4 view;
    float4   selection_color;
};

// Per-material parameters (bind group 3 in WGSL → cbuffer b1 here). Must match the
// `#[repr(C)]` `MaterialUniform` (invariant 11). `params` = (metallic, roughness,
// slot_flags bitfield, alpha mode). `channels0` selects the channel (0 R … 3 A)
// feeding slots 0..3; `channels1` does the same for slots 4..6 with `w` the alpha
// cutoff. `flags.x` is the roughness workflow (0 roughness, 1 smoothness).
cbuffer MaterialUniform : register(b1)
{
    float4 mat_base_color;
    float4 mat_emissive;
    float4 mat_params;
    float4 mat_channels0;
    float4 mat_channels1;
    float4 mat_flags;
};

// Group 1: the UV-checker texture + its sampler.
Texture2D    checker_texture : register(t0);
SamplerState checker_sampler : register(s0);

// Group 2: the precomputed image-based-lighting maps + the shared IBL sampler.
TextureCube  irradiance_cube : register(t1);
TextureCube  prefilter_cube  : register(t2);
Texture2D    brdf_lut        : register(t3);
TextureCube  env_cube        : register(t4);
SamplerState ibl_sampler     : register(s1);

// Group 3: the seven material texture slots + their shared (anisotropic) sampler.
Texture2D    base_color_tex  : register(t5);
Texture2D    normal_tex      : register(t6);
Texture2D    roughness_tex   : register(t7);
Texture2D    metallic_tex    : register(t8);
Texture2D    ao_tex          : register(t9);
Texture2D    emissive_tex    : register(t10);
Texture2D    opacity_tex     : register(t11);
SamplerState material_sampler : register(s2);

struct VsInput
{
    float3 position : POSITION;
    float3 normal   : NORMAL;
    float2 uv       : TEXCOORD0;
    float4 tangent  : TANGENT;
    float4 color    : COLOR0;
};

struct VsOutput
{
    float4 clip_position  : SV_Position;
    float3 normal         : NORMAL;
    float2 uv             : TEXCOORD0;
    float4 color          : COLOR0;
    float3 world_position : TEXCOORD1;
    float4 tangent        : TANGENT;
};

VsOutput vs_main(VsInput input)
{
    VsOutput output;
    output.clip_position = mul(view_projection, float4(input.position, 1.0));
    output.normal = input.normal;
    output.uv = input.uv;
    output.color = input.color;
    output.world_position = input.position;
    output.tangent = input.tangent;
    return output;
}

// Decode an sRGB-authored color into the scene's linear-HDR working space. Tone
// mapping + the final sRGB encode happen once in the post composite (`post.hlsl`).
// Mirrors `scene.wgsl`'s `srgb_to_linear` (WGSL `select(hi, lo, c <= 0.04045)`;
// here `step(c, edge)` is 1 where `c <= edge`, so `lerp(hi, lo, cutoff)` picks the
// low branch there).
float3 srgb_to_linear(float3 c)
{
    float3 lo = c / 12.92;
    // max() guards fxc's negative-base pow warning (/WX); the base is >= 0 for any
    // valid (non-negative) color, so this changes nothing for real inputs.
    float3 hi = pow(max((c + 0.055) / 1.055, 0.0), 2.4);
    float3 cutoff = step(c, 0.04045);
    return lerp(hi, lo, cutoff);
}

// Select one channel of a sampled texel by index (0 R, 1 G, 2 B, 3 A), so a packed
// map can route any channel into a scalar property.
float select_channel(float4 texel, float index)
{
    if (index < 0.5) { return texel.r; }
    if (index < 1.5) { return texel.g; }
    if (index < 2.5) { return texel.b; }
    return texel.a;
}

// Perturb the geometric normal `n` by a tangent-space normal-map sample, using the
// vertex tangent (xyz) + handedness (w) to build the TBN basis. Geometry is
// world-baked, so the tangent is already world-space (no model matrix).
float3 apply_normal_map(float3 n, float4 tangent, float3 sample_rgb)
{
    float3 geo_n = normalize(n);
    // Gram-Schmidt orthonormalize the tangent against the normal.
    float3 t = normalize(tangent.xyz - geo_n * dot(geo_n, tangent.xyz));
    if (dot(t, t) < 1e-8)
    {
        return geo_n;
    }
    float3 b = cross(geo_n, t) * tangent.w;
    float3 m = sample_rgb * 2.0 - 1.0;
    return normalize(t * m.x + b * m.y + geo_n * m.z);
}

// Rotate an environment sample direction about the world Y axis by the negative of
// the configured environment yaw (`projection_params.y`), so increasing the yaw
// spins the whole environment. Applied to every cube lookup (irradiance,
// prefiltered specular, skybox) so the IBL + skybox rotate together rigidly.
float3 env_sample_dir(float3 dir)
{
    float angle = -projection_params.y;
    float s = sin(angle);
    float c = cos(angle);
    return float3(c * dir.x + s * dir.z, dir.y, -s * dir.x + c * dir.z);
}

// Fresnel-Schlick with a roughness term, so rough surfaces don't over-brighten at
// grazing angles (the IBL ambient form).
float3 fresnel_schlick_roughness(float cos_theta, float3 f0, float roughness)
{
    float3 inv_rough = (float3)(1.0 - roughness);
    return f0 + (max(inv_rough, f0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

struct ShadingResult
{
    float3 color;
    float3 ambient;
};

// Environment-lit metallic-roughness PBR for the shaded path. Returns linear
// radiance and the diffuse ambient term GTAO may attenuate (tone mapping happens in
// post). Mirrors `scene.wgsl`'s `shade_ibl`.
ShadingResult shade_ibl(float3 albedo, float3 world_normal, float3 world_pos, float roughness, float metallic)
{
    float3 n = normalize(world_normal);
    float3 v = normalize(camera_position.xyz - world_pos);
    float3 r = reflect(-v, n);
    float n_dot_v = max(dot(n, v), 1e-4);
    float3 f0 = lerp((float3)0.04, albedo, metallic);

    // Diffuse: cosine-convolved irradiance modulated by albedo, yaw-rotated.
    float3 irradiance = irradiance_cube.SampleLevel(ibl_sampler, env_sample_dir(n), 0.0).rgb;
    float3 diffuse = irradiance * albedo;

    // Specular: split-sum — prefiltered env at the roughness mip, scaled by the
    // BRDF LUT (scale + bias). Same yaw rotation applied to the reflection lookup.
    float max_lod = env_params.w;
    float3 prefiltered = prefilter_cube.SampleLevel(ibl_sampler, env_sample_dir(r), roughness * max_lod).rgb;
    float2 brdf = brdf_lut.SampleLevel(ibl_sampler, float2(n_dot_v, roughness), 0.0).rg;
    float3 fresnel = fresnel_schlick_roughness(n_dot_v, f0, roughness);
    float3 specular = prefiltered * (fresnel * brdf.x + brdf.y);

    float3 kd = ((float3)1.0 - fresnel) * (1.0 - metallic);
    float3 ambient = kd * diffuse * env_params.y;

    ShadingResult result;
    result.color = ambient + specular * env_params.y;
    result.ambient = ambient;
    return result;
}

// Scene fragment output (MRT): location 0 is the linear-HDR color the composite
// tone-maps; location 1 is the linear ambient radiance GTAO may attenuate (Phase
// 3). Overlays/lines write 0 ambient *color* with the overlay alpha so they never
// bloom or get AO-darkened. Mirrors `scene.wgsl`'s `FragOutput`.
struct FragOutput
{
    float4 color   : SV_Target0;
    float4 ambient : SV_Target1;
};

// The full per-material shaded / unlit / uv-checker / vertex-color / alpha path —
// the Direct3D 11 port of `scene.wgsl`'s `fs_main`. All textures are sampled at the
// top (uniform control flow) before any branch, as in the WGSL.
FragOutput fs_main(VsOutput input)
{
    FragOutput out_frag;
    out_frag.ambient = float4(0.0, 0.0, 0.0, 0.0);

    float shading_mode = render_options.x;
    bool uv_checker_enabled = render_options.y > 0.5;
    float tiling = max(render_options.z, 1.0);
    // Vertex-color view: -1 off, else mode (0 RGB, 1 Alpha, 2 RGB+A).
    float vertex_color_mode = render_options.w;
    // FBX authors UVs bottom-left (V up); D3D samples top-left (V down), so flip V
    // once here at the single UV→texel boundary for the checker + every slot.
    float2 tex_uv = float2(input.uv.x, 1.0 - input.uv.y);
    float4 checker = checker_texture.Sample(checker_sampler, tex_uv * tiling);
    // Sample every material slot at the top (uniform control flow).
    float4 tex_base = base_color_tex.Sample(material_sampler, tex_uv);
    float4 tex_normal = normal_tex.Sample(material_sampler, tex_uv);
    float4 tex_roughness = roughness_tex.Sample(material_sampler, tex_uv);
    float4 tex_metallic = metallic_tex.Sample(material_sampler, tex_uv);
    float4 tex_ao = ao_tex.Sample(material_sampler, tex_uv);
    float4 tex_emissive = emissive_tex.Sample(material_sampler, tex_uv);
    float4 tex_opacity = opacity_tex.Sample(material_sampler, tex_uv);
    uint slot_flags = (uint)mat_params.z;
    float normal_length_sq = dot(input.normal, input.normal);

    // Grid / wireframe / normal lines carry a zero normal — they always render
    // their own vertex color. Their ambient output carries the overlay alpha with
    // zero color so post GTAO darkens only the still-visible mesh beneath them.
    if (normal_length_sq < 1e-6)
    {
        out_frag.color = float4(srgb_to_linear(input.color.rgb), input.color.a);
        out_frag.ambient = float4(0.0, 0.0, 0.0, input.color.a);
        return out_frag;
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
    float3 base_color = mat_base_color.rgb;
    if (has_base)
    {
        if (mat_channels0.x > 3.5)
        {
            base_color = base_color * tex_base.rgb;
        }
        else
        {
            base_color = base_color * select_channel(tex_base, mat_channels0.x);
        }
    }
    float out_alpha = mat_base_color.a;
    if (has_opacity)
    {
        out_alpha = out_alpha * select_channel(tex_opacity, mat_channels1.z);
    }
    if (uv_checker_enabled)
    {
        base_color = checker.rgb;
    }
    // The vertex-color view replaces the surface material (overriding the checker if
    // both are on). RGB is gamma-space like the material color; alpha is a raw 0..1
    // scalar shown as linear grey.
    if (vertex_color_mode >= 0.0)
    {
        if (vertex_color_mode < 0.5)
        {
            base_color = srgb_to_linear(input.color.rgb);
            out_alpha = 1.0;
        }
        else if (vertex_color_mode < 1.5)
        {
            base_color = (float3)input.color.a;
            out_alpha = 1.0;
        }
        else
        {
            base_color = srgb_to_linear(input.color.rgb);
            out_alpha = input.color.a;
        }
    }

    // Alpha-clip cutout (material flag): drop fully-transparent fragments before any
    // shading. Blend mode keeps the fragment and lets the MRT alpha-blend it.
    float alpha_mode = mat_params.w;
    if (alpha_mode > 1.5 && out_alpha < mat_channels1.w)
    {
        discard;
    }

    if (shading_mode < 1.5)
    {
        // Unlit: flat emissive material color.
        out_frag.color = float4(base_color, out_alpha);
        return out_frag;
    }

    // The shading normal: the geometric normal, optionally perturbed by a
    // tangent-space normal map.
    float3 world_normal = normalize(input.normal);
    if (has_normal)
    {
        world_normal = apply_normal_map(input.normal, input.tangent, tex_normal.rgb);
    }
    float3 n = world_normal;
    // Metallic + roughness from the uniform, modulated by their (channel-routed)
    // textures when bound.
    float metallic = mat_params.x;
    if (has_metallic)
    {
        metallic = metallic * select_channel(tex_metallic, mat_channels0.w);
    }
    metallic = clamp(metallic, 0.0, 1.0);
    float roughness_value = mat_params.y;
    if (has_roughness)
    {
        float sample_r = select_channel(tex_roughness, mat_channels0.z);
        if (mat_flags.x > 0.5)
        {
            // Smoothness workflow: combine in smoothness space (Unity-style),
            // final = slider * map, so a slider of 1 uses the map as authored.
            float smoothness_scalar = 1.0 - roughness_value;
            roughness_value = 1.0 - smoothness_scalar * sample_r;
        }
        else
        {
            // Roughness workflow: roughnessFactor * roughness_map (glTF convention).
            roughness_value = roughness_value * sample_r;
        }
    }
    // Ambient-occlusion factor (channel-routed), darkening only the ambient term.
    float ao = 1.0;
    if (has_ao)
    {
        ao = select_channel(tex_ao, mat_channels1.x);
    }

    float3 color_linear;
    float3 ambient_linear;
    if (env_params.x > 0.5)
    {
        // Image-based lighting (the default Shaded look). Roughness clamped away
        // from a perfect mirror so the lowest mip still reads as a surface.
        float roughness = clamp(roughness_value, 0.04, 1.0);
        ShadingResult shaded = shade_ibl(base_color, world_normal, input.world_position, roughness, metallic);
        color_linear = shaded.color;
        ambient_linear = shaded.ambient;
    }
    else
    {
        // Analytic fallback: the neutral-grey hemisphere + Blinn-Phong specular used
        // before IBL. Smoothness is the complement of the (textured) roughness.
        float smoothness = clamp(1.0 - roughness_value, 0.0, 1.0);
        float3 light_dir = normalize(float3(0.35, 0.82, 0.44));
        float diffuse = max(dot(n, light_dir), 0.0);
        float hemi_t = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
        float3 sky = float3(0.63, 0.63, 0.63);
        float3 ground = float3(0.11, 0.11, 0.11);
        float3 hemi = lerp(ground, sky, hemi_t);
        float3 ambient_light = hemi * 0.55 + float3(1.0, 1.0, 1.0) * 0.20;
        float3 direct_light = float3(1.0, 1.0, 1.0) * (diffuse * 0.75);
        float3 lighting = ambient_light + direct_light;
        float3 lit = base_color * lighting;

        float3 view_dir = normalize(camera_position.xyz - input.world_position);
        float3 half_dir = normalize(light_dir + view_dir);
        float shininess = exp2(1.0 + smoothness * 10.0);
        float spec = pow(max(dot(n, half_dir), 0.0), shininess) * smoothness * step(0.0, diffuse);
        color_linear = lit + (float3)spec;
        ambient_linear = base_color * ambient_light;
    }

    // Ambient occlusion darkens only the ambient term: remove the un-occluded
    // ambient from the lit color and add back the occluded fraction, leaving direct
    // + specular light untouched (matching the post GTAO compose).
    color_linear = color_linear + ambient_linear * (ao - 1.0);
    ambient_linear = ambient_linear * ao;

    // Emissive adds on top of the lit result (linear). Default emissive is black; an
    // emissive texture (when bound) modulates it.
    float3 emissive = mat_emissive.rgb;
    if (has_emissive)
    {
        float3 factor = mat_emissive.rgb;
        if (all(factor <= 0.0))
        {
            factor = (float3)1.0;
        }
        if (mat_channels1.y > 3.5)
        {
            emissive = tex_emissive.rgb * factor;
        }
        else
        {
            emissive = (float3)select_channel(tex_emissive, mat_channels1.y) * factor;
        }
    }
    color_linear = color_linear + emissive;

    out_frag.color = float4(color_linear, out_alpha);
    out_frag.ambient = float4(ambient_linear, out_alpha);
    return out_frag;
}

// Flat-color path for the grid + every line overlay (wireframe, bounding box,
// face/vertex normal lines). The vertices carry their color in `COLOR0`; lines
// sample no texture and no light. Mirrors `scene.wgsl`'s `fs_line`.
FragOutput fs_line(VsOutput input)
{
    FragOutput output;
    output.color = float4(srgb_to_linear(input.color.rgb), input.color.a);
    output.ambient = float4(0.0, 0.0, 0.0, input.color.a);
    return output;
}

// Selection flash: a flat color fill over the selected triangles (the solo index
// buffer redrawn over the mesh). Ignores the mesh's lighting / material — it emits
// the uniform highlight color (`selection_color.rgb`, gamma-space) tinted by the
// flash fade (`selection_color.a`). Writes zero ambient *color* with the fade alpha
// so it masks (rather than GTAO-darkens) the mesh ambient beneath, like the other
// overlays. Mirrors `scene.wgsl`'s `fs_selection`.
FragOutput fs_selection(VsOutput input)
{
    FragOutput output;
    float fade = selection_color.a;
    output.color = float4(srgb_to_linear(selection_color.rgb), fade);
    output.ambient = float4(0.0, 0.0, 0.0, fade);
    return output;
}

// --- GTAO G-buffer: a mesh-only pass writing the view-space normal (xyz) + the
// linear view-space Z (w) into a single-sample target the GTAO occlusion pass
// reads. Single-sample (no MSAA) so geometry-edge normals/depths aren't averaged
// by a resolve before the occlusion + bilateral blur read them. Uses `vs_main` and
// reads the `view` matrix from `b0`. Mirrors `scene.wgsl`'s `fs_gtao_gbuffer`.
float4 fs_gtao_gbuffer(VsOutput input) : SV_Target
{
    float normal_length_sq = dot(input.normal, input.normal);
    // Overlays / lines (zero normal) write a zero G-buffer entry → GTAO treats them
    // as background and applies no occlusion there.
    if (normal_length_sq < 1e-6)
    {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    // View Z is negative in front of the camera; GTAO treats zero as background.
    float4 view_pos = mul(view, float4(input.world_position, 1.0));
    float3 view_normal = normalize(mul(view, float4(input.normal, 0.0)).xyz);
    return float4(view_normal, view_pos.z);
}

// --- Skybox: draw the environment cubemap as the viewport background. -----------
// A fullscreen triangle whose per-pixel world ray direction is reconstructed from
// the inverse view-projection, sampled into the env cube. Drawn first in the
// offscreen pass (depth-test always, no depth write) so geometry composites over it.
struct SkyOutput
{
    float4 clip_position : SV_Position;
    float2 ndc           : TEXCOORD0;
};

SkyOutput vs_skybox(uint vertex_id : SV_VertexID)
{
    float2 positions[3] = {
        float2(-1.0, -1.0),
        float2( 3.0, -1.0),
        float2(-1.0,  3.0),
    };
    float2 xy = positions[vertex_id];
    SkyOutput output;
    // Reversed-Z far depth is 0; depth write is off and the pipeline always passes,
    // so this is only for completeness.
    output.clip_position = float4(xy, 0.0, 1.0);
    output.ndc = xy;
    return output;
}

FragOutput fs_skybox(SkyOutput input)
{
    // Perspective uses an infinite reversed projection, so unproject a finite
    // near-plane point (reversed-Z near depth = 1). Orthographic keeps a finite far
    // plane and unprojects at depth 0 to preserve the viewport-varying direction.
    float unproject_depth = (projection_params.x > 0.5) ? 0.0 : 1.0;
    float4 world = mul(inv_view_projection, float4(input.ndc, unproject_depth, 1.0));
    float3 world_pos = world.xyz / world.w;
    float3 dir = normalize(world_pos - camera_position.xyz);
    // Yaw-rotate the skybox lookup so the background spins with the IBL. Clamp the
    // intensity-scaled sample to f16 max so a bright sun saturates to white rather
    // than overflowing to inf/NaN in the HDR target.
    float3 raw = env_cube.SampleLevel(ibl_sampler, env_sample_dir(dir), 0.0).rgb * env_params.y;
    float3 color = min(raw, (float3)65504.0);
    FragOutput output;
    output.color = float4(color, 1.0);
    // The sky is background: no ambient target, so GTAO never darkens it.
    output.ambient = float4(0.0, 0.0, 0.0, 0.0);
    return output;
}
