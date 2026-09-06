#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

// Implementation of the GLSL mod() function, which is slightly different than Metal fmod()
template<typename Tx, typename Ty>
inline Tx mod(Tx x, Ty y)
{
    return x - y * floor(x / y);
}

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

fragment main0_out main0(constant tex_params& _14 [[buffer(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float2 _30 = floor(gl_FragCoord.xy / float2(fast::max(_14.checker_cell, 1.0)));
    float4 _49;
    if (mod(_30.x + _30.y, 2.0) < 0.5)
    {
        _49 = _14.bg_light;
    }
    else
    {
        _49 = _14.bg_dark;
    }
    out.frag_color = _49;
    return out;
}

