#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct scene_fs
{
    float4x4 view_projection;
    float4x4 inv_view_projection;
    float4 render_options;
    float4 camera_position;
    float4 env_params;
    float4 projection_params;
    float4x4 view;
    float4 selection_color;
};

struct main0_out
{
    float4 frag_gbuffer [[color(0)]];
    float frag_depth [[color(1)]];
};

struct main0_in
{
    float3 v_normal [[user(locn0)]];
    float3 v_world_position [[user(locn3)]];
};

fragment main0_out main0(main0_in in [[stage_in]], constant scene_fs& sc [[buffer(1)]])
{
    main0_out out = {};
    if (dot(in.v_normal, in.v_normal) < 9.9999999747524270787835121154785e-07)
    {
        out.frag_gbuffer = float4(0.0);
        out.frag_depth = 1000000015047466219876688855040.0;
        return out;
    }
    float4 _48 = sc.view * float4(in.v_world_position, 1.0);
    float _65 = _48.z;
    out.frag_gbuffer = float4(fast::normalize((sc.view * float4(in.v_normal, 0.0)).xyz), _65);
    out.frag_depth = -_65;
    return out;
}

