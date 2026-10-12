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

struct LineVertex
{
    float px;
    float py;
    float pz;
    float nx;
    float ny;
    float nz;
    float u;
    float v;
    float tx;
    float ty;
    float tz;
    float tw;
    float r;
    float g;
    float b;
    float a;
    uint d0;
    uint d1;
    uint d2;
    uint d3;
};

struct line_vertices
{
    LineVertex line_vertex[1];
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

struct LineIndex
{
    uint index;
};

struct line_indices
{
    LineIndex line_index[1];
};

struct main0_out
{
    float3 v_corner_a [[user(locn0)]];
    float3 v_corner_b [[user(locn1)]];
    float3 v_corner_c [[user(locn2)]];
    float4 gl_Position [[position]];
};

static inline __attribute__((always_inline))
float3 palette_point(thread const PaletteEntry& m, thread const float3& p)
{
    float4 _43 = float4(p, 1.0);
    return float3(dot(m.r0, _43), dot(m.r1, _43), dot(m.r2, _43));
}

static inline __attribute__((always_inline))
float3 palette_direction(thread const PaletteEntry& m, thread const float3& v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

static inline __attribute__((always_inline))
void apply_deform(thread const uint4& lane, thread float3& position, thread float3& normal, thread float3& tangent, const device morph_deltas& _108, const device morph_weights& _141, const device deform_influences& _204, const device deform_palette& _221)
{
    bool _88 = dot(normal, normal) > 9.9999999600419720025001879548654e-13;
    for (uint m = 0u; m < lane.w; m++)
    {
        uint _113 = lane.z + m;
        position += (float3(_108.morphs[_113].px, _108.morphs[_113].py, _108.morphs[_113].pz) * _141.weights[_108.morphs[_113].shape].value);
        if (_88)
        {
            normal += (float3(_108.morphs[_113].nx, _108.morphs[_113].ny, _108.morphs[_113].nz) * _141.weights[_108.morphs[_113].shape].value);
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
        uint _208 = lane.x + i;
        PaletteEntry _525 = PaletteEntry{ _221.palette[_204.influences[_208].entry].r0, _221.palette[_204.influences[_208].entry].r1, _221.palette[_204.influences[_208].entry].r2 };
        PaletteEntry param = _525;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _204.influences[_208].weight);
        PaletteEntry param_2 = _525;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _204.influences[_208].weight);
        PaletteEntry param_4 = _525;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _204.influences[_208].weight);
        total += _204.influences[_208].weight;
    }
    if (total <= 0.0)
    {
        return;
    }
    if (abs(total - 1.0) > 9.9999999747524270787835121154785e-07)
    {
        float _283 = 1.0 / total;
        blended_position *= _283;
        blended_normal *= _283;
        blended_tangent *= _283;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

static inline __attribute__((always_inline))
float3 line_corner_position(thread const uint& vertex0, const device morph_deltas& _108, const device morph_weights& _141, const device deform_influences& _204, const device deform_palette& _221, const device line_vertices& _303, constant scene_vs& su)
{
    float3 position = float3(_303.line_vertex[vertex0].px, _303.line_vertex[vertex0].py, _303.line_vertex[vertex0].pz);
    bool _386 = su.camera_position.w > 0.5;
    bool _395;
    if (_386)
    {
        _395 = (_303.line_vertex[vertex0].d1 | _303.line_vertex[vertex0].d3) != 0u;
    }
    else
    {
        _395 = _386;
    }
    if (_395)
    {
        uint4 param = uint4(_303.line_vertex[vertex0].d0, _303.line_vertex[vertex0].d1, _303.line_vertex[vertex0].d2, _303.line_vertex[vertex0].d3);
        float3 param_1 = position;
        float3 param_2 = float3(0.0);
        float3 param_3 = float3(0.0);
        apply_deform(param, param_1, param_2, param_3, _108, _141, _204, _221);
        position = param_1;
    }
    return position;
}

vertex main0_out main0(constant scene_vs& su [[buffer(0)]], const device deform_influences& _204 [[buffer(8)]], const device deform_palette& _221 [[buffer(9)]], const device morph_deltas& _108 [[buffer(10)]], const device morph_weights& _141 [[buffer(11)]], const device line_vertices& _303 [[buffer(12)]], const device line_indices& _431 [[buffer(13)]], uint gl_VertexIndex [[vertex_id]])
{
    main0_out out = {};
    uint _419 = uint(int(gl_VertexIndex));
    uint _422 = _419 / 3u;
    uint _433 = 3u * _422;
    uint param = _431.line_index[_433].index;
    float4 _443 = su.view_projection * float4(line_corner_position(param, _108, _141, _204, _221, _303, su), 1.0);
    uint param_1 = _431.line_index[_433 + 1u].index;
    float4 _458 = su.view_projection * float4(line_corner_position(param_1, _108, _141, _204, _221, _303, su), 1.0);
    uint param_2 = _431.line_index[_433 + 2u].index;
    float4 _473 = su.view_projection * float4(line_corner_position(param_2, _108, _141, _204, _221, _303, su), 1.0);
    uint _478 = _419 - (_422 * 3u);
    float4 _485;
    if (_478 == 0u)
    {
        _485 = _443;
    }
    else
    {
        _485 = select(_473, _458, bool4(_478 == 1u));
    }
    out.gl_Position = _485;
    out.v_corner_a = _443.xyw;
    out.v_corner_b = _458.xyw;
    out.v_corner_c = _473.xyw;
    return out;
}

