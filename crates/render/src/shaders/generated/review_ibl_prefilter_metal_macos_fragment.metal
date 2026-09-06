#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct face_params_fs
{
    float4 forward;
    float4 right;
    float4 up;
    float4 params;
};

struct main0_out
{
    float4 frag_color [[color(0)]];
};

struct main0_in
{
    float3 v_local_dir [[user(locn0)]];
};

static inline __attribute__((always_inline))
float radical_inverse_vdc(thread uint& bits)
{
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 1431655765u) << 1u) | ((bits & 2863311530u) >> 1u);
    bits = ((bits & 858993459u) << 2u) | ((bits & 3435973836u) >> 2u);
    bits = ((bits & 252645135u) << 4u) | ((bits & 4042322160u) >> 4u);
    bits = ((bits & 16711935u) << 8u) | ((bits & 4278255360u) >> 8u);
    return float(bits) * 2.3283064365386962890625e-10;
}

static inline __attribute__((always_inline))
float2 hammersley(thread const uint& i, thread const uint& n)
{
    uint param = i;
    float _88 = radical_inverse_vdc(param);
    return float2(float(i) / float(n), _88);
}

static inline __attribute__((always_inline))
float3 importance_sample_ggx(thread const float2& xi, thread const float3& n, thread const float& roughness)
{
    float _95 = roughness * roughness;
    float _101 = 6.283185482025146484375 * xi.x;
    float _116 = sqrt((1.0 - xi.y) / (1.0 + (((_95 * _95) - 1.0) * xi.y)));
    float _122 = sqrt(1.0 - (_116 * _116));
    float3 up_axis = float3(0.0, 0.0, 1.0);
    if (abs(n.z) > 0.999000012874603271484375)
    {
        up_axis = float3(1.0, 0.0, 0.0);
    }
    float3 _150 = fast::normalize(cross(up_axis, n));
    return fast::normalize(((_150 * (cos(_101) * _122)) + (cross(n, _150) * (sin(_101) * _122))) + (n * _116));
}

fragment main0_out main0(main0_in in [[stage_in]], constant face_params_fs& face [[buffer(1)]], texturecube<float> src_cube [[texture(0)]], sampler src_sampler [[sampler(0)]])
{
    main0_out out = {};
    float3 _176 = fast::normalize(in.v_local_dir);
    float3 prefiltered = float3(0.0);
    float total_weight = 0.0;
    for (uint i = 0u; i < 64u; i++)
    {
        uint param = i;
        uint param_1 = 64u;
        float2 param_2 = hammersley(param, param_1);
        float3 param_3 = _176;
        float param_4 = face.params.x;
        float3 _216 = importance_sample_ggx(param_2, param_3, param_4);
        float3 _227 = fast::normalize((_216 * (2.0 * dot(_176, _216))) - _176);
        float _232 = fast::max(dot(_176, _227), 0.0);
        if (_232 > 0.0)
        {
            prefiltered += (fast::min(src_cube.sample(src_sampler, _227, level(0.0)).xyz, float3(64.0)) * _232);
            total_weight += _232;
        }
    }
    if (total_weight > 0.0)
    {
        prefiltered /= float3(total_weight);
    }
    out.frag_color = float4(prefiltered, 1.0);
    return out;
}

