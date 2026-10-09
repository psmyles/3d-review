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

struct line_params
{
    float4 params;
};

struct LineIndex
{
    uint index;
};

struct line_indices
{
    LineIndex line_index[1];
};

constant spvUnsafeArray<float2, 6> _435 = spvUnsafeArray<float2, 6>({ float2(0.0, -1.0), float2(1.0, -1.0), float2(1.0), float2(0.0, -1.0), float2(1.0), float2(0.0, 1.0) });

struct main0_out
{
    float4 v_line [[user(locn0)]];
    float4 v_color [[user(locn1)]];
    float4 gl_Position [[position]];
};

static inline __attribute__((always_inline))
float3 palette_point(thread const PaletteEntry& m, thread const float3& p)
{
    float4 _54 = float4(p, 1.0);
    return float3(dot(m.r0, _54), dot(m.r1, _54), dot(m.r2, _54));
}

static inline __attribute__((always_inline))
float3 palette_direction(thread const PaletteEntry& m, thread const float3& v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

static inline __attribute__((always_inline))
void apply_deform(thread const uint4& lane, thread float3& position, thread float3& normal, thread float3& tangent, const device morph_deltas& _119, const device morph_weights& _152, const device deform_influences& _215, const device deform_palette& _232)
{
    bool _99 = dot(normal, normal) > 9.9999999600419720025001879548654e-13;
    for (uint m = 0u; m < lane.w; m++)
    {
        uint _124 = lane.z + m;
        position += (float3(_119.morphs[_124].px, _119.morphs[_124].py, _119.morphs[_124].pz) * _152.weights[_119.morphs[_124].shape].value);
        if (_99)
        {
            normal += (float3(_119.morphs[_124].nx, _119.morphs[_124].ny, _119.morphs[_124].nz) * _152.weights[_119.morphs[_124].shape].value);
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
        uint _219 = lane.x + i;
        PaletteEntry _700 = PaletteEntry{ _232.palette[_215.influences[_219].entry].r0, _232.palette[_215.influences[_219].entry].r1, _232.palette[_215.influences[_219].entry].r2 };
        PaletteEntry param = _700;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _215.influences[_219].weight);
        PaletteEntry param_2 = _700;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _215.influences[_219].weight);
        PaletteEntry param_4 = _700;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _215.influences[_219].weight);
        total += _215.influences[_219].weight;
    }
    if (total <= 0.0)
    {
        return;
    }
    if (abs(total - 1.0) > 9.9999999747524270787835121154785e-07)
    {
        float _294 = 1.0 / total;
        blended_position *= _294;
        blended_normal *= _294;
        blended_tangent *= _294;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

static inline __attribute__((always_inline))
float3 line_corner_position(thread const uint& vertex0, const device morph_deltas& _119, const device morph_weights& _152, const device deform_influences& _215, const device deform_palette& _232, const device line_vertices& _314, constant scene_vs& su)
{
    float3 position = float3(_314.line_vertex[vertex0].px, _314.line_vertex[vertex0].py, _314.line_vertex[vertex0].pz);
    bool _397 = su.camera_position.w > 0.5;
    bool _406;
    if (_397)
    {
        _406 = (_314.line_vertex[vertex0].d1 | _314.line_vertex[vertex0].d3) != 0u;
    }
    else
    {
        _406 = _397;
    }
    if (_406)
    {
        uint4 param = uint4(_314.line_vertex[vertex0].d0, _314.line_vertex[vertex0].d1, _314.line_vertex[vertex0].d2, _314.line_vertex[vertex0].d3);
        float3 param_1 = position;
        float3 param_2 = float3(0.0);
        float3 param_3 = float3(0.0);
        apply_deform(param, param_1, param_2, param_3, _119, _152, _215, _232);
        position = param_1;
    }
    return position;
}

static inline __attribute__((always_inline))
float2 line_quad_corner(thread const uint& corner_index)
{
    return _435[corner_index];
}

static inline __attribute__((always_inline))
void emit_line_quad(thread float4& first, thread float4& second, thread const uint& corner_index, thread float4& gl_Position, thread float4& v_line, constant line_params& lu)
{
    uint param = corner_index;
    float2 _445 = line_quad_corner(param);
    float _448 = first.w;
    float _450 = first.z;
    float _451 = _448 - _450;
    float _457 = second.w - second.z;
    bool _459 = _451 < 0.0;
    bool _461 = _457 < 0.0;
    if (_459 && _461)
    {
        gl_Position = float4(2.0, 2.0, 2.0, 1.0);
        v_line = float4(0.0);
        return;
    }
    if (_459)
    {
        first = mix(first, second, float4(_451 / (_451 - _457)));
    }
    else
    {
        if (_461)
        {
            second = mix(second, first, float4(_457 / (_457 - _451)));
        }
    }
    float2 _511 = lu.params.xy * 0.5;
    float2 _533 = ((second.xy / float2(second.w)) * _511) - ((first.xy / float2(first.w)) * _511);
    float _536 = length(_533);
    float2 _541;
    if (_536 > 9.9999997473787516355514526367188e-05)
    {
        _541 = _533 / float2(_536);
    }
    else
    {
        _541 = float2(1.0, 0.0);
    }
    float _561 = 0.5 * lu.params.z;
    float _564 = fast::max(_561, 0.5);
    float _565 = _564 + 0.5;
    float _568 = _445.x;
    bool _574 = _568 < 0.5;
    float4 _579 = select(second, first, bool4(_574));
    float _585 = _445.y * _565;
    float2 _601 = _579.xy + ((((float2(-_541.y, _541.x) * _585) + (_541 * (((_568 * 2.0) - 1.0) * _565))) / _511) * _579.w);
    float4 _738 = _579;
    _738.x = _601.x;
    _738.y = _601.y;
    gl_Position = _738;
    float _615;
    if (_574)
    {
        _615 = (-0.5) - _564;
    }
    else
    {
        _615 = _536 + _565;
    }
    v_line = float4(_585, _615, _536, _561);
}

vertex main0_out main0(constant scene_vs& su [[buffer(0)]], constant line_params& lu [[buffer(3)]], const device deform_influences& _215 [[buffer(8)]], const device deform_palette& _232 [[buffer(9)]], const device morph_deltas& _119 [[buffer(10)]], const device morph_weights& _152 [[buffer(11)]], const device line_vertices& _314 [[buffer(12)]], const device line_indices& _644 [[buffer(13)]], uint gl_VertexIndex [[vertex_id]])
{
    main0_out out = {};
    uint _632 = uint(int(gl_VertexIndex));
    uint _635 = _632 / 6u;
    uint _646 = 2u * _635;
    uint param = _644.line_index[_646].index;
    uint param_1 = _644.line_index[_646 + 1u].index;
    float4 param_2 = su.view_projection * float4(line_corner_position(param, _119, _152, _215, _232, _314, su), 1.0);
    float4 param_3 = su.view_projection * float4(line_corner_position(param_1, _119, _152, _215, _232, _314, su), 1.0);
    uint param_4 = _632 - (_635 * 6u);
    emit_line_quad(param_2, param_3, param_4, out.gl_Position, out.v_line, lu);
    out.v_color = su.selection_color;
    return out;
}

