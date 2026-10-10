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

struct quad_params
{
    float4 params;
};

struct main0_out
{
    float4 frag_count [[color(0)]];
};

struct main0_in
{
    float3 v_corner_a [[user(locn0), flat]];
    float3 v_corner_b [[user(locn1), flat]];
    float3 v_corner_c [[user(locn2), flat]];
};

fragment main0_out main0(main0_in in [[stage_in]], constant quad_params& qu [[buffer(4)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float3 _15 = cross(in.v_corner_b, in.v_corner_c);
    float3 _20 = cross(in.v_corner_c, in.v_corner_a);
    float3 _24 = cross(in.v_corner_a, in.v_corner_b);
    float _29 = dot(in.v_corner_a, _15);
    float2 _45 = fast::max(qu.params.xy, float2(1.0));
    float2 _51 = floor(gl_FragCoord.xy);
    float2 _58 = _51 - mod(_51, float2(2.0));
    float covered = 0.0;
    for (int i = 0; i < 4; i++)
    {
        float2 _85 = (_58 + float2(float(i & 1), float(i >> 1))) + float2(0.5);
        float3 _104 = float3(((_85.x / _45.x) * 2.0) - 1.0, 1.0 - ((_85.y / _45.y) * 2.0), 1.0);
        float3 _118 = float3(dot(_15, _104), dot(_20, _104), dot(_24, _104)) * sign(_29);
        bool _121 = _118.x >= 0.0;
        bool _127;
        if (_121)
        {
            _127 = _118.y >= 0.0;
        }
        else
        {
            _127 = _121;
        }
        bool _134;
        if (_127)
        {
            _134 = _118.z >= 0.0;
        }
        else
        {
            _134 = _127;
        }
        covered += float(_134);
    }
    float _145;
    if (abs(_29) > 9.9999999600419720025001879548654e-13)
    {
        _145 = fast::max(covered, 1.0);
    }
    else
    {
        _145 = 4.0;
    }
    out.frag_count = float4(4.0 / _145, 0.0, 0.0, 1.0);
    return out;
}

