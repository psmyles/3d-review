#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct overdraw_params
{
    float4 rect;
    float4 range;
    float4 stop0;
    float4 stop1;
    float4 stop2;
    float4 stop3;
    float4 empty;
};

struct main0_out
{
    float4 frag_color [[color(0)]];
};

fragment main0_out main0(constant overdraw_params& _17 [[buffer(0)]], texture2d<float> count_tex [[texture(0)]], sampler count_smp [[sampler(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float4 _46 = count_tex.sample(count_smp, ((gl_FragCoord.xy - _17.rect.xy) / fast::max(_17.rect.zw, float2(1.0))), level(0.0));
    float _49 = _46.x;
    if (_49 <= 0.0)
    {
        out.frag_color = float4(_17.empty.xyz, 1.0);
        return out;
    }
    float _85 = fast::clamp((_49 - _17.range.y) / fast::max(_17.range.x - _17.range.y, 0.001000000047497451305389404296875), 0.0, 1.0) * 3.0;
    float3 _90;
    if (_85 < 1.0)
    {
        _90 = mix(_17.stop0.xyz, _17.stop1.xyz, float3(_85));
    }
    else
    {
        float3 _108;
        if (_85 < 2.0)
        {
            _108 = mix(_17.stop1.xyz, _17.stop2.xyz, float3(_85 - 1.0));
        }
        else
        {
            _108 = mix(_17.stop2.xyz, _17.stop3.xyz, float3(_85 - 2.0));
        }
        _90 = _108;
    }
    out.frag_color = float4(_90, 1.0);
    return out;
}

