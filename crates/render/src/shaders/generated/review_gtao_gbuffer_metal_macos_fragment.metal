#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct material
{
    float4 mat_base_color;
    float4 mat_emissive;
    float4 mat_params;
    float4 mat_channels0;
    float4 mat_channels1;
    float4 mat_flags;
};

struct scene_fs
{
    float4x4 view_projection;
    float4x4 inv_view_projection;
    float4 render_options;
    float4 camera_position;
    float4 env_params;
    float4 projection_params;
    float4x4 view;
    float4 selection_color;
};

struct main0_out
{
    float4 frag_gbuffer [[color(0)]];
    float frag_depth [[color(1)]];
};

struct main0_in
{
    float3 v_normal [[user(locn0)]];
    float2 v_uv [[user(locn1)]];
    float4 v_color [[user(locn2)]];
    float3 v_world_position [[user(locn3)]];
};

static inline __attribute__((always_inline))
float2 material_uv(thread const float2& uv)
{
    return float2(uv.x, 1.0 - uv.y);
}

static inline __attribute__((always_inline))
float select_channel(thread const float4& texel, thread const float& index)
{
    if (index < 0.5)
    {
        return texel.x;
    }
    if (index < 1.5)
    {
        return texel.y;
    }
    if (index < 2.5)
    {
        return texel.z;
    }
    return texel.w;
}

static inline __attribute__((always_inline))
float material_alpha(thread const float4& tex_opacity, constant material& _83)
{
    float alpha = _83.mat_base_color.w;
    if ((uint(_83.mat_params.z) & 64u) != 0u)
    {
        float4 param = tex_opacity;
        float param_1 = _83.mat_channels1.z;
        alpha *= select_channel(param, param_1);
    }
    return alpha;
}

static inline __attribute__((always_inline))
float vertex_color_alpha(thread const float& alpha, constant scene_fs& sc, thread float4& v_color)
{
    float result = alpha;
    if (sc.render_options.w >= 0.0)
    {
        float _126;
        if (sc.render_options.w < 1.5)
        {
            _126 = 1.0;
        }
        else
        {
            _126 = v_color.w;
        }
        result = _126;
    }
    return result;
}

static inline __attribute__((always_inline))
bool alpha_clipped(thread const float& alpha, constant material& _83)
{
    bool _141 = _83.mat_params.w > 1.5;
    bool _148;
    if (_141)
    {
        _148 = alpha < _83.mat_channels1.w;
    }
    else
    {
        _148 = _141;
    }
    return _148;
}

static inline __attribute__((always_inline))
bool surface_cut_out(thread const float4& tex_opacity, constant material& _83, constant scene_fs& sc, thread float4& v_color, thread float3& v_normal)
{
    bool _160 = dot(v_normal, v_normal) >= 9.9999999747524270787835121154785e-07;
    bool _167;
    if (_160)
    {
        _167 = sc.projection_params.z < 0.0;
    }
    else
    {
        _167 = _160;
    }
    bool _173;
    if (_167)
    {
        _173 = sc.projection_params.w <= 0.5;
    }
    else
    {
        _173 = _167;
    }
    bool _184;
    if (_173)
    {
        float4 param = tex_opacity;
        float param_1 = material_alpha(param, _83);
        float param_2 = vertex_color_alpha(param_1, sc, v_color);
        _184 = alpha_clipped(param_2, _83);
    }
    else
    {
        _184 = _173;
    }
    return _184;
}

fragment main0_out main0(main0_in in [[stage_in]], constant scene_fs& sc [[buffer(1)]], constant material& _83 [[buffer(2)]], texture2d<float> opacity_tex [[texture(0)]], sampler material_sampler [[sampler(2)]])
{
    main0_out out = {};
    float2 param = in.v_uv;
    float4 param_1 = opacity_tex.sample(material_sampler, material_uv(param));
    if (surface_cut_out(param_1, _83, sc, in.v_color, in.v_normal))
    {
        discard_fragment();
    }
    if (dot(in.v_normal, in.v_normal) < 9.9999999747524270787835121154785e-07)
    {
        out.frag_gbuffer = float4(0.0);
        out.frag_depth = 1000000015047466219876688855040.0;
        return out;
    }
    float4 _236 = sc.view * float4(in.v_world_position, 1.0);
    float _251 = _236.z;
    out.frag_gbuffer = float4(fast::normalize((sc.view * float4(in.v_normal, 0.0)).xyz), _251);
    out.frag_depth = -_251;
    return out;
}

