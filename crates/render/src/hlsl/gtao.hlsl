// Ground-Truth Ambient Occlusion (HLSL / SM 5.0) — the Direct3D 11 port of
// `gtao.wgsl`. Two fullscreen passes sharing `vs_fullscreen`:
//   * fs_gtao — horizon-based slice integration over a single-sample view-space
//               normal + depth G-buffer into an R8 visibility target.
//   * fs_blur — a 5x5 bilateral blur that removes the per-pixel-rotation noise
//               while preserving depth/normal edges.
// The composite (`post.hlsl`) applies the blurred AO only to the ambient radiance.
//
// GTAO works in view space: the G-buffer stores the view-space normal (xyz) and
// the linear view-space Z (w, negative in front of the camera). For each slice
// direction the shader marches the screen-space horizon both ways, then integrates
// the cosine-weighted visible arc (the GTAO ground-truth integral), averaging over
// slices.
//
// Registers: gbuffer `t0`, raw AO `t1`, GTAO sampler `s0`, `GtaoUniforms` `b0`.
//
// Matrix note: `proj` is uploaded column-major and read with HLSL's default
// `column_major` packing, so `mul(proj, v)` is the standard projection and an
// element `M[row][col]` is read `proj[row][col]` (WGSL's column-indexed
// `proj[col][row]` becomes `proj[row][col]` here).

static const float PI = 3.14159265359;
static const float HALF_PI = 1.57079632679;

cbuffer GtaoUniforms : register(b0)
{
    // Projection matrix (view -> clip): projects sample points to screen and its
    // terms reconstruct view-space position from depth.
    float4x4 proj;
    // x = sample radius (view units), y = intensity (power on visibility),
    // z = thickness heuristic (0..1), w = unused.
    float4 params;
    // x = 1.0 when the projection is orthographic, else 0.0; y = slice count;
    // z = steps per slice; w = unused.
    float4 config;
};

Texture2D    gbuffer       : register(t0);
Texture2D    raw_ao        : register(t1);
SamplerState gtao_sampler  : register(s0);

struct VsOutput
{
    float4 clip_position : SV_Position;
    float2 uv : TEXCOORD0;
};

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

// Reconstruct view-space position from a pixel's uv + its stored view-space Z.
float3 reconstruct_view_pos(float2 uv, float view_z)
{
    float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    if (config.x > 0.5)
    {
        // Orthographic: ndc = P00*x + P03 (x = (ndc - P03) / P00); Z is independent.
        float x = (ndc.x - proj[0][3]) / proj[0][0];
        float y = (ndc.y - proj[1][3]) / proj[1][1];
        return float3(x, y, view_z);
    }
    // Perspective: ndc.x = P00 * x / (-view_z)  =>  x = ndc.x * (-view_z) / P00.
    float x = ndc.x * (-view_z) / proj[0][0];
    float y = ndc.y * (-view_z) / proj[1][1];
    return float3(x, y, view_z);
}

