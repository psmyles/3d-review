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
    float _677;
    float _694;
    for (uint s = 0u; s < _330; s++)
    {
        float _438 = ((float(s) + _404) * 3.1415927410125732421875) / float(_330);
        float _441 = cos(_438);
        float _444 = sin(_438);
        float2 _449 = float2(_441, -_444);
        float3 _453 = float3(_441, _444, 0.0);
        float3 _461 = _453 - (_313 * dot(_453, _313));
        float3 _466 = fast::normalize(cross(_461, _313));
        float3 _474 = _303 - (_466 * dot(_303, _466));
        float _477 = length(_474);
        float proj_len = _477;
        if (_477 < 9.9999999747524270787835121154785e-07)
        {
            continue;
        }
        float _493 = proj_len;
        float _495 = fast::clamp(dot(_474, _313) / _493, 0.0, 1.0);
        float _500 = sign(dot(_474, _461)) * acos(_495);
        float _505 = cos(_500 + 1.57079637050628662109375);
        float _509 = cos(_500 - 1.57079637050628662109375);
        float horizon_cos0 = _505;
        float horizon_cos1 = _509;
        for (uint t = 0u; t < _335; t++)
        {
            float _542 = (float(t) + fract(_411 + (float(s + (t * _330)) * 0.61803400516510009765625))) / float(_335);
            float2 _553 = (_449 * ((_542 * _542) + _368)) * _359;
            int _566 = int(fast::clamp(log2(fast::max(length(_553), 1.0)) - 3.2999999523162841796875, 0.0, 4.0) + 0.5);
            float2 _571 = round(_553) * _341;
            float2 _575 = in.v_uv + _571;
            float2 _579 = in.v_uv - _571;
            float2 param_4 = _575;
            int param_5 = _566;
            float _585 = sample_depth_mip(param_4, param_5, depth_mip0, gtao_sampler, depth_mip1, depth_mip2, depth_mip3, depth_mip4);
            float2 param_6 = _579;
            int param_7 = _566;
            float _591 = sample_depth_mip(param_6, param_7, depth_mip0, gtao_sampler, depth_mip1, depth_mip2, depth_mip3, depth_mip4);
            float shc0 = _505;
            float param_8 = _585;
            if (!is_background_depth(param_8))
            {
                float2 param_9 = _575;
                float param_10 = -_585;
                float3 _608 = reconstruct_view_pos(param_9, param_10, _57) - _309;
                float _611 = length(_608);
                if (_611 > 9.9999999747524270787835121154785e-07)
                {
                    shc0 = mix(_505, dot(_608, _313) / _611, fast::clamp((_611 * _380) + _385, 0.0, 1.0));
                }
            }
            float shc1 = _509;
            float param_11 = _591;
            if (!is_background_depth(param_11))
            {
                float2 param_12 = _579;
                float param_13 = -_591;
                float3 _647 = reconstruct_view_pos(param_12, param_13, _57) - _309;
                float _650 = length(_647);
                if (_650 > 9.9999999747524270787835121154785e-07)
                {
                    shc1 = mix(_509, dot(_647, _313) / _650, fast::clamp((_650 * _380) + _385, 0.0, 1.0));
                }
            }
            float _673 = fast::max(horizon_cos0, shc0);
            if (horizon_cos0 > shc0)
            {
                _677 = mix(_673, shc0, _325);
            }
            else
            {
                _677 = _673;
            }
            horizon_cos0 = _677;
            float _690 = fast::max(horizon_cos1, shc1);
            if (horizon_cos1 > shc1)
            {
                _694 = mix(_690, shc1, _325);
            }
            else
            {
                _694 = _690;
            }
            horizon_cos1 = _694;
        }
        float _706 = proj_len;
        float _708 = mix(_706, 1.0, 0.0500000007450580596923828125);
        proj_len = _708;
        float _720 = sin(_500);
        float _724 = acos(fast::clamp(horizon_cos1, -1.0, 1.0)) * (-2.0);
        float _738 = 2.0 * acos(fast::clamp(horizon_cos0, -1.0, 1.0));
        visibility += (_708 * (0.25 * (((_495 + (_724 * _720)) - cos(_724 - _500)) + ((_495 + (_738 * _720)) - cos(_738 - _500)))));
    }
    float _758 = visibility;
    float _762 = fast::clamp(_758 / float(_330), 0.0, 1.0);
    visibility = _762;
    float _768 = powr(fast::max(_762, 0.0), fast::max(_57.params.y, 0.0));
    if (_57.temporal.z >= 1.0)
    {
        out.frag_ao = _768;
    }
    else
    {
        out.frag_ao = mix(ao_history.sample(gtao_sampler, in.v_uv, level(0.0)).x, _768, _57.temporal.z);
    }
    return out;
}

