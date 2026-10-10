#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct main0_out
{
    float4 frag_color [[color(0)]];
    float4 frag_ambient [[color(1)]];
};

struct main0_in
{
    float4 v_line [[user(locn0), center_no_perspective]];
    float4 v_color [[user(locn1)]];
};

static inline __attribute__((always_inline))
float3 srgb_to_linear(thread const float3& c)
{
    return mix(powr(fast::max((c + float3(0.054999999701976776123046875)) * float3(0.947867333889007568359375), float3(0.0)), float3(2.400000095367431640625)), c * float3(0.077399380505084991455078125), step(c, float3(0.040449999272823333740234375)));
}

fragment main0_out main0(main0_in in [[stage_in]])
{
    main0_out out = {};
    float _81 = fast::max(in.v_line.w, 0.5) + 0.5;
    float _98 = in.v_color.w * ((fast::clamp(_81 - abs(in.v_line.x), 0.0, 1.0) * fast::clamp(_81 - fast::max(-in.v_line.y, in.v_line.y - in.v_line.z), 0.0, 1.0)) * fast::min(in.v_line.w * 2.0, 1.0));
    float3 param = in.v_color.xyz;
    out.frag_color = float4(srgb_to_linear(param), _98);
    out.frag_ambient = float4(0.0, 0.0, 0.0, _98);
    return out;
}

