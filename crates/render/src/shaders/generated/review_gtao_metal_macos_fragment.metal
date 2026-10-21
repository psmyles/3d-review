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
bool is_background_depth(thread const float& depth)
{
    return !((depth > 0.0) && (depth < 1000000015047466219876688855040.0));
}

static inline __attribute__((always_inline))
float3 reconstruct_view_pos(thread const float2& uv, thread const float& view_z, constant gtao_params& _57)
{
    float _78 = (uv.x * 2.0) - 1.0;
    float _83 = 1.0 - (uv.y * 2.0);
    if (_57.config.x > 0.5)
    {
        return float3((_78 - _57.proj[3].x) / _57.proj[0].x, (_83 - _57.proj[3].y) / _57.proj[1].y, view_z);
    }
    return float3((_78 * (-view_z)) / _57.proj[0].x, (_83 * (-view_z)) / _57.proj[1].y, view_z);
}

static inline __attribute__((always_inline))
float2 target_dims(constant gtao_params& _57)
{
    return float2(fast::max(_57.params.w, 1.0), fast::max(_57.config.w, 1.0));
}

static inline __attribute__((always_inline))
float2 project_to_uv(thread const float3& view_pos, constant gtao_params& _57)
{
    float4 _150 = _57.proj * float4(view_pos, 1.0);
    float2 _153 = _150.xy;
    float2 ndc = _153;
    if (_57.config.x <= 0.5)
    {
        ndc = _153 / float2(_150.w);
    }
    return float2((ndc.x * 0.5) + 0.5, 0.5 - (ndc.y * 0.5));
}

static inline __attribute__((always_inline))
float2 spatial_dither(thread const uint2& pix)
{
    return float2(0.0625 * float((((pix.x + pix.y) & 3u) << 2u) + (pix.x & 3u)), 0.25 * float((pix.y - pix.x) & 3u));
}

static inline __attribute__((always_inline))
float sample_depth_mip(thread const float2& uv, thread const int& level0, texture2d<float> depth_mip0, sampler gtao_sampler, texture2d<float> depth_mip1, texture2d<float> depth_mip2, texture2d<float> depth_mip3, texture2d<float> depth_mip4)
{
    float depth = 0.0;
    if (level0 <= 0)
    {
        depth = depth_mip0.sample(gtao_sampler, uv, level(0.0)).x;
    }
    else
    {
        if (level0 == 1)
        {
            depth = depth_mip1.sample(gtao_sampler, uv, level(0.0)).x;
        }
        else
        {
            if (level0 == 2)
            {
                depth = depth_mip2.sample(gtao_sampler, uv, level(0.0)).x;
            }
            else
            {
                if (level0 == 3)
                {
                    depth = depth_mip3.sample(gtao_sampler, uv, level(0.0)).x;
                }
                else
                {
                    depth = depth_mip4.sample(gtao_sampler, uv, level(0.0)).x;
                }
            }
        }
    }
    return depth;
}

