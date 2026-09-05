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
    float4 frag_color [[color(0)]];
    float4 frag_ambient [[color(1)]];
};

struct main0_in
{
    float2 v_ndc [[user(locn0)]];
};

fragment main0_out main0(main0_in in [[stage_in]], constant scene_fs& sc [[buffer(1)]], texturecube<float> env_cube [[texture(0)]], sampler ibl_sampler [[sampler(1)]])
{
    main0_out out = {};
    float4 _41 = sc.inv_view_projection * float4(in.v_ndc, (sc.projection_params.x > 0.5) ? 0.0 : 1.0, 1.0);
    float3 _60 = fast::normalize((_41.xyz / float3(_41.w)) - sc.camera_position.xyz);
    float _65 = -sc.projection_params.y;
    float _68 = sin(_65);
    float _71 = cos(_65);
    float _75 = _60.x;
    float _80 = _60.z;
    out.frag_color = float4(fast::min(env_cube.sample(ibl_sampler, float3((_71 * _75) + (_68 * _80), _60.y, ((-_68) * _75) + (_71 * _80)), level(0.0)).xyz * sc.env_params.y, float3(65504.0)), 1.0);
    out.frag_ambient = float4(0.0);
    return out;
}

