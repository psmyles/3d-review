#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct main0_out
{
    float4 frag_color [[color(0)]];
};

struct main0_in
{
    float3 v_local_dir [[user(locn0)]];
};

static inline __attribute__((always_inline))
float2 dir_to_equirect_uv(thread const float3& dir)
{
    float3 _16 = fast::normalize(dir);
    return float2((precise::atan2(_16.z, _16.x) * 0.15915493667125701904296875) + 0.5, acos(fast::clamp(_16.y, -1.0, 1.0)) * 0.3183098733425140380859375);
}

fragment main0_out main0(main0_in in [[stage_in]], texture2d<float> src_equirect [[texture(0)]], sampler src_sampler [[sampler(0)]])
{
    main0_out out = {};
    float3 param = in.v_local_dir;
    out.frag_color = float4(src_equirect.sample(src_sampler, dir_to_equirect_uv(param), level(0.0)).xyz, 1.0);
    return out;
}

