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

static inline __attribute__((always_inline))
bool is_background_depth(thread const float& depth)
{
    return !((depth > 0.0) && (depth < 1000000015047466219876688855040.0));
}

fragment main0_out main0(main0_in in [[stage_in]], constant gtao_params& _57 [[buffer(0)]], texture2d<float> gbuffer [[texture(0)]], texture2d<float> depth_mip0 [[texture(1)]], texture2d<float> depth_mip1 [[texture(2)]], texture2d<float> depth_mip2 [[texture(3)]], texture2d<float> depth_mip3 [[texture(4)]], texture2d<float> depth_mip4 [[texture(5)]], texture2d<float> ao_history [[texture(6)]], sampler gtao_sampler [[sampler(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float4 _281 = gbuffer.sample(gtao_sampler, in.v_uv, level(0.0));
    float3 _284 = _281.xyz;
    float _287 = _281.w;
    if ((dot(_284, _284) < 0.25) || (_287 >= (-9.9999997473787516355514526367188e-05)))
    {
        out.frag_ao = 1.0;
        return out;
    }
    float3 _303 = fast::normalize(_284);
    float2 param = in.v_uv;
    float param_1 = _287;
    float3 _309 = reconstruct_view_pos(param, param_1, _57);
    float3 _313 = fast::normalize(-_309);
    float _318 = fast::max(_57.params.x, 9.9999997473787516355514526367188e-05);
    float _325 = fast::clamp(_57.params.z, 0.0, 1.0);
    uint _330 = max(uint(_57.config.y), 1u);
    uint _335 = max(uint(_57.config.z), 1u);
    float2 _337 = target_dims(_57);
    float2 _341 = float2(1.0) / _337;
    float3 param_2 = _309 + float3(_318, 0.0, 0.0);
    float _359 = abs(project_to_uv(param_2, _57).x - in.v_uv.x) * _337.x;
    if (_359 < 1.2999999523162841796875)
    {
        out.frag_ao = 1.0;
        return out;
    }
    float _368 = 1.2999999523162841796875 / _359;
    float _380 = (-1.62601625919342041015625) / _318;
    float _385 = ((_318 * 0.3849999904632568359375) / (0.6150000095367431640625 * _318)) + 1.0;
    uint2 param_3 = uint2(uint(gl_FragCoord.x), uint(gl_FragCoord.y));
    float2 _397 = spatial_dither(param_3);
    float _404 = fract(_397.x + _57.temporal.x);
    float _411 = fract(_397.y + _57.temporal.y);
    float visibility = fast::clamp((10.0 - _359) * 0.00999999977648258209228515625, 0.0, 1.0) * 0.5;
    float _691;
    float _708;
    for (uint s = 0u; s < _330; s++)
    {
        float _438 = ((float(s) + _404) * 3.1415927410125732421875) / float(_330);
        float _441 = cos(_438);
        float _444 = sin(_438);
        float2 _449 = float2(_441, -_444);
        float3 _453 = float3(_441, _444, 0.0);
        float3 _461 = _453 - (_313 * dot(_453, _313));
        float3 _465 = cross(_461, _313);
        float _469 = dot(_465, _465);
        if (_469 < 9.9999999600419720025001879548654e-13)
        {
            continue;
        }
        float3 _480 = _465 * rsqrt(_469);
        float3 _488 = _303 - (_480 * dot(_303, _480));
        float _491 = length(_488);
        float proj_len = _491;
        if (_491 < 9.9999999747524270787835121154785e-07)
        {
            continue;
        }
        float _507 = proj_len;
        float _509 = fast::clamp(dot(_488, _313) / _507, 0.0, 1.0);
        float _514 = sign(dot(_488, _461)) * acos(_509);
        float _519 = cos(_514 + 1.57079637050628662109375);
        float _523 = cos(_514 - 1.57079637050628662109375);
        float horizon_cos0 = _519;
        float horizon_cos1 = _523;
        for (uint t = 0u; t < _335; t++)
        {
            float _556 = (float(t) + fract(_411 + (float(s + (t * _330)) * 0.61803400516510009765625))) / float(_335);
            float2 _567 = (_449 * ((_556 * _556) + _368)) * _359;
            int _580 = int(fast::clamp(log2(fast::max(length(_567), 1.0)) - 3.2999999523162841796875, 0.0, 4.0) + 0.5);
            float2 _585 = round(_567) * _341;
            float2 _589 = in.v_uv + _585;
            float2 _593 = in.v_uv - _585;
            float2 param_4 = _589;
            int param_5 = _580;
            float _599 = sample_depth_mip(param_4, param_5, depth_mip0, gtao_sampler, depth_mip1, depth_mip2, depth_mip3, depth_mip4);
            float2 param_6 = _593;
            int param_7 = _580;
            float _605 = sample_depth_mip(param_6, param_7, depth_mip0, gtao_sampler, depth_mip1, depth_mip2, depth_mip3, depth_mip4);
            float shc0 = _519;
            float param_8 = _599;
            if (!is_background_depth(param_8))
            {
                float2 param_9 = _589;
                float param_10 = -_599;
                float3 _622 = reconstruct_view_pos(param_9, param_10, _57) - _309;
                float _625 = length(_622);
                if (_625 > 9.9999999747524270787835121154785e-07)
                {
                    shc0 = mix(_519, dot(_622, _313) / _625, fast::clamp((_625 * _380) + _385, 0.0, 1.0));
                }
            }
            float shc1 = _523;
            float param_11 = _605;
            if (!is_background_depth(param_11))
            {
                float2 param_12 = _593;
                float param_13 = -_605;
                float3 _661 = reconstruct_view_pos(param_12, param_13, _57) - _309;
                float _664 = length(_661);
                if (_664 > 9.9999999747524270787835121154785e-07)
                {
                    shc1 = mix(_523, dot(_661, _313) / _664, fast::clamp((_664 * _380) + _385, 0.0, 1.0));
                }
            }
            float _687 = fast::max(horizon_cos0, shc0);
            if (horizon_cos0 > shc0)
            {
                _691 = mix(_687, shc0, _325);
            }
            else
            {
                _691 = _687;
            }
            horizon_cos0 = _691;
            float _704 = fast::max(horizon_cos1, shc1);
            if (horizon_cos1 > shc1)
            {
                _708 = mix(_704, shc1, _325);
            }
            else
            {
                _708 = _704;
            }
            horizon_cos1 = _708;
        }
        float _720 = proj_len;
        float _722 = mix(_720, 1.0, 0.0500000007450580596923828125);
        proj_len = _722;
        float _734 = sin(_514);
        float _738 = acos(fast::clamp(horizon_cos1, -1.0, 1.0)) * (-2.0);
        float _752 = 2.0 * acos(fast::clamp(horizon_cos0, -1.0, 1.0));
        visibility += (_722 * (0.25 * (((_509 + (_738 * _734)) - cos(_738 - _514)) + ((_509 + (_752 * _734)) - cos(_752 - _514)))));
    }
    float _772 = visibility;
    float _776 = fast::clamp(_772 / float(_330), 0.0, 1.0);
    visibility = _776;
    float _782 = powr(fast::max(_776, 9.9999999747524270787835121154785e-07), fast::max(_57.params.y, 0.0));
    if (_57.temporal.z >= 1.0)
    {
        out.frag_ao = _782;
    }
    else
    {
        float4 _796 = ao_history.sample(gtao_sampler, in.v_uv, level(0.0));
        float _797 = _796.x;
        float _806;
        if ((_797 >= 0.0) && (_797 <= 1.0))
        {
            _806 = mix(_797, _782, _57.temporal.z);
        }
        else
        {
            _806 = _782;
        }
        out.frag_ao = _806;
    }
    return out;
}

