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

fragment main0_out main0(main0_in in [[stage_in]], texturecube<float> src_cube [[texture(0)]], sampler src_sampler [[sampler(0)]])
{
    main0_out out = {};
    float3 _13 = fast::normalize(in.v_local_dir);
    float3 up_axis = float3(0.0, 1.0, 0.0);
    if (abs(_13.y) > 0.999000012874603271484375)
    {
        up_axis = float3(0.0, 0.0, 1.0);
    }
    float3 _34 = fast::normalize(cross(up_axis, _13));
    float3 _38 = cross(_13, _34);
    float3 irradiance = float3(0.0);
    float samples = 0.0;
    for (float phi = 0.0; phi < 6.283185482025146484375; phi += 0.0500000007450580596923828125)
    {
        for (float theta = 0.0; theta < 1.57079637050628662109375; theta += 0.0500000007450580596923828125)
        {
            float _62 = sin(theta);
            float _72 = cos(theta);
            irradiance += ((fast::min(src_cube.sample(src_sampler, (((_34 * (_62 * cos(phi))) + (_38 * (_62 * sin(phi)))) + (_13 * _72)), level(0.0)).xyz, float3(64.0)) * _72) * _62);
            samples += 1.0;
        }
    }
    float3 _126 = irradiance;
    float3 _131 = (_126 * 3.1415927410125732421875) / float3(fast::max(samples, 1.0));
    irradiance = _131;
    out.frag_color = float4(_131, 1.0);
    return out;
}

