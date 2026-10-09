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

constant spvUnsafeArray<float2, 6> _495 = spvUnsafeArray<float2, 6>({ float2(0.0, -1.0), float2(1.0, -1.0), float2(1.0), float2(0.0, -1.0), float2(1.0), float2(0.0, 1.0) });

struct main0_out
{
    float4 v_line [[user(locn0)]];
    float4 v_color [[user(locn1)]];
    float4 gl_Position [[position]];
};

static inline __attribute__((always_inline))
float3 palette_point(thread const PaletteEntry& m, thread const float3& p)
{
    float4 _60 = float4(p, 1.0);
    return float3(dot(m.r0, _60), dot(m.r1, _60), dot(m.r2, _60));
}

static inline __attribute__((always_inline))
float3 palette_direction(thread const PaletteEntry& m, thread const float3& v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

static inline __attribute__((always_inline))
void apply_deform(thread const uint4& lane, thread float3& position, thread float3& normal, thread float3& tangent, const device morph_deltas& _124, const device morph_weights& _157, const device deform_influences& _220, const device deform_palette& _237)
{
    bool _104 = dot(normal, normal) > 9.9999999600419720025001879548654e-13;
    for (uint m = 0u; m < lane.w; m++)
    {
        uint _129 = lane.z + m;
        position += (float3(_124.morphs[_129].px, _124.morphs[_129].py, _124.morphs[_129].pz) * _157.weights[_124.morphs[_129].shape].value);
        if (_104)
        {
            normal += (float3(_124.morphs[_129].nx, _124.morphs[_129].ny, _124.morphs[_129].nz) * _157.weights[_124.morphs[_129].shape].value);
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
        uint _224 = lane.x + i;
        PaletteEntry _758 = PaletteEntry{ _237.palette[_220.influences[_224].entry].r0, _237.palette[_220.influences[_224].entry].r1, _237.palette[_220.influences[_224].entry].r2 };
        PaletteEntry param = _758;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _220.influences[_224].weight);
        PaletteEntry param_2 = _758;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _220.influences[_224].weight);
        PaletteEntry param_4 = _758;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _220.influences[_224].weight);
        total += _220.influences[_224].weight;
    }
    if (total <= 0.0)
    {
        return;
    }
    if (abs(total - 1.0) > 9.9999999747524270787835121154785e-07)
    {
        float _299 = 1.0 / total;
        blended_position *= _299;
        blended_normal *= _299;
        blended_tangent *= _299;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

static inline __attribute__((always_inline))
float3 line_corner_position(thread const uint& vertex0, const device morph_deltas& _124, const device morph_weights& _157, const device deform_influences& _220, const device deform_palette& _237, const device line_vertices& _319, constant scene_vs& su)
{
    float3 position = float3(_319.line_vertex[vertex0].px, _319.line_vertex[vertex0].py, _319.line_vertex[vertex0].pz);
    bool _402 = su.camera_position.w > 0.5;
    bool _411;
    if (_402)
    {
        _411 = (_319.line_vertex[vertex0].d1 | _319.line_vertex[vertex0].d3) != 0u;
    }
    else
    {
        _411 = _402;
    }
    if (_411)
    {
        uint4 param = uint4(_319.line_vertex[vertex0].d0, _319.line_vertex[vertex0].d1, _319.line_vertex[vertex0].d2, _319.line_vertex[vertex0].d3);
        float3 param_1 = position;
        float3 param_2 = float3(0.0);
        float3 param_3 = float3(0.0);
        apply_deform(param, param_1, param_2, param_3, _124, _157, _220, _237);
        position = param_1;
    }
    return position;
}

static inline __attribute__((always_inline))
float2 line_quad_corner(thread const int& corner_index)
{
    return _495[corner_index];
}

static inline __attribute__((always_inline))
void emit_line_quad(thread float4& first, thread float4& second, thread const int& corner_index, thread float4& gl_Position, thread float4& v_line, constant line_params& lu)
{
    int param = corner_index;
    float2 _505 = line_quad_corner(param);
    float _508 = first.w;
    float _510 = first.z;
    float _511 = _508 - _510;
    float _517 = second.w - second.z;
    bool _519 = _511 < 0.0;
    bool _521 = _517 < 0.0;
    if (_519 && _521)
    {
        gl_Position = float4(2.0, 2.0, 2.0, 1.0);
        v_line = float4(0.0);
        return;
    }
    if (_519)
    {
        first = mix(first, second, float4(_511 / (_511 - _517)));
    }
    else
    {
        if (_521)
        {
            second = mix(second, first, float4(_517 / (_517 - _511)));
        }
    }
    float2 _571 = lu.params.xy * 0.5;
    float2 _593 = ((second.xy / float2(second.w)) * _571) - ((first.xy / float2(first.w)) * _571);
    float _596 = length(_593);
    float2 _601;
    if (_596 > 9.9999997473787516355514526367188e-05)
    {
        _601 = _593 / float2(_596);
    }
    else
    {
        _601 = float2(1.0, 0.0);
    }
    float _621 = 0.5 * lu.params.z;
    float _624 = fast::max(_621, 0.5);
    float _625 = _624 + 0.5;
    float _628 = _505.x;
    bool _634 = _628 < 0.5;
    float4 _639 = select(second, first, bool4(_634));
    float _645 = _505.y * _625;
    float2 _661 = _639.xy + ((((float2(-_601.y, _601.x) * _645) + (_601 * (((_628 * 2.0) - 1.0) * _625))) / _571) * _639.w);
    float4 _816 = _639;
    _816.x = _661.x;
    _816.y = _661.y;
    gl_Position = _816;
    float _675;
    if (_634)
    {
        _675 = (-0.5) - _624;
    }
    else
    {
        _675 = _596 + _625;
    }
    v_line = float4(_645, _675, _596, _621);
}

static inline __attribute__((always_inline))
float4 line_corner_color(thread const uint& vertex0, const device line_vertices& _319)
{
    return float4(_319.line_vertex[vertex0].r, _319.line_vertex[vertex0].g, _319.line_vertex[vertex0].b, _319.line_vertex[vertex0].a);
}

vertex main0_out main0(constant scene_vs& su [[buffer(0)]], constant line_params& lu [[buffer(3)]], const device deform_influences& _220 [[buffer(8)]], const device deform_palette& _237 [[buffer(9)]], const device morph_deltas& _124 [[buffer(10)]], const device morph_weights& _157 [[buffer(11)]], const device line_vertices& _319 [[buffer(12)]], uint gl_VertexIndex [[vertex_id]])
{
    main0_out out = {};
    int _692 = int(gl_VertexIndex) / 6;
    uint _696 = uint(2 * _692);
    uint param = _696;
    uint param_1 = _696 + 1u;
    int _725 = int(gl_VertexIndex) - (_692 * 6);
    float4 param_2 = su.view_projection * float4(line_corner_position(param, _124, _157, _220, _237, _319, su), 1.0);
    float4 param_3 = su.view_projection * float4(line_corner_position(param_1, _124, _157, _220, _237, _319, su), 1.0);
    int param_4 = _725;
    emit_line_quad(param_2, param_3, param_4, out.gl_Position, out.v_line, lu);
    int param_5 = _725;
    uint param_6 = _696 + uint(line_quad_corner(param_5).x);
    out.v_color = line_corner_color(param_6, _319);
    return out;
}

