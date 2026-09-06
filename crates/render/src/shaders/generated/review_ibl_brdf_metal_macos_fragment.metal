#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct main0_out
{
    float2 frag_brdf [[color(0)]];
};

struct main0_in
{
    float2 v_uv [[user(locn1)]];
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
    float _99 = radical_inverse_vdc(param);
    return float2(float(i) / float(n), _99);
}

static inline __attribute__((always_inline))
float3 importance_sample_ggx(thread const float2& xi, thread const float3& n, thread const float& roughness)
{
    float _106 = roughness * roughness;
    float _112 = 6.283185482025146484375 * xi.x;
    float _127 = sqrt((1.0 - xi.y) / (1.0 + (((_106 * _106) - 1.0) * xi.y)));
    float _133 = sqrt(1.0 - (_127 * _127));
    float3 up_axis = float3(0.0, 0.0, 1.0);
    if (abs(n.z) > 0.999000012874603271484375)
    {
        up_axis = float3(1.0, 0.0, 0.0);
    }
    float3 _161 = fast::normalize(cross(up_axis, n));
    return fast::normalize(((_161 * (cos(_112) * _133)) + (cross(n, _161) * (sin(_112) * _133))) + (n * _127));
}

static inline __attribute__((always_inline))
float geometry_schlick_ggx(thread const float& n_dot_v, thread const float& roughness)
{
    float _188 = (roughness * roughness) * 0.5;
    return n_dot_v / ((n_dot_v * (1.0 - _188)) + _188);
}

static inline __attribute__((always_inline))
float geometry_smith(thread const float& n_dot_v, thread const float& n_dot_l, thread const float& roughness)
{
    float param = n_dot_v;
    float param_1 = roughness;
    float param_2 = n_dot_l;
    float param_3 = roughness;
    return geometry_schlick_ggx(param, param_1) * geometry_schlick_ggx(param_2, param_3);
}

fragment main0_out main0(main0_in in [[stage_in]])
{
    main0_out out = {};
    float _219 = fast::max(in.v_uv.x, 9.9999997473787516355514526367188e-05);
    float3 _230 = float3(sqrt(1.0 - (_219 * _219)), 0.0, _219);
    float a = 0.0;
    float b = 0.0;
    for (uint i = 0u; i < 512u; i++)
    {
        uint param = i;
        uint param_1 = 512u;
        float2 param_2 = hammersley(param, param_1);
        float3 param_3 = float3(0.0, 0.0, 1.0);
        float param_4 = in.v_uv.y;
        float3 _258 = importance_sample_ggx(param_2, param_3, param_4);
        float _262 = dot(_230, _258);
        float _272 = fast::max(fast::normalize((_258 * (2.0 * _262)) - _230).z, 0.0);
        float _281 = fast::max(_262, 0.0);
        if (_272 > 0.0)
        {
            float param_5 = _219;
            float param_6 = _272;
            float param_7 = in.v_uv.y;
            float _301 = (geometry_smith(param_5, param_6, param_7) * _281) / (fast::max(_258.z, 0.0) * _219);
            float _307 = powr(fast::max(1.0 - _281, 0.0), 5.0);
            a += ((1.0 - _307) * _301);
            b += (_307 * _301);
        }
    }
    out.frag_brdf = float2(a, b) * float2(0.001953125);
    return out;
}