fragment main0_out main0(main0_in in [[stage_in]], constant gtao_params& _57 [[buffer(0)]], texture2d<float> gbuffer [[texture(0)]], texture2d<float> depth_mip0 [[texture(1)]], texture2d<float> depth_mip1 [[texture(2)]], texture2d<float> depth_mip2 [[texture(3)]], texture2d<float> depth_mip3 [[texture(4)]], texture2d<float> depth_mip4 [[texture(5)]], texture2d<float> ao_history [[texture(6)]], sampler gtao_sampler [[sampler(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float3 _284 = gbuffer.sample(gtao_sampler, in.v_uv, level(0.0)).xyz;
    float4 _290 = depth_mip0.sample(gtao_sampler, in.v_uv, level(0.0));
    float _291 = _290.x;
    bool _295 = dot(_284, _284) < 0.25;
    bool _302;
    if (!_295)
    {
        float param = _291;
        _302 = is_background_depth(param);
    }
    else
    {
        _302 = _295;
    }
    if (_302 || (_291 <= 9.9999997473787516355514526367188e-05))
    {
        out.frag_ao = 1.0;
        return out;
    }
    float3 _317 = fast::normalize(_284);
    float2 param_1 = in.v_uv;
    float param_2 = -_291;
    float3 _323 = reconstruct_view_pos(param_1, param_2, _57);
    float3 _327 = fast::normalize(-_323);
    float _331 = fast::max(_57.params.x, 9.9999997473787516355514526367188e-05);
    float _338 = fast::clamp(_57.params.z, 0.0, 1.0);
    uint _343 = max(uint(_57.config.y), 1u);
    uint _348 = max(uint(_57.config.z), 1u);
    float2 _350 = target_dims(_57);
    float2 _354 = float2(1.0) / _350;
    float3 param_3 = _323 + float3(_331, 0.0, 0.0);
    float _372 = abs(project_to_uv(param_3, _57).x - in.v_uv.x) * _350.x;
    if (_372 < 1.2999999523162841796875)
    {
        out.frag_ao = 1.0;
        return out;
    }
    float _381 = 1.2999999523162841796875 / _372;
    float _393 = (-1.62601625919342041015625) / _331;
    float _398 = ((_331 * 0.3849999904632568359375) / (0.6150000095367431640625 * _331)) + 1.0;
    uint2 param_4 = uint2(uint(gl_FragCoord.x), uint(gl_FragCoord.y));
    float2 _410 = spatial_dither(param_4);
    float _417 = fract(_410.x + _57.temporal.x);
    float _424 = fract(_410.y + _57.temporal.y);
    float visibility = fast::clamp((10.0 - _372) * 0.00999999977648258209228515625, 0.0, 1.0) * 0.5;
    float _704;
    float _721;
    for (uint s = 0u; s < _343; s++)
    {
        float _451 = ((float(s) + _417) * 3.1415927410125732421875) / float(_343);
        float _454 = cos(_451);
        float _457 = sin(_451);
        float2 _462 = float2(_454, -_457);
        float3 _466 = float3(_454, _457, 0.0);
        float3 _474 = _466 - (_327 * dot(_466, _327));
        float3 _478 = cross(_474, _327);
        float _482 = dot(_478, _478);
        if (_482 < 9.9999999600419720025001879548654e-13)
        {
            continue;
        }
        float3 _493 = _478 * rsqrt(_482);
        float3 _501 = _317 - (_493 * dot(_317, _493));
        float _504 = length(_501);
        float proj_len = _504;
        if (_504 < 9.9999999747524270787835121154785e-07)
        {
            continue;
        }
        float _520 = proj_len;
        float _522 = fast::clamp(dot(_501, _327) / _520, 0.0, 1.0);
        float _527 = sign(dot(_501, _474)) * acos(_522);
        float _532 = cos(_527 + 1.57079637050628662109375);
        float _536 = cos(_527 - 1.57079637050628662109375);
        float horizon_cos0 = _532;
        float horizon_cos1 = _536;
        for (uint t = 0u; t < _348; t++)
        {
            float _569 = (float(t) + fract(_424 + (float(s + (t * _343)) * 0.61803400516510009765625))) / float(_348);
            float2 _580 = (_462 * ((_569 * _569) + _381)) * _372;
            int _593 = int(fast::clamp(log2(fast::max(length(_580), 1.0)) - 3.2999999523162841796875, 0.0, 4.0) + 0.5);
            float2 _598 = round(_580) * _354;
            float2 _602 = in.v_uv + _598;
            float2 _606 = in.v_uv - _598;
            float2 param_5 = _602;
            int param_6 = _593;
            float _612 = sample_depth_mip(param_5, param_6, depth_mip0, gtao_sampler, depth_mip1, depth_mip2, depth_mip3, depth_mip4);
            float2 param_7 = _606;
            int param_8 = _593;
            float _618 = sample_depth_mip(param_7, param_8, depth_mip0, gtao_sampler, depth_mip1, depth_mip2, depth_mip3, depth_mip4);
            float shc0 = _532;
            float param_9 = _612;
            if (!is_background_depth(param_9))
            {
                float2 param_10 = _602;
                float param_11 = -_612;
                float3 _635 = reconstruct_view_pos(param_10, param_11, _57) - _323;
                float _638 = length(_635);
                if (_638 > 9.9999999747524270787835121154785e-07)
                {
                    shc0 = mix(_532, dot(_635, _327) / _638, fast::clamp((_638 * _393) + _398, 0.0, 1.0));
                }
            }
            float shc1 = _536;
            float param_12 = _618;
            if (!is_background_depth(param_12))
            {
                float2 param_13 = _606;
                float param_14 = -_618;
                float3 _674 = reconstruct_view_pos(param_13, param_14, _57) - _323;
                float _677 = length(_674);
                if (_677 > 9.9999999747524270787835121154785e-07)
                {
                    shc1 = mix(_536, dot(_674, _327) / _677, fast::clamp((_677 * _393) + _398, 0.0, 1.0));
                }
            }
            float _700 = fast::max(horizon_cos0, shc0);
            if (horizon_cos0 > shc0)
            {
                _704 = mix(_700, shc0, _338);
            }
            else
            {
                _704 = _700;
            }
            horizon_cos0 = _704;
            float _717 = fast::max(horizon_cos1, shc1);
            if (horizon_cos1 > shc1)
            {
                _721 = mix(_717, shc1, _338);
            }
            else
            {
                _721 = _717;
            }
            horizon_cos1 = _721;
        }
        float _733 = proj_len;
        float _735 = mix(_733, 1.0, 0.0500000007450580596923828125);
        proj_len = _735;
        float _747 = sin(_527);
        float _751 = acos(fast::clamp(horizon_cos1, -1.0, 1.0)) * (-2.0);
        float _765 = 2.0 * acos(fast::clamp(horizon_cos0, -1.0, 1.0));
        visibility += (_735 * (0.25 * (((_522 + (_751 * _747)) - cos(_751 - _527)) + ((_522 + (_765 * _747)) - cos(_765 - _527)))));
    }
    float _785 = visibility;
    float _789 = fast::clamp(_785 / float(_343), 0.0, 1.0);
    visibility = _789;
    float _795 = powr(fast::max(_789, 9.9999999747524270787835121154785e-07), fast::max(_57.params.y, 0.0));
    if (_57.temporal.z >= 1.0)
    {
        out.frag_ao = _795;
    }
    else
    {
        float4 _809 = ao_history.sample(gtao_sampler, in.v_uv, level(0.0));
        float _810 = _809.x;
        float _819;
        if ((_810 >= 0.0) && (_810 <= 1.0))
        {
            _819 = mix(_810, _795, _57.temporal.z);
        }
        else
        {
            _819 = _795;
        }
        out.frag_ao = _819;
    }
    return out;
}

