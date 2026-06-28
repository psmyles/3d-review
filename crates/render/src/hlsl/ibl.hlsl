// Image-based-lighting precompute (HLSL / SM 5.0) — the Direct3D 11 port of
// `ibl.wgsl`. One file, four pixel entry points driven by the offline bake tool
// (`ibl.rs`, the `bake` feature):
//   * fs_equirect_to_cube — project the loaded equirectangular HDR onto the six
//     faces of an environment cubemap.
//   * fs_irradiance       — cosine-convolve the env cube into a low-res diffuse
//     irradiance cube.
//   * fs_prefilter        — GGX importance-sample the env cube into the mip chain
//     of a prefiltered specular cube (one roughness per mip).
//   * fs_brdf             — integrate the split-sum BRDF response into a 2D LUT.
// Every pass renders a fullscreen triangle (`vs_fullscreen`, no vertex buffer); the
// cube passes reconstruct the world-space sample direction from a per-face basis in
// `FaceUniform`. These run once per environment at bake time, never at runtime.
//
// Registers (see `rhi::bake` / `ibl.rs` binding): `FaceUniform` cbuffer `b0`
// (VS + PS), equirect source `t0`, env cube source `t1`, shared sampler `s0`.

static const float PI = 3.14159265359;

// Ceiling for sampled environment radiance in the convolution passes. The equirect
// upload is already clamped to finite f16 in `ibl.rs`, but a small, very bright
// source texel (a sun in a high-range HDR) still dominates the cosine / GGX
// integral, leaving fireflies in the diffuse irradiance and the specular mips.
// Clamping the per-sample radiance tames those highlights so the lighting stays
// balanced across environments.
static const float IBL_RADIANCE_CLAMP = 64.0;

// Per-face / per-pass parameters. `forward/right/up` are the cube face's basis:
// a fullscreen-triangle position (clip xy in -1..1) maps to the world direction
// `forward + x*right + y*up`. `params.x` carries the prefilter roughness.
cbuffer FaceUniform : register(b0)
{
    float4 forward;
    float4 right;
    float4 up;
    float4 params;
};

Texture2D    src_equirect : register(t0);
TextureCube  src_cube     : register(t1);
SamplerState src_sampler  : register(s0);

struct VsOutput
{
    float4 clip_position : SV_Position;
    // World-space direction for this fragment (cube passes).
    float3 local_dir : TEXCOORD0;
    // 0..1 screen coordinate (BRDF LUT pass): x = N·V, y = roughness.
    float2 uv : TEXCOORD1;
};

VsOutput vs_fullscreen(uint index : SV_VertexID)
{
    // A single oversized triangle covering the framebuffer.
    float2 corners[3] = {
        float2(-1.0, -1.0),
        float2(3.0, -1.0),
        float2(-1.0, 3.0),
    };
    float2 xy = corners[index];

    VsOutput output;
    output.clip_position = float4(xy, 0.0, 1.0);
    output.local_dir = forward.xyz + xy.x * right.xyz + xy.y * up.xyz;
    // Flip Y so texel row t (top = 0) maps to roughness = t for the BRDF LUT.
    output.uv = float2(xy.x * 0.5 + 0.5, 1.0 - (xy.y * 0.5 + 0.5));
    return output;
}

// Map a unit direction to equirectangular UV: u around the equator, v from the
// top (+Y) down. Matches the orientation the HDR is uploaded with (row 0 = top).
float2 dir_to_equirect_uv(float3 dir)
{
    float3 n = normalize(dir);
    float u = atan2(n.z, n.x) / (2.0 * PI) + 0.5;
    float v = acos(clamp(n.y, -1.0, 1.0)) / PI;
    return float2(u, v);
}

float4 fs_equirect_to_cube(VsOutput input) : SV_Target
{
    float2 uv = dir_to_equirect_uv(input.local_dir);
    float3 color = src_equirect.SampleLevel(src_sampler, uv, 0.0).rgb;
    return float4(color, 1.0);
}

float4 fs_irradiance(VsOutput input) : SV_Target
{
    float3 normal = normalize(input.local_dir);

    // Build a tangent frame around the surface normal.
    float3 up_axis = float3(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999)
    {
        up_axis = float3(0.0, 0.0, 1.0);
    }
    float3 tangent = normalize(cross(up_axis, normal));
    float3 bitangent = cross(normal, tangent);

    float3 irradiance = float3(0.0, 0.0, 0.0);
    float samples = 0.0;
    const float sample_delta = 0.05;
    [loop]
    for (float phi = 0.0; phi < 2.0 * PI; phi += sample_delta)
    {
        [loop]
        for (float theta = 0.0; theta < 0.5 * PI; theta += sample_delta)
        {
            // Spherical -> tangent-space -> world.
            float3 tangent_sample = float3(
                sin(theta) * cos(phi),
                sin(theta) * sin(phi),
                cos(theta));
            float3 sample_vec = tangent_sample.x * tangent
                + tangent_sample.y * bitangent
                + tangent_sample.z * normal;
            // Clamp the sampled radiance (see IBL_RADIANCE_CLAMP): an HDR sun
            // otherwise dominates the cosine integral and leaves fireflies / a
            // blown-out diffuse term.
            float3 radiance = min(
                src_cube.SampleLevel(src_sampler, sample_vec, 0.0).rgb,
                (float3)IBL_RADIANCE_CLAMP);
            irradiance += radiance * cos(theta) * sin(theta);
            samples += 1.0;
        }
    }
    irradiance = PI * irradiance / max(samples, 1.0);
    return float4(irradiance, 1.0);
}

