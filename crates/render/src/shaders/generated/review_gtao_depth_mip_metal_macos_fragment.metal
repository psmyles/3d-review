#pragma clang diagnostic ignored "-Wmissing-prototypes"
#pragma clang diagnostic ignored "-Wmissing-braces"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

template<typename T, size_t Num>
struct spvUnsafeArray
{
    T elements[Num ? Num : 1];
    
    thread T& operator [] (size_t pos) thread
    {
        return elements[pos];
    }
    constexpr const thread T& operator [] (size_t pos) const thread
    {
        return elements[pos];
    }
    
    device T& operator [] (size_t pos) device
    {
        return elements[pos];
    }
    constexpr const device T& operator [] (size_t pos) const device
    {
        return elements[pos];
    }
    
    constexpr const constant T& operator [] (size_t pos) const constant
    {
        return elements[pos];
    }
    
    threadgroup T& operator [] (size_t pos) threadgroup
    {
        return elements[pos];
    }
    constexpr const threadgroup T& operator [] (size_t pos) const threadgroup
    {
        return elements[pos];
    }
};

struct gtao_mip_params
{
    float4 mip;
};

struct main0_out
{
    float frag_depth [[color(0)]];
};

static inline __attribute__((always_inline))
bool is_background_depth(thread const float& depth)
{
    return !((depth > 0.0) && (depth < 1000000015047466219876688855040.0));
}

fragment main0_out main0(constant gtao_mip_params& _29 [[buffer(0)]], texture2d<float> depth_src [[texture(0)]], sampler gtao_sampler [[sampler(0)]], float4 gl_FragCoord [[position]])
{
    main0_out out = {};
    float2 _38 = fast::max(_29.mip.yz, float2(1.0));
    float2 _46 = floor(gl_FragCoord.xy) * 2.0;
    spvUnsafeArray<float, 4> depths;
    for (int i = 0; i < 4; i++)
    {
        float4 _88 = depth_src.sample(gtao_sampler, ((fast::min(_46 + float2(float(i & 1), float(i >> 1)), _38 - float2(1.0)) + float2(0.5)) / _38), level(0.0));
        float _91 = _88.x;
        float param = _91;
        depths[i] = is_background_depth(param) ? 1000000015047466219876688855040.0 : _91;
    }
    float _118 = fast::max(fast::max(depths[0], depths[1]), fast::max(depths[2], depths[3]));
    if (_118 >= 1000000015047466219876688855040.0)
    {
        out.frag_depth = 1000000015047466219876688855040.0;
        return out;
    }
    float _132 = fast::max(_29.mip.w, 9.9999997473787516355514526367188e-05);
    float _148 = (-1.48800384998321533203125) / _132;
    float _153 = ((_132 * 0.42070877552032470703125) / (_132 * 0.67204129695892333984375)) + 1.0;
    float sum = 0.0;
    float weight_sum = 0.0;
    for (int i_1 = 0; i_1 < 4; i_1++)
    {
        float _169 = fast::min(depths[i_1], _118);
        float _178 = fast::clamp(((_118 - _169) * _148) + _153, 0.0, 1.0);
        sum += (_169 * _178);
        weight_sum += _178;
    }
    out.frag_depth = sum / fast::max(weight_sum, 9.9999999747524270787835121154785e-07);
    return out;
}

