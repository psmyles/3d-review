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

constant spvUnsafeArray<float2, 6> _436 = spvUnsafeArray<float2, 6>({ float2(0.0, -1.0), float2(1.0, -1.0), float2(1.0), float2(0.0, -1.0), float2(1.0), float2(0.0, 1.0) });

struct main0_out
{
    float4 v_line [[user(locn0)]];
    float4 v_color [[user(locn1)]];
    float4 gl_Position [[position]];
};

static inline __attribute__((always_inline))
float3 palette_point(thread const PaletteEntry& m, thread const float3& p)
{
    float4 _56 = float4(p, 1.0);
    return float3(dot(m.r0, _56), dot(m.r1, _56), dot(m.r2, _56));
}

static inline __attribute__((always_inline))
float3 palette_direction(thread const PaletteEntry& m, thread const float3& v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

static inline __attribute__((always_inline))
void apply_deform(thread const uint4& lane, thread float3& position, thread float3& normal, thread float3& tangent, const device morph_deltas& _120, const device morph_weights& _153, const device deform_influences& _216, const device deform_palette& _233)
{
    bool _100 = dot(normal, normal) > 9.9999999600419720025001879548654e-13;
    for (uint m = 0u; m < lane.w; m++)
    {
        uint _125 = lane.z + m;
        position += (float3(_120.morphs[_125].px, _120.morphs[_125].py, _120.morphs[_125].pz) * _153.weights[_120.morphs[_125].shape].value);
        if (_100)
        {
            normal += (float3(_120.morphs[_125].nx, _120.morphs[_125].ny, _120.morphs[_125].nz) * _153.weights[_120.morphs[_125].shape].value);
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
        uint _220 = lane.x + i;
        PaletteEntry _698 = PaletteEntry{ _233.palette[_216.influences[_220].entry].r0, _233.palette[_216.influences[_220].entry].r1, _233.palette[_216.influences[_220].entry].r2 };
        PaletteEntry param = _698;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _216.influences[_220].weight);
        PaletteEntry param_2 = _698;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _216.influences[_220].weight);
        PaletteEntry param_4 = _698;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _216.influences[_220].weight);
        total += _216.influences[_220].weight;
    }
    if (total <= 0.0)
    {
        return;
    }
    if (abs(total - 1.0) > 9.9999999747524270787835121154785e-07)
    {
        float _295 = 1.0 / total;
        blended_position *= _295;
        blended_normal *= _295;
        blended_tangent *= _295;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

static inline __attribute__((always_inline))
float3 line_corner_position(thread const uint& vertex0, const device morph_deltas& _120, const device morph_weights& _153, const device deform_influences& _216, const device deform_palette& _233, const device line_vertices& _315, constant scene_vs& su)
{
    float3 position = float3(_315.line_vertex[vertex0].px, _315.line_vertex[vertex0].py, _315.line_vertex[vertex0].pz);
    bool _398 = su.camera_position.w > 0.5;
    bool _407;
    if (_398)
    {
        _407 = (_315.line_vertex[vertex0].d1 | _315.line_vertex[vertex0].d3) != 0u;
    }
    else
    {
        _407 = _398;
    }
    if (_407)
    {
        uint4 param = uint4(_315.line_vertex[vertex0].d0, _315.line_vertex[vertex0].d1, _315.line_vertex[vertex0].d2, _315.line_vertex[vertex0].d3);
        float3 param_1 = position;
        float3 param_2 = float3(0.0);
        float3 param_3 = float3(0.0);
        apply_deform(param, param_1, param_2, param_3, _120, _153, _216, _233);
        position = param_1;
    }
    return position;
}

static inline __attribute__((always_inline))
float2 line_quad_corner(thread const int& corner_index)
{
    return _436[corner_index];
}

static inline __attribute__((always_inline))
void emit_line_quad(thread float4& first, thread float4& second, thread const int& corner_index, thread float4& gl_Position, thread float4& v_line, constant line_params& lu)
{
    int param = corner_index;
    float2 _446 = line_quad_corner(param);
    float _449 = first.w;
    float _451 = first.z;
    float _452 = _449 - _451;
    float _458 = second.w - second.z;
    bool _460 = _452 < 0.0;
    bool _462 = _458 < 0.0;
    if (_460 && _462)
    {
        gl_Position = float4(2.0, 2.0, 2.0, 1.0);
        v_line = float4(0.0);
        return;
    }
    if (_460)
    {
        first = mix(first, second, float4(_452 / (_452 - _458)));
    }
    else
    {
        if (_462)
        {
            second = mix(second, first, float4(_458 / (_458 - _452)));
        }
    }
    float2 _512 = lu.params.xy * 0.5;
    float2 _534 = ((second.xy / float2(second.w)) * _512) - ((first.xy / float2(first.w)) * _512);
    float _537 = length(_534);
    float2 _542;
    if (_537 > 9.9999997473787516355514526367188e-05)
    {
        _542 = _534 / float2(_537);
    }
    else
    {
        _542 = float2(1.0, 0.0);
    }
    float _562 = 0.5 * lu.params.z;
    float _565 = fast::max(_562, 0.5);
    float _566 = _565 + 0.5;
    float _569 = _446.x;
    bool _575 = _569 < 0.5;
    float4 _580 = select(second, first, bool4(_575));
    float _586 = _446.y * _566;
    float2 _602 = _580.xy + ((((float2(-_542.y, _542.x) * _586) + (_542 * (((_569 * 2.0) - 1.0) * _566))) / _512) * _580.w);
    float4 _736 = _580;
    _736.x = _602.x;
    _736.y = _602.y;
    gl_Position = _736;
    float _616;
    if (_575)
    {
        _616 = (-0.5) - _565;
    }
    else
    {
        _616 = _537 + _566;
    }
    v_line = float4(_586, _616, _537, _562);
}

vertex main0_out main0(constant scene_vs& su [[buffer(0)]], constant line_params& lu [[buffer(3)]], const device deform_influences& _216 [[buffer(8)]], const device deform_palette& _233 [[buffer(9)]], const device morph_deltas& _120 [[buffer(10)]], const device morph_weights& _153 [[buffer(11)]], const device line_vertices& _315 [[buffer(12)]], const device line_indices& _642 [[buffer(13)]], uint gl_VertexIndex [[vertex_id]])
{
    main0_out out = {};
    int _633 = int(gl_VertexIndex) / 6;
    int _644 = 2 * _633;
    uint param = _642.line_index[_644].index;
    uint param_1 = _642.line_index[_644 + 1].index;
    float4 param_2 = su.view_projection * float4(line_corner_position(param, _120, _153, _216, _233, _315, su), 1.0);
    float4 param_3 = su.view_projection * float4(line_corner_position(param_1, _120, _153, _216, _233, _315, su), 1.0);
    int param_4 = int(gl_VertexIndex) - (_633 * 6);
    emit_line_quad(param_2, param_3, param_4, out.gl_Position, out.v_line, lu);
    out.v_color = su.selection_color;
    return out;
}

