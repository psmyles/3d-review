#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct PaletteEntry
{
    float4 r0;
    float4 r1;
    float4 r2;
};

struct MorphEntry
{
    uint shape;
    float px;
    float py;
    float pz;
    float nx;
    float ny;
    float nz;
};

struct morph_deltas
{
    MorphEntry morphs[1];
};

struct MorphWeight
{
    float value;
};

struct morph_weights
{
    MorphWeight weights[1];
};

struct InfluenceEntry
{
    uint entry;
    float weight;
};

struct deform_influences
{
    InfluenceEntry influences[1];
};

struct PaletteEntry_1
{
    float4 r0;
    float4 r1;
    float4 r2;
};

struct deform_palette
{
    PaletteEntry_1 palette[1];
};

struct scene_vs
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
    float3 v_normal [[user(locn0)]];
    float2 v_uv [[user(locn1)]];
    float4 v_color [[user(locn2)]];
    float3 v_world_position [[user(locn3)]];
    float4 v_tangent [[user(locn4)]];
    float4 gl_Position [[position]];
};

struct main0_in
{
    float3 in_position [[attribute(0)]];
    float3 in_normal [[attribute(1)]];
    float2 in_uv [[attribute(2)]];
    float4 in_tangent [[attribute(3)]];
    float4 in_color [[attribute(4)]];
    uint4 in_deform [[attribute(5)]];
};

static inline __attribute__((always_inline))
float3 palette_point(thread const PaletteEntry& m, thread const float3& p)
{
    float4 _38 = float4(p, 1.0);
    return float3(dot(m.r0, _38), dot(m.r1, _38), dot(m.r2, _38));
}

static inline __attribute__((always_inline))
float3 palette_direction(thread const PaletteEntry& m, thread const float3& v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

static inline __attribute__((always_inline))
void apply_deform(thread const uint4& lane, thread float3& position, thread float3& normal, thread float3& tangent, const device morph_deltas& _104, const device morph_weights& _137, const device deform_influences& _200, const device deform_palette& _217)
{
    bool _83 = dot(normal, normal) > 9.9999999600419720025001879548654e-13;
    for (uint m = 0u; m < lane.w; m++)
    {
        uint _109 = lane.z + m;
        position += (float3(_104.morphs[_109].px, _104.morphs[_109].py, _104.morphs[_109].pz) * _137.weights[_104.morphs[_109].shape].value);
        if (_83)
        {
            normal += (float3(_104.morphs[_109].nx, _104.morphs[_109].ny, _104.morphs[_109].nz) * _137.weights[_104.morphs[_109].shape].value);
        }
    }
    if (lane.y == 0u)
    {
        return;
    }
    float3 blended_position = float3(0.0);
    float3 blended_normal = float3(0.0);
    float3 blended_tangent = float3(0.0);
    float total = 0.0;
    for (uint i = 0u; i < lane.y; i++)
    {
        uint _204 = lane.x + i;
        PaletteEntry _415 = PaletteEntry{ _217.palette[_200.influences[_204].entry].r0, _217.palette[_200.influences[_204].entry].r1, _217.palette[_200.influences[_204].entry].r2 };
        PaletteEntry param = _415;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _200.influences[_204].weight);
        PaletteEntry param_2 = _415;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _200.influences[_204].weight);
        PaletteEntry param_4 = _415;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _200.influences[_204].weight);
        total += _200.influences[_204].weight;
    }
    if (total <= 0.0)
    {
        return;
    }
    if (abs(total - 1.0) > 9.9999999747524270787835121154785e-07)
    {
        float _279 = 1.0 / total;
        blended_position *= _279;
        blended_normal *= _279;
        blended_tangent *= _279;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

vertex main0_out main0(main0_in in [[stage_in]], constant scene_vs& su [[buffer(0)]], const device deform_influences& _200 [[buffer(8)]], const device deform_palette& _217 [[buffer(9)]], const device morph_deltas& _104 [[buffer(10)]], const device morph_weights& _137 [[buffer(11)]])
{
    main0_out out = {};
    float3 position = in.in_position;
    float3 normal = in.in_normal;
    float3 tangent = in.in_tangent.xyz;
    bool _311 = su.camera_position.w > 0.5;
    bool _323;
    if (_311)
    {
        _323 = (in.in_deform.y | in.in_deform.w) != 0u;
    }
    else
    {
        _323 = _311;
    }
    if (_323)
    {
        uint4 param = in.in_deform;
        float3 param_1 = position;
        float3 param_2 = normal;
        float3 param_3 = tangent;
        apply_deform(param, param_1, param_2, param_3, _104, _137, _200, _217);
        position = param_1;
        normal = param_2;
        tangent = param_3;
        float _341 = dot(param_2, param_2);
        if (_341 > 9.9999999600419720025001879548654e-13)
        {
            normal *= rsqrt(_341);
        }
        float _353 = dot(tangent, tangent);
        if (_353 > 9.9999999600419720025001879548654e-13)
        {
            tangent *= rsqrt(_353);
        }
    }
    out.gl_Position = su.view_projection * float4(position, 1.0);
    out.v_normal = normal;
    out.v_uv = in.in_uv;
    out.v_color = in.in_color;
    out.v_world_position = position;
    out.v_tangent = float4(tangent, in.in_tangent.w);
    return out;
}