// Van der Corput / Hammersley low-discrepancy sequence for importance sampling.
float radical_inverse_vdc(uint bits)
{
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10;
}

float2 hammersley(uint i, uint n)
{
    return float2(float(i) / float(n), radical_inverse_vdc(i));
}

// Sample a GGX half-vector around `n` for a given roughness, from a 2D random.
float3 importance_sample_ggx(float2 xi, float3 n, float roughness)
{
    float a = roughness * roughness;
    float phi = 2.0 * PI * xi.x;
    float cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    float sin_theta = sqrt(1.0 - cos_theta * cos_theta);

    float3 h = float3(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);

    float3 up_axis = float3(0.0, 0.0, 1.0);
    if (abs(n.z) > 0.999)
    {
        up_axis = float3(1.0, 0.0, 0.0);
    }
    float3 tangent = normalize(cross(up_axis, n));
    float3 bitangent = cross(n, tangent);
    return normalize(tangent * h.x + bitangent * h.y + n * h.z);
}

float4 fs_prefilter(VsOutput input) : SV_Target
{
    float3 normal = normalize(input.local_dir);
    float3 view_dir = normal;
    float roughness = params.x;

    uint sample_count = 64u;
    float3 prefiltered = float3(0.0, 0.0, 0.0);
    float total_weight = 0.0;
    [loop]
    for (uint i = 0u; i < sample_count; i = i + 1u)
    {
        float2 xi = hammersley(i, sample_count);
        float3 h = importance_sample_ggx(xi, normal, roughness);
        float3 l = normalize(2.0 * dot(view_dir, h) * h - view_dir);
        float n_dot_l = max(dot(normal, l), 0.0);
        if (n_dot_l > 0.0)
        {
            // Clamp the sampled radiance (see IBL_RADIANCE_CLAMP): importance
            // sampling a tiny, very bright texel otherwise leaves sparkle
            // (fireflies) in the prefiltered mip.
            float3 radiance = min(
                src_cube.SampleLevel(src_sampler, l, 0.0).rgb,
                (float3)IBL_RADIANCE_CLAMP);
            prefiltered += radiance * n_dot_l;
            total_weight += n_dot_l;
        }
    }
    if (total_weight > 0.0)
    {
        prefiltered = prefiltered / total_weight;
    }
    return float4(prefiltered, 1.0);
}

// Smith geometry term for IBL (k uses the IBL remap, not the direct-lighting one).
float geometry_schlick_ggx(float n_dot_v, float roughness)
{
    float k = (roughness * roughness) / 2.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

float geometry_smith(float n_dot_v, float n_dot_l, float roughness)
{
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

float2 fs_brdf(VsOutput input) : SV_Target
{
    float n_dot_v = max(input.uv.x, 1e-4);
    float roughness = input.uv.y;

    float3 view = float3(sqrt(1.0 - n_dot_v * n_dot_v), 0.0, n_dot_v);
    float3 normal = float3(0.0, 0.0, 1.0);

    float a = 0.0;
    float b = 0.0;
    uint sample_count = 512u;
    [loop]
    for (uint i = 0u; i < sample_count; i = i + 1u)
    {
        float2 xi = hammersley(i, sample_count);
        float3 h = importance_sample_ggx(xi, normal, roughness);
        float3 l = normalize(2.0 * dot(view, h) * h - view);

        float n_dot_l = max(l.z, 0.0);
        float n_dot_h = max(h.z, 0.0);
        float v_dot_h = max(dot(view, h), 0.0);
        if (n_dot_l > 0.0)
        {
            float g = geometry_smith(n_dot_v, n_dot_l, roughness);
            float g_vis = (g * v_dot_h) / (n_dot_h * n_dot_v);
            // max() guards fxc's negative-base pow warning (/WX); 1 - v_dot_h is
            // already non-negative (v_dot_h is clamped to [0, 1]).
            float fc = pow(max(1.0 - v_dot_h, 0.0), 5.0);
            a += (1.0 - fc) * g_vis;
            b += fc * g_vis;
        }
    }
    return float2(a, b) / float(sample_count);
}
