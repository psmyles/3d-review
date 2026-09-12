#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct gtao_params
{
    float4x4 proj;
    float4 params;
    float4 config;
    float4 temporal;
};

struct main0_out
{
    float frag_ao [[color(0)]];
};

struct main0_in
{
    float2 v_uv [[user(locn0)]];
};

static inline __attribute__((always_inline))
float2 target_dims(constant gtao_params& _27)
{
    return float2(fast::max(_27.params.w, 1.0), fast::max(_27.config.w, 1.0));
}

static inline __attribute__((always_inline))
float3 reconstruct_view_pos(thread const float2& uv, thread const float& view_z, constant gtao_params& _27)
{
    float _50 = (uv.x * 2.0) - 1.0;
    float _55 = 1.0 - (uv.y * 2.0);
    if (_27.config.x > 0.5)
    {
        return float3((_50 - _27.proj[3].x) / _27.proj[0].x, (_55 - _27.proj[3].y) / _27.proj[1].y, view_z);
    }
    return float3((_50 * (-view_z)) / _27.proj[0].x, (_55 * (-view_z)) / _27.proj[1].y, view_z);
}

static inline __attribute__((always_inline))
float view_pixel_size(thread const float& view_z, constant gtao_params& _27)
{
    float _121;
    if (_27.config.x > 0.5)
    {
        _121 = 2.0;
    }
    else
    {
        _121 = 2.0 * abs(view_z);
    }
    return _121 / (fast::max(abs(_27.proj[1].y), 9.9999999747524270787835121154785e-07) * fast::max(_27.config.w, 1.0));
}

fragment main0_out main0(main0_in in [[stage_in]], constant gtao_params& _27 [[buffer(0)]], texture2d<float> gbuffer [[texture(0)]], texture2d<float> ao_in [[texture(1)]], sampler gtao_sampler [[sampler(0)]])
{
    main0_out out = {};
    float4 _156 = gbuffer.sample(gtao_sampler, in.v_uv, level(0.0));
    float3 _160 = _156.xyz;
    float _163 = _156.w;
    if ((dot(_160, _160) < 0.25) || (_163 >= (-9.9999997473787516355514526367188e-05)))
    {
        out.frag_ao = 1.0;
        return out;
    }
    float2 _181 = float2(1.0) / target_dims(_27);
    float3 _184 = fast::normalize(_160);
    float2 param = in.v_uv;
    float param_1 = _163;
    float3 _190 = reconstruct_view_pos(param, param_1, _27);
    float param_2 = _163;
    float _196 = fast::max(2.0 * view_pixel_size(param_2, _27), 9.9999999747524270787835121154785e-07);
    float sum = 0.0;
    float weight_sum = 0.0;
    for (int x = -2; x <= 2; x++)
    {
        for (int y = -2; y <= 2; y++)
        {
            float2 _222 = float2(float(x), float(y));
            float2 _228 = in.v_uv + (_222 * _181);
            float4 _234 = gbuffer.sample(gtao_sampler, _228, level(0.0));
            float3 _236 = _234.xyz;
            bool _240 = dot(_236, _236) < 0.25;
            bool _247;
            if (!_240)
            {
                _247 = _234.w >= (-9.9999997473787516355514526367188e-05);
            }
            else
            {
                _247 = _240;
            }
            if (_247)
            {
                continue;
            }
            float2 param_3 = _228;
            float param_4 = _234.w;
            float _261 = dot(_184, reconstruct_view_pos(param_3, param_4, _27) - _190);
            float _295 = (exp(dot(_222, _222) * (-0.22222222387790679931640625)) * exp((-(_261 * _261)) / ((2.0 * _196) * _196))) * powr(fast::max(dot(_184, fast::normalize(_236)), 0.0), 8.0);
            sum += (ao_in.sample(gtao_sampler, _228, level(0.0)).x * _295);
            weight_sum += _295;
        }
    }
    if (weight_sum <= 9.9999997473787516355514526367188e-06)
    {
        out.frag_ao = ao_in.sample(gtao_sampler, in.v_uv, level(0.0)).x;
        return out;
    }
    out.frag_ao = sum / weight_sum;
    return out;
}

