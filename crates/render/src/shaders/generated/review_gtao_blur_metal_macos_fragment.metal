#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct gtao_params
{
    float4x4 proj;
    float4 params;
    float4 config;
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
float2 target_dims(constant gtao_params& _15)
{
    return float2(fast::max(_15.params.w, 1.0), fast::max(_15.config.w, 1.0));
}

fragment main0_out main0(main0_in in [[stage_in]], constant gtao_params& _15 [[buffer(0)]], texture2d<float> gbuffer [[texture(0)]], texture2d<float> raw_ao [[texture(1)]], sampler gtao_sampler [[sampler(0)]])
{
    main0_out out = {};
    float4 _48 = gbuffer.sample(gtao_sampler, in.v_uv, level(0.0));
    float3 _53 = _48.xyz;
    float _57 = _48.w;
    if ((dot(_53, _53) < 0.25) || (_57 >= (-9.9999997473787516355514526367188e-05)))
    {
        out.frag_ao = 1.0;
        return out;
    }
    float2 _77 = float2(1.0) / target_dims(_15);
    float3 _80 = fast::normalize(_53);
    float _88 = fast::max(_15.params.x * 0.119999997317790985107421875, 9.9999997473787516355514526367188e-05);
    float sum = 0.0;
    float weight_sum = 0.0;
    for (int x = -2; x <= 2; x++)
    {
        for (int y = -2; y <= 2; y++)
        {
            float2 _116 = float2(float(x), float(y));
            float2 _122 = in.v_uv + (_116 * _77);
            float4 _128 = gbuffer.sample(gtao_sampler, _122, level(0.0));
            float3 _131 = _128.xyz;
            bool _136 = dot(_131, _131) < 0.25;
            bool _143;
            if (!_136)
            {
                _143 = _128.w >= (-9.9999997473787516355514526367188e-05);
            }
            else
            {
                _143 = _136;
            }
            if (_143)
            {
                continue;
            }
            float _152 = dot(_80, fast::normalize(_131));
            if (_152 < 0.75)
            {
                continue;
            }
            float _175 = abs(_128.w - _57);
            float _195 = (exp(dot(_116, _116) * (-0.125)) * exp((-(_175 * _175)) / ((2.0 * _88) * _88))) * smoothstep(0.75, 1.0, _152);
            sum += (raw_ao.sample(gtao_sampler, _122, level(0.0)).x * _195);
            weight_sum += _195;
        }
    }
    if (weight_sum <= 9.9999997473787516355514526367188e-06)
    {
        out.frag_ao = raw_ao.sample(gtao_sampler, in.v_uv, level(0.0)).x;
        return out;
    }
    out.frag_ao = sum / weight_sum;
    return out;
}

