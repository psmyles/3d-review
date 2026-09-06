#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct tex_params
{
    float2 img_min;
    float2 img_size;
    int channel;
    int target_srgb;
    float checker_cell;
    int pad;
    float4 bg_light;
    float4 bg_dark;
};

struct main0_out
{
    float4 frag_color [[color(0)]];
};

static inline __attribute__((always_inline))
float3 srgb_to_linear(thread const float3& c)
{
    return mix(powr(fast::max((c + float3(0.054999999701976776123046875)) * float3(0.947867333889007568359375), float3(0.0)), float3(2.400000095367431640625)), c * float3(0.077399380505084991455078125), step(c, float3(0.040449999272823333740234375)));
}

fragment main0_out main0(constant tex_params& _54 [[buffer(0)]], texture2d<float> tex [[texture(0)]], sampler samp [[sampler(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float2 _63 = (gl_FragCoord.xy - _54.img_min) / _54.img_size;
    float4 _77 = tex.sample(samp, _63);
    float _83 = _63.x;
    bool _84 = _83 < 0.0;
    bool _92;
    if (!_84)
    {
        _92 = _83 > 1.0;
    }
    else
    {
        _92 = _84;
    }
    bool _100;
    if (!_92)
    {
        _100 = _63.y < 0.0;
    }
    else
    {
        _100 = _92;
    }
    bool _107;
    if (!_100)
    {
        _107 = _63.y > 1.0;
    }
    else
    {
        _107 = _100;
    }
    if (_107)
    {
        discard_fragment();
    }
    float alpha = 1.0;
    float3 rgb;
    switch (_54.channel)
    {
        case 0:
        {
            rgb = _77.xyz;
            alpha = _77.w;
            break;
        }
        case 1:
        {
            rgb = float3(_77.x);
            break;
        }
        case 2:
        {
            rgb = float3(_77.y);
            break;
        }
        case 3:
        {
            rgb = float3(_77.z);
            break;
        }
        default:
        {
            rgb = float3(_77.w);
            break;
        }
    }
    float3 _152;
    if (_54.target_srgb == 1)
    {
        float3 param = rgb;
        _152 = srgb_to_linear(param);
    }
    else
    {
        _152 = rgb;
    }
    out.frag_color = float4(_152, alpha);
    return out;
}

