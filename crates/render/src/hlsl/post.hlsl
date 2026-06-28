// Post / composite pass (HLSL / SM 5.0) — the Direct3D 11 port of `post.wgsl`.
// Samples the offscreen linear-HDR scene targets, applies ambient-only GTAO
// (Phase 3), tone-maps, encodes to sRGB, and writes the final LDR color to the
// swapchain backbuffer behind the egui chrome.
//
// Registers (see `rhi`/`scene::d3d` binding): scene color `t0`, blurred GTAO `t1`,
// ambient radiance `t2`, shared sampler `s0`, `PostUniforms` cbuffer `b0`.

Texture2D scene_color : register(t0);
Texture2D gtao_texture : register(t1);   // blurred GTAO occlusion (R8); Phase 3
Texture2D ambient_texture : register(t2); // linear-HDR ambient radiance; Phase 3
SamplerState scene_sampler : register(s0);

cbuffer PostUniforms : register(b0)
{
    uint gtao_enabled;
    uint tonemap_enabled;
    uint tonemap_op;
};

struct VsOutput
{
    float4 clip_position : SV_Position;
    float2 uv : TEXCOORD0;
};

// Single oversized triangle covering the framebuffer (the fullscreen-triangle
// trick — no vertex buffer). Mirrors `post.wgsl`'s `vs_main`.
VsOutput vs_fullscreen(uint index : SV_VertexID)
{
    float2 corners[3] = {
        float2(-1.0, -1.0),
        float2(3.0, -1.0),
        float2(-1.0, 3.0),
    };
    float2 corner = corners[index];
    VsOutput output;
    output.clip_position = float4(corner, 0.0, 1.0);
    float2 uv = corner * 0.5 + float2(0.5, 0.5);
    uv.y = 1.0 - uv.y; // texture origin is top-left
    output.uv = uv;
    return output;
}

float3 linear_to_srgb(float3 c)
{
    float3 lo = c * 12.92;
    // max() guards fxc's negative-base pow warning (/WX); the sRGB encode is only
    // meaningful for non-negative radiance anyway.
    float3 hi = 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
    float3 cutoff = step(c, 0.0031308); // 1 where c <= 0.0031308
    return lerp(hi, lo, cutoff);
}

// Build a column-major float3x3 from its three column vectors, so the WGSL
// `mat3x3(col0, col1, col2)` + `M * v` ports verbatim as `mul(M, v)` (HLSL's
// float3x3 literal constructor is row-major, hence this helper).
float3x3 mat_from_cols(float3 c0, float3 c1, float3 c2)
{
    return float3x3(
        c0.x, c1.x, c2.x,
        c0.y, c1.y, c2.y,
        c0.z, c1.z, c2.z);
}

// Khronos PBR Neutral tone mapping (Rec.709 linear). Ported from `post.wgsl`.
float3 pbr_neutral_tonemap(float3 color_in)
{
    const float start_compression = 0.8 - 0.04;
    const float desaturation = 0.15;
    float3 color = color_in;
    float x = min(color.r, min(color.g, color.b));
    float offset = (x < 0.08) ? (x - 6.25 * x * x) : 0.04;
    color = color - offset;
    float peak = max(color.r, max(color.g, color.b));
    if (peak < start_compression)
    {
        return color;
    }
    float d = 1.0 - start_compression;
    float new_peak = 1.0 - d * d / (peak + d - start_compression);
    color = color * (new_peak / peak);
    float g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return lerp(color, new_peak * float3(1.0, 1.0, 1.0), g);
}

float3 reinhard_tonemap(float3 color)
{
    return color / (float3(1.0, 1.0, 1.0) + color);
}

float3 aces_rrt_odt_fit(float3 v)
{
    float3 a = v * (v + 0.0245786) - 0.000090537;
    float3 b = v * (0.983729 * v + 0.432951) + 0.238081;
    return a / b;
}

float3 aces_tonemap(float3 color_in)
{
    float3x3 input_mat = mat_from_cols(
        float3(0.59719, 0.07600, 0.02840),
        float3(0.35458, 0.90834, 0.13383),
        float3(0.04823, 0.01566, 0.83777));
    float3x3 output_mat = mat_from_cols(
        float3(1.60475, -0.10208, -0.00327),
        float3(-0.53108, 1.10813, -0.07276),
        float3(-0.07367, -0.00605, 1.07602));
    float3 color = color_in / 0.6;
    color = mul(input_mat, color);
    color = aces_rrt_odt_fit(color);
    color = mul(output_mat, color);
    return clamp(color, 0.0, 1.0);
}

float3 agx_contrast_approx(float3 x)
{
    float3 x2 = x * x;
    float3 x4 = x2 * x2;
    return 15.5 * x4 * x2
        - 40.14 * x4 * x
        + 31.96 * x4
        - 6.868 * x2 * x
        + 0.4298 * x2
        + 0.1191 * x
        - 0.00232;
}

float3 agx_tonemap(float3 color_in)
{
    float3x3 srgb_to_rec2020 = mat_from_cols(
        float3(0.6274, 0.0691, 0.0164),
        float3(0.3293, 0.9195, 0.0880),
        float3(0.0433, 0.0113, 0.8956));
    float3x3 rec2020_to_srgb = mat_from_cols(
        float3(1.6605, -0.1246, -0.0182),
        float3(-0.5876, 1.1329, -0.1006),
        float3(-0.0728, -0.0083, 1.1187));
    float3x3 inset = mat_from_cols(
        float3(0.856627153315983, 0.137318972929847, 0.11189821299995),
        float3(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        float3(0.0482516061458583, 0.101439036467562, 0.811302368396859));
    float3x3 outset = mat_from_cols(
        float3(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        float3(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
        float3(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405));
    const float min_ev = -12.47393;
    const float max_ev = 4.026069;

    float3 color = mul(srgb_to_rec2020, color_in);
    color = mul(inset, color);
    color = max(color, 1e-10);
    color = log2(color);
    color = (color - min_ev) / (max_ev - min_ev);
    color = clamp(color, 0.0, 1.0);
    color = agx_contrast_approx(color);
    color = mul(outset, color);
    color = pow(max(color, 0.0), 2.2);
    color = mul(rec2020_to_srgb, color);
    return clamp(color, 0.0, 1.0);
}

// Select the operator (indices match `TonemapOperator::shader_index`). When tone
// mapping is off the linear radiance passes straight to the sRGB encode.
float3 apply_tonemap(float3 color)
{
    if (tonemap_enabled == 0u)
    {
        return color;
    }
    switch (tonemap_op)
    {
        case 0u: return pbr_neutral_tonemap(color);
        case 1u: return color;
        case 2u: return reinhard_tonemap(color);
        case 3u: return aces_tonemap(color);
        case 4u: return agx_tonemap(color);
        default: return pbr_neutral_tonemap(color);
    }
}

float4 fs_post(VsOutput input) : SV_Target
{
    float3 lit = scene_color.Sample(scene_sampler, input.uv).rgb;
    if (gtao_enabled != 0u)
    {
        float ao = gtao_texture.Sample(scene_sampler, input.uv).r;
        float3 ambient = ambient_texture.Sample(scene_sampler, input.uv).rgb;
        lit = max(lit - ambient * (1.0 - ao), 0.0);
    }
    return float4(linear_to_srgb(apply_tonemap(lit)), 1.0);
}
