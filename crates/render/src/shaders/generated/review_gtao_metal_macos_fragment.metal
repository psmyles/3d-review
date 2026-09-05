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
float3 reconstruct_view_pos(thread const float2& uv, thread const float& view_z, constant gtao_params& _53)
{
    float _75 = (uv.x * 2.0) - 1.0;
    float _80 = 1.0 - (uv.y * 2.0);
    if (_53.config.x > 0.5)
    {
        return float3((_75 - _53.proj[3].x) / _53.proj[0].x, (_80 - _53.proj[3].y) / _53.proj[1].y, view_z);
    }
    return float3((_75 * (-view_z)) / _53.proj[0].x, (_80 * (-view_z)) / _53.proj[1].y, view_z);
}

static inline __attribute__((always_inline))
float2 target_dims(constant gtao_params& _53)
{
    return float2(fast::max(_53.params.w, 1.0), fast::max(_53.config.w, 1.0));
}

static inline __attribute__((always_inline))
float2 project_to_uv(thread const float3& view_pos, constant gtao_params& _53)
{
    float4 _148 = _53.proj * float4(view_pos, 1.0);
    float2 _151 = _148.xy;
    float2 ndc = _151;
    if (_53.config.x <= 0.5)
    {
        ndc = _151 / float2(_148.w);
    }
    return float2((ndc.x * 0.5) + 0.5, 0.5 - (ndc.y * 0.5));
}

static inline __attribute__((always_inline))
float2 spatial_dither(thread const uint2& pix)
{
    return float2(0.0625 * float((((pix.x + pix.y) & 3u) << 2u) + (pix.x & 3u)), 0.25 * float((pix.y - pix.x) & 3u));
}

static inline __attribute__((always_inline))
float4 sample_view_pos(thread const float2& uv, constant gtao_params& _53, texture2d<float> gbuffer, sampler gtao_sampler)
{
    float4 _187 = gbuffer.sample(gtao_sampler, uv, level(0.0));
    float3 _189 = _187.xyz;
    bool _194 = dot(_189, _189) < 0.25;
    bool _202;
    if (!_194)
    {
        _202 = _187.w >= (-9.9999997473787516355514526367188e-05);
    }
    else
    {
        _202 = _194;
    }
    if (_202)
    {
        return float4(0.0);
    }
    float2 param = uv;
    float param_1 = _187.w;
    return float4(reconstruct_view_pos(param, param_1, _53), 1.0);
}

static inline __attribute__((always_inline))
float horizon_cos(thread const float2& origin_uv, thread const float3& p, thread const float3& v, thread const float2& dir_px, thread const float& radius_px, thread const float& radius, thread const float& thickness, thread const uint& steps, thread const float& jitter, thread const float2& inv_dims, constant gtao_params& _53, texture2d<float> gbuffer, sampler gtao_sampler)
{
    float cos_h = -1.0;
    for (uint t = 1u; t <= steps; t++)
    {
        float2 _277 = origin_uv + ((dir_px * (radius_px * ((float(t) - jitter) / float(steps)))) * inv_dims);
        float _279 = _277.x;
        bool _280 = _279 < 0.0;
        bool _287;
        if (!_280)
        {
            _287 = _279 > 1.0;
        }
        else
        {
            _287 = _280;
        }
        bool _294;
        if (!_287)
        {
            _294 = _277.y < 0.0;
        }
        else
        {
            _294 = _287;
        }
        bool _301;
        if (!_294)
        {
            _301 = _277.y > 1.0;
        }
        else
        {
            _301 = _294;
        }
        if (_301)
        {
            break;
        }
        float2 param = _277;
        float4 _308 = sample_view_pos(param, _53, gbuffer, gtao_sampler);
        if (_308.w < 0.5)
        {
            continue;
        }
        float3 _319 = _308.xyz - p;
        float _322 = length(_319);
        if (_322 < 9.9999997473787516355514526367188e-06)
        {
            continue;
        }
        float _340 = fast::clamp(1.0 - (_322 / radius), 0.0, 1.0);
        if (_340 <= 0.0)
        {
            continue;
        }
        cos_h = fast::max(cos_h, (dot(_319, v) / _322) - ((1.0 - _340) * thickness));
    }
    return cos_h;
}

