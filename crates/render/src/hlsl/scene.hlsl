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

// Flat-color path for the grid + every line overlay (wireframe, bounding box,
// face/vertex normal lines). The vertices carry their color in the `COLOR0`
// channel; lines sample no texture and no light.
//
// Phase 1 renders the scene straight to the gamma (non-sRGB) backbuffer, so the
// authored gamma-space line colors are emitted directly. Once the offscreen
// linear-HDR target + the post composite land (Phase 2/3), this returns to
// writing linear color (`srgb_to_linear`) into the HDR MRT and the post pass does
// the sRGB encode — mirroring `scene.wgsl`'s `fs_line`.
float4 fs_line(VsOutput input) : SV_Target
{
    return input.color;
}
