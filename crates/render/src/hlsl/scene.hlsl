// Scene shader (HLSL / Shader Model 5.0) — the Direct3D 11 port of `scene.wgsl`.
// Migration note: the WGSL file remains in-tree as the golden reference while the
// scene path is ported pass by pass; this HLSL is compiled offline to DXBC by
// `fxc` (see `build.rs`) and shipped as bytecode (no runtime shader compilation).
//
// Phase 1 covers only `vs_main` + `fs_line` (the grid + line overlays). The full
// `fs_main` PBR/IBL path, the skybox, the GTAO G-buffer and the selection fill
// land in later phases — translated from the matching `scene.wgsl` entries.
//
// Conventions that must hold (see the migration plan's HLSL guardrails):
//  * The `SceneUniforms` cbuffer below is byte-for-byte the `#[repr(C)]`
//    `SceneUniforms` in `scene/gpu_types.rs` (invariant 11). Every field is a
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

// Scene fragment output (MRT): location 0 is the linear-HDR color the composite
// tone-maps; location 1 is the linear ambient radiance GTAO may attenuate (Phase
// 3). Overlays/lines write 0 ambient *color* with the overlay alpha so they never
// bloom or get AO-darkened. Mirrors `scene.wgsl`'s `FragOutput`.
struct FragOutput
{
    float4 color   : SV_Target0;
    float4 ambient : SV_Target1;
};

// Flat-color path for the grid + every line overlay (wireframe, bounding box,
// face/vertex normal lines). The vertices carry their color in the `COLOR0`
// channel; lines sample no texture and no light. Writes the authored gamma color
// decoded to linear into the HDR MRT; the post pass encodes sRGB. Mirrors
// `scene.wgsl`'s `fs_line`.
FragOutput fs_line(VsOutput input)
{
    FragOutput output;
    output.color = float4(srgb_to_linear(input.color.rgb), input.color.a);
    output.ambient = float4(0.0, 0.0, 0.0, input.color.a);
    return output;
}

// Phase 2a mesh path: the analytic neutral-hemisphere + Blinn-Phong lighting from
// `scene.wgsl`'s IBL-disabled fallback, with a fixed neutral base color and
// roughness. This makes a loaded model appear as a lit solid and exercises the
// offscreen-MRT + composite seam + the mesh pipeline (triangle list, back-face
// cull, depth write + bias). Phase 2b replaces this with the full `fs_main`
// (per-material PBR, IBL, textures, checker, vertex-color, unlit, alpha) reading
// the material cbuffer + texture/IBL SRVs.
FragOutput fs_mesh(VsOutput input)
{
    const float3 base_color = float3(0.8, 0.8, 0.8);
    const float roughness_value = 0.5;

    float3 n = normalize(input.normal);
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

    FragOutput output;
    output.color = float4(lit + float3(spec, spec, spec), 1.0);
    output.ambient = float4(base_color * ambient_light, 1.0);
    return output;
}