// Project a view-space point to texture uv via the projection matrix.
float2 project_to_uv(float3 view_pos)
{
    float4 clip = mul(proj, float4(view_pos, 1.0));
    float2 ndc = clip.xy;
    if (config.x <= 0.5)
    {
        ndc = clip.xy / clip.w;
    }
    return float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// Jimenez 2016 (Activision GTAO) structured 4x4 spatial dither. Returns the slice
// rotation in x and the step offset in y, both in [0,1). Adjacent pixels get
// evenly spread rotations/offsets, so the bilateral denoiser averages complementary
// slice directions over a small block and resolves to a clean result at low sample
// counts instead of leaving high-variance speckle.
float2 spatial_dither(uint2 pix)
{
    float rot = (1.0 / 16.0) * float((((pix.x + pix.y) & 3u) << 2u) + (pix.x & 3u));
    float offset = (1.0 / 4.0) * float((pix.y - pix.x) & 3u);
    return float2(rot, offset);
}

// Reconstruct the view-space position at `uv` (xyz) + a foreground flag (w): 0 for
// background (overlays / skybox / cleared frame, which wrote a zero normal / Z).
float4 sample_view_pos(float2 uv)
{
    float4 g = gbuffer.SampleLevel(gtao_sampler, uv, 0.0);
    if (dot(g.xyz, g.xyz) < 0.25 || g.w >= -1e-4)
    {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    return float4(reconstruct_view_pos(uv, g.w), 1.0);
}

// March one side of a slice for the maximum horizon cosine (dot of the direction
// to the closest occluder with the view vector). `dir_px` is the per-step screen
// offset in pixels for that side; `inv_dims` converts pixels to uv.
float horizon_cos(
    float2 origin_uv,
    float3 p,
    float3 v,
    float2 dir_px,
    float radius_px,
    float radius,
    float thickness,
    uint steps,
    float jitter,
    float2 inv_dims)
{
    float cos_h = -1.0;
    [loop]
    for (uint t = 1u; t <= steps; t = t + 1u)
    {
        float frac = (float(t) - jitter) / float(steps);
        float2 uv = origin_uv + dir_px * (radius_px * frac) * inv_dims;
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0)
        {
            break;
        }
        float4 s = sample_view_pos(uv);
        if (s.w < 0.5)
        {
            continue;
        }
        float3 d = s.xyz - p;
        float len = length(d);
        if (len < 1e-5)
        {
            continue;
        }
        float c = dot(d, v) / len;
        // Radius falloff: ignore occluders past the radius; far ones inside the
        // radius are pulled back by the thickness heuristic so thin objects don't
        // over-occlude.
        float w = clamp(1.0 - len / radius, 0.0, 1.0);
        if (w <= 0.0)
        {
            continue;
        }
        cos_h = max(cos_h, c - (1.0 - w) * thickness);
    }
    return cos_h;
}

float fs_gtao(VsOutput input) : SV_Target
{
    float4 g = gbuffer.SampleLevel(gtao_sampler, input.uv, 0.0);
    float3 raw_normal = g.xyz;
    float view_z = g.w;
    // Background (overlays / skybox / cleared frame write 0): no occlusion.
    if (dot(raw_normal, raw_normal) < 0.25 || view_z >= -1e-4)
    {
        return 1.0;
    }

    float3 n = normalize(raw_normal);
    float3 p = reconstruct_view_pos(input.uv, view_z);
    float3 v = normalize(-p);

    float radius = params.x;
    float intensity = params.y;
    float thickness = clamp(params.z, 0.0, 1.0);
    uint slice_count = max((uint)config.y, 1u);
    uint step_count = max((uint)config.z, 1u);

    float dim_w, dim_h;
    gbuffer.GetDimensions(dim_w, dim_h);
    float2 dims = float2(dim_w, dim_h);
    float2 inv_dims = 1.0 / dims;
    // Screen radius (px): UV span of `radius` view units along view X at this depth.
    float2 edge_uv = project_to_uv(p + float3(radius, 0.0, 0.0));
    float radius_px = clamp(abs(edge_uv.x - input.uv.x) * dims.x, 1.0, max(dims.x, dims.y));

    // Structured per-pixel rotation + step offset so slices/steps decorrelate over a
    // 4x4 block (the blur then resolves them to a clean result — see spatial_dither).
    float2 dither = spatial_dither(uint2((uint)input.clip_position.x, (uint)input.clip_position.y));
    float rot_noise = dither.x;
    float offset_noise = dither.y;

    float visibility = 0.0;
    [loop]
    for (uint s = 0u; s < slice_count; s = s + 1u)
    {
        float phi = (float(s) + rot_noise) * PI / float(slice_count);
        float2 omega = float2(cos(phi), sin(phi));

        // View-space slice direction: reconstruct a neighbor a few px along omega.
        float4 neighbor = sample_view_pos(input.uv + omega * (2.0 * inv_dims));
        float3 slice_dir = normalize(float3(omega.x, omega.y, 0.0));
        if (neighbor.w > 0.5)
        {
            float3 d = neighbor.xyz - p;
            if (dot(d, d) > 1e-12)
            {
                slice_dir = normalize(d);
            }
        }

        // Project the normal onto the slice plane (spanned by v and slice_dir).
        float3 plane_normal = normalize(cross(v, slice_dir));
        float3 proj_n = n - plane_normal * dot(n, plane_normal);
        float proj_len = length(proj_n);
        if (proj_len < 1e-4)
        {
            continue;
        }
        float3 proj_n_dir = proj_n / proj_len;
        // Signed angle of the projected normal from the view vector in the plane.
        float sign_n = sign(dot(cross(slice_dir, proj_n_dir), plane_normal));
        float gamma = sign_n * acos(clamp(dot(proj_n_dir, v), -1.0, 1.0));

        // Search both horizons (positive omega and negative omega side).
        float cos_pos = horizon_cos(
            input.uv, p, v, omega, radius_px, radius, thickness, step_count, offset_noise, inv_dims);
        float cos_neg = horizon_cos(
            input.uv, p, v, -omega, radius_px, radius, thickness, step_count, offset_noise, inv_dims);

        // Clamp horizons into the hemisphere around the (projected) normal, then
        // integrate the visible cosine-weighted arc (GTAO inner integral).
        float h1 = gamma + max(-acos(clamp(cos_neg, -1.0, 1.0)) - gamma, -HALF_PI);
        float h2 = gamma + min(acos(clamp(cos_pos, -1.0, 1.0)) - gamma, HALF_PI);
        float cos_gamma = cos(gamma);
        float sin_gamma = sin(gamma);
        float arc = 0.25 * (
            (-cos(2.0 * h1 - gamma) + cos_gamma + 2.0 * h1 * sin_gamma)
            + (-cos(2.0 * h2 - gamma) + cos_gamma + 2.0 * h2 * sin_gamma));
        visibility = visibility + proj_len * arc;
    }

    visibility = clamp(visibility / float(slice_count), 0.0, 1.0);
    // Intensity sharpens the falloff (1 = ground truth, >1 darkens, 0 disables).
    // max() guards fxc's negative-base pow warning (/WX); visibility is already >= 0.
    return pow(max(visibility, 0.0), max(intensity, 0.0));
}

// 5x5 bilateral blur over the raw AO. Depth and normal weights keep occlusion
// from bleeding across silhouettes and hard creases.
float fs_blur(VsOutput input) : SV_Target
{
    float4 center_g = gbuffer.SampleLevel(gtao_sampler, input.uv, 0.0);
    float3 center_n = center_g.xyz;
    float center_z = center_g.w;
    if (dot(center_n, center_n) < 0.25 || center_z >= -1e-4)
    {
        return 1.0;
    }

    float dim_w, dim_h;
    raw_ao.GetDimensions(dim_w, dim_h);
    float2 dims = float2(dim_w, dim_h);
    float2 texel = 1.0 / dims;
    float3 n0 = normalize(center_n);
    float depth_sigma = max(params.x * 0.12, 1e-4);
    float spatial_sigma = 2.0;
    float sum = 0.0;
    float weight_sum = 0.0;
    for (int x = -2; x <= 2; x = x + 1)
    {
        for (int y = -2; y <= 2; y = y + 1)
        {
            float2 pixel_offset = float2(float(x), float(y));
            float2 uv = input.uv + pixel_offset * texel;
            float4 g = gbuffer.SampleLevel(gtao_sampler, uv, 0.0);
            float n_len = dot(g.xyz, g.xyz);
            if (n_len < 0.25 || g.w >= -1e-4)
            {
                continue;
            }

            float normal_dot = dot(n0, normalize(g.xyz));
            if (normal_dot < 0.75)
            {
                continue;
            }
            float spatial_weight = exp(-dot(pixel_offset, pixel_offset) / (2.0 * spatial_sigma * spatial_sigma));
            float dz = abs(g.w - center_z);
            float depth_weight = exp(-(dz * dz) / (2.0 * depth_sigma * depth_sigma));
            float normal_weight = smoothstep(0.75, 1.0, normal_dot);
            float weight = spatial_weight * depth_weight * normal_weight;
            float ao = raw_ao.SampleLevel(gtao_sampler, uv, 0.0).r;
            sum = sum + ao * weight;
            weight_sum = weight_sum + weight;
        }
    }
    if (weight_sum <= 1e-5)
    {
        return raw_ao.SampleLevel(gtao_sampler, input.uv, 0.0).r;
    }
    return sum / weight_sum;
}