fragment main0_out main0(main0_in in [[stage_in]], constant gtao_params& _53 [[buffer(0)]], texture2d<float> gbuffer [[texture(0)]], sampler gtao_sampler [[sampler(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float4 _366 = gbuffer.sample(gtao_sampler, in.v_uv, level(0.0));
    float3 _369 = _366.xyz;
    float _372 = _366.w;
    if ((dot(_369, _369) < 0.25) || (_372 >= (-9.9999997473787516355514526367188e-05)))
    {
        out.frag_ao = 1.0;
        return out;
    }
    float3 _387 = fast::normalize(_369);
    float2 param = in.v_uv;
    float param_1 = _372;
    float3 _393 = reconstruct_view_pos(param, param_1, _53);
    float3 _397 = fast::normalize(-_393);
    float _407 = fast::clamp(_53.params.z, 0.0, 1.0);
    uint _412 = max(uint(_53.config.y), 1u);
    uint _417 = max(uint(_53.config.z), 1u);
    float2 _419 = target_dims(_53);
    float2 _423 = float2(1.0) / _419;
    float3 param_2 = _393 + float3(_53.params.x, 0.0, 0.0);
    float _440 = _419.x;
    float _447 = fast::clamp(abs(project_to_uv(param_2, _53).x - in.v_uv.x) * _440, 1.0, fast::max(_440, _419.y));
    uint2 param_3 = uint2(uint(gl_FragCoord.x), uint(gl_FragCoord.y));
    float2 _459 = spatial_dither(param_3);
    float _462 = _459.x;
    float _465 = _459.y;
    float visibility = 0.0;
    for (uint s = 0u; s < _412; s++)
    {
        float _485 = ((float(s) + _462) * 3.1415927410125732421875) / float(_412);
        float _488 = cos(_485);
        float _490 = sin(_485);
        float2 _491 = float2(_488, _490);
        float2 param_4 = in.v_uv + (_491 * (_423 * 2.0));
        float4 _500 = sample_view_pos(param_4, _53, gbuffer, gtao_sampler);
        float3 slice_dir = fast::normalize(float3(_488, _490, 0.0));
        if (_500.w > 0.5)
        {
            float3 _517 = _500.xyz - _393;
            if (dot(_517, _517) > 9.9999999600419720025001879548654e-13)
            {
                slice_dir = fast::normalize(_517);
            }
        }
        float3 _531 = fast::normalize(cross(_397, slice_dir));
        float3 _539 = _387 - (_531 * dot(_387, _531));
        float _542 = length(_539);
        if (_542 < 9.9999997473787516355514526367188e-05)
        {
            continue;
        }
        float3 _553 = _539 / float3(_542);
        float _568 = sign(dot(cross(slice_dir, _553), _531)) * acos(fast::clamp(dot(_553, _397), -1.0, 1.0));
        float2 param_5 = in.v_uv;
        float3 param_6 = _393;
        float3 param_7 = _397;
        float2 param_8 = _491;
        float param_9 = _447;
        float param_10 = _53.params.x;
        float param_11 = _407;
        uint param_12 = _417;
        float param_13 = _465;
        float2 param_14 = _423;
        float2 param_15 = in.v_uv;
        float3 param_16 = _393;
        float3 param_17 = _397;
        float2 param_18 = -_491;
        float param_19 = _447;
        float param_20 = _53.params.x;
        float param_21 = _407;
        uint param_22 = _417;
        float param_23 = _465;
        float2 param_24 = _423;
        float _637 = cos(_568);
        float _640 = sin(_568);
        float _643 = 2.0 * (_568 + fast::max((-acos(fast::clamp(horizon_cos(param_15, param_16, param_17, param_18, param_19, param_20, param_21, param_22, param_23, param_24, _53, gbuffer, gtao_sampler), -1.0, 1.0))) - _568, -1.57079637050628662109375));
        float _656 = 2.0 * (_568 + fast::min(acos(fast::clamp(horizon_cos(param_5, param_6, param_7, param_8, param_9, param_10, param_11, param_12, param_13, param_14, _53, gbuffer, gtao_sampler), -1.0, 1.0)) - _568, 1.57079637050628662109375));
        visibility += (_542 * (0.25 * ((((-cos(_643 - _568)) + _637) + (_643 * _640)) + (((-cos(_656 - _568)) + _637) + (_656 * _640)))));
    }
    float _677 = visibility;
    float _681 = fast::clamp(_677 / float(_412), 0.0, 1.0);
    visibility = _681;
    out.frag_ao = powr(fast::max(_681, 0.0), fast::max(_53.params.y, 0.0));
    return out;
}

