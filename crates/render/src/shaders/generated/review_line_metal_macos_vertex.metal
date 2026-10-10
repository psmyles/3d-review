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

constant spvUnsafeArray<float2, 6> _494 = spvUnsafeArray<float2, 6>({ float2(0.0, -1.0), float2(1.0, -1.0), float2(1.0), float2(0.0, -1.0), float2(1.0), float2(0.0, 1.0) });

struct main0_out
{
    float4 v_line [[user(locn0)]];
    float4 v_color [[user(locn1)]];
    float4 gl_Position [[position]];
};

static inline __attribute__((always_inline))
float3 palette_point(thread const PaletteEntry& m, thread const float3& p)
{
    float4 _58 = float4(p, 1.0);
    return float3(dot(m.r0, _58), dot(m.r1, _58), dot(m.r2, _58));
}

static inline __attribute__((always_inline))
float3 palette_direction(thread const PaletteEntry& m, thread const float3& v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

static inline __attribute__((always_inline))
void apply_deform(thread const uint4& lane, thread float3& position, thread float3& normal, thread float3& tangent, const device morph_deltas& _123, const device morph_weights& _156, const device deform_influences& _219, const device deform_palette& _236)
{
    bool _103 = dot(normal, normal) > 9.9999999600419720025001879548654e-13;
    for (uint m = 0u; m < lane.w; m++)
    {
        uint _128 = lane.z + m;
        position += (float3(_123.morphs[_128].px, _123.morphs[_128].py, _123.morphs[_128].pz) * _156.weights[_123.morphs[_128].shape].value);
        if (_103)
        {
            normal += (float3(_123.morphs[_128].nx, _123.morphs[_128].ny, _123.morphs[_128].nz) * _156.weights[_123.morphs[_128].shape].value);
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
        uint _223 = lane.x + i;
        PaletteEntry _759 = PaletteEntry{ _236.palette[_219.influences[_223].entry].r0, _236.palette[_219.influences[_223].entry].r1, _236.palette[_219.influences[_223].entry].r2 };
        PaletteEntry param = _759;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _219.influences[_223].weight);
        PaletteEntry param_2 = _759;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _219.influences[_223].weight);
        PaletteEntry param_4 = _759;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _219.influences[_223].weight);
        total += _219.influences[_223].weight;
    }
    if (total <= 0.0)
    {
        return;
    }
    if (abs(total - 1.0) > 9.9999999747524270787835121154785e-07)
    {
        float _298 = 1.0 / total;
        blended_position *= _298;
        blended_normal *= _298;
        blended_tangent *= _298;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

static inline __attribute__((always_inline))
float3 line_corner_position(thread const uint& vertex0, const device morph_deltas& _123, const device morph_weights& _156, const device deform_influences& _219, const device deform_palette& _236, const device line_vertices& _318, constant scene_vs& su)
{
    float3 position = float3(_318.line_vertex[vertex0].px, _318.line_vertex[vertex0].py, _318.line_vertex[vertex0].pz);
    bool _401 = su.camera_position.w > 0.5;
    bool _410;
    if (_401)
    {
        _410 = (_318.line_vertex[vertex0].d1 | _318.line_vertex[vertex0].d3) != 0u;
    }
    else
    {
        _410 = _401;
    }
    if (_410)
    {
        uint4 param = uint4(_318.line_vertex[vertex0].d0, _318.line_vertex[vertex0].d1, _318.line_vertex[vertex0].d2, _318.line_vertex[vertex0].d3);
        float3 param_1 = position;
        float3 param_2 = float3(0.0);
        float3 param_3 = float3(0.0);
        apply_deform(param, param_1, param_2, param_3, _123, _156, _219, _236);
        position = param_1;
    }
    return position;
}

static inline __attribute__((always_inline))
float2 line_quad_corner(thread const uint& corner_index)
{
    return _494[corner_index];
}

static inline __attribute__((always_inline))
void emit_line_quad(thread float4& first, thread float4& second, thread const uint& corner_index, thread float4& gl_Position, thread float4& v_line, constant line_params& lu)
{
    uint param = corner_index;
    float2 _504 = line_quad_corner(param);
    float _507 = first.w;
    float _509 = first.z;
    float _510 = _507 - _509;
    float _516 = second.w - second.z;
    bool _518 = _510 < 0.0;
    bool _520 = _516 < 0.0;
    if (_518 && _520)
    {
        gl_Position = float4(2.0, 2.0, 2.0, 1.0);
        v_line = float4(0.0);
        return;
    }
    if (_518)
    {
        first = mix(first, second, float4(_510 / (_510 - _516)));
    }
    else
    {
        if (_520)
        {
            second = mix(second, first, float4(_516 / (_516 - _510)));
        }
    }
    float2 _570 = lu.params.xy * 0.5;
    float2 _592 = ((second.xy / float2(second.w)) * _570) - ((first.xy / float2(first.w)) * _570);
    float _595 = length(_592);
    float2 _600;
    if (_595 > 9.9999997473787516355514526367188e-05)
    {
        _600 = _592 / float2(_595);
    }
    else
    {
        _600 = float2(1.0, 0.0);
    }
    float _620 = 0.5 * lu.params.z;
    float _623 = fast::max(_620, 0.5);
    float _624 = _623 + 0.5;
    float _627 = _504.x;
    bool _633 = _627 < 0.5;
    float4 _638 = select(second, first, bool4(_633));
    float _644 = _504.y * _624;
    float2 _660 = _638.xy + ((((float2(-_600.y, _600.x) * _644) + (_600 * (((_627 * 2.0) - 1.0) * _624))) / _570) * _638.w);
    float4 _817 = _638;
    _817.x = _660.x;
    _817.y = _660.y;
    gl_Position = _817;
    float _674;
    if (_633)
    {
        _674 = (-0.5) - _623;
    }
    else
    {
        _674 = _595 + _624;
    }
    v_line = float4(_644, _674, _595, _620);
}

static inline __attribute__((always_inline))
float4 line_corner_color(thread const uint& vertex0, const device line_vertices& _318)
{
    return float4(_318.line_vertex[vertex0].r, _318.line_vertex[vertex0].g, _318.line_vertex[vertex0].b, _318.line_vertex[vertex0].a);
}

vertex main0_out main0(constant scene_vs& su [[buffer(0)]], constant line_params& lu [[buffer(3)]], const device deform_influences& _219 [[buffer(8)]], const device deform_palette& _236 [[buffer(9)]], const device morph_deltas& _123 [[buffer(10)]], const device morph_weights& _156 [[buffer(11)]], const device line_vertices& _318 [[buffer(12)]], uint gl_VertexIndex [[vertex_id]])
{
    main0_out out = {};
    uint _691 = uint(int(gl_VertexIndex));
    uint _694 = _691 / 6u;
    uint _697 = 2u * _694;
    uint param = _697;
    uint param_1 = _697 + 1u;
    uint _726 = _691 - (_694 * 6u);
    float4 param_2 = su.view_projection * float4(line_corner_position(param, _123, _156, _219, _236, _318, su), 1.0);
    float4 param_3 = su.view_projection * float4(line_corner_position(param_1, _123, _156, _219, _236, _318, su), 1.0);
    uint param_4 = _726;
    emit_line_quad(param_2, param_3, param_4, out.gl_Position, out.v_line, lu);
    uint param_5 = _726;
    uint param_6 = _697 + uint(line_quad_corner(param_5).x);
    out.v_color = line_corner_color(param_6, _318);
    return out;
}

