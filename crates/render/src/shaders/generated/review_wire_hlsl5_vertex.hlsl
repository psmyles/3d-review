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

struct MorphWeight
{
    float value;
};

struct InfluenceEntry
{
    uint entry;
    float weight;
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

struct LineIndex
{
    uint index;
};

static const float2 _435[6] = { float2(0.0f, -1.0f), float2(1.0f, -1.0f), 1.0f.xx, float2(0.0f, -1.0f), 1.0f.xx, float2(0.0f, 1.0f) };

ByteAddressBuffer _119 : register(t2);
ByteAddressBuffer _152 : register(t3);
ByteAddressBuffer _215 : register(t0);
ByteAddressBuffer _232 : register(t1);
ByteAddressBuffer _314 : register(t4);
cbuffer scene_vs : register(b0)
{
    row_major float4x4 su_view_projection : packoffset(c0);
    row_major float4x4 su_inv_view_projection : packoffset(c4);
    float4 su_render_options : packoffset(c8);
    float4 su_camera_position : packoffset(c9);
    float4 su_env_params : packoffset(c10);
    float4 su_projection_params : packoffset(c11);
    row_major float4x4 su_view : packoffset(c12);
    float4 su_selection_color : packoffset(c16);
};

cbuffer line_params : register(b3)
{
    float4 lu_params : packoffset(c0);
};

ByteAddressBuffer _644 : register(t5);

static float4 gl_Position;
static int gl_VertexIndex;
static float4 v_line;
static float4 v_color;

struct SPIRV_Cross_Input
{
    uint gl_VertexIndex : SV_VertexID;
};

struct SPIRV_Cross_Output
{
    noperspective float4 v_line : TEXCOORD0;
    float4 v_color : TEXCOORD1;
    float4 gl_Position : SV_Position;
};

float3 palette_point(PaletteEntry m, float3 p)
{
    float4 _54 = float4(p, 1.0f);
    return float3(dot(m.r0, _54), dot(m.r1, _54), dot(m.r2, _54));
}

float3 palette_direction(PaletteEntry m, float3 v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

void apply_deform(uint4 lane, inout float3 position, inout float3 normal, inout float3 tangent)
{
    bool _99 = dot(normal, normal) > 9.9999999600419720025001879548654e-13f;
    for (uint m = 0u; m < lane.w; m++)
    {
        MorphEntry _127;
        _127.shape = _119.Load((lane.z + m) * 28 + 0);
        _127.px = asfloat(_119.Load((lane.z + m) * 28 + 4));
        _127.py = asfloat(_119.Load((lane.z + m) * 28 + 8));
        _127.pz = asfloat(_119.Load((lane.z + m) * 28 + 12));
        _127.nx = asfloat(_119.Load((lane.z + m) * 28 + 16));
        _127.ny = asfloat(_119.Load((lane.z + m) * 28 + 20));
        _127.nz = asfloat(_119.Load((lane.z + m) * 28 + 24));
        position += (float3(_127.px, _127.py, _127.pz) * asfloat(_152.Load(_127.shape * 4 + 0)));
        if (_99)
        {
            normal += (float3(_127.nx, _127.ny, _127.nz) * asfloat(_152.Load(_127.shape * 4 + 0)));
        }
    }
    if (lane.y == 0u)
    {
        return;
    }
    float3 blended_position = 0.0f.xxx;
    float3 blended_normal = 0.0f.xxx;
    float3 blended_tangent = 0.0f.xxx;
    float total = 0.0f;
    for (uint i = 0u; i < lane.y; i++)
    {
        InfluenceEntry _222;
        _222.entry = _215.Load((lane.x + i) * 8 + 0);
        _222.weight = asfloat(_215.Load((lane.x + i) * 8 + 4));
        PaletteEntry _237;
        _237.r0 = asfloat(_232.Load4(_222.entry * 48 + 0));
        _237.r1 = asfloat(_232.Load4(_222.entry * 48 + 16));
        _237.r2 = asfloat(_232.Load4(_222.entry * 48 + 32));
        PaletteEntry _700 = { _237.r0, _237.r1, _237.r2 };
        PaletteEntry param = _700;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _222.weight);
        PaletteEntry param_2 = _700;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _222.weight);
        PaletteEntry param_4 = _700;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _222.weight);
        total += _222.weight;
    }
    if (total <= 0.0f)
    {
        return;
    }
    if (abs(total - 1.0f) > 9.9999999747524270787835121154785e-07f)
    {
        float _294 = 1.0f / total;
        blended_position *= _294;
        blended_normal *= _294;
        blended_tangent *= _294;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

float3 line_corner_position(uint vertex)
{
    LineVertex _318;
    _318.px = asfloat(_314.Load(vertex * 80 + 0));
    _318.py = asfloat(_314.Load(vertex * 80 + 4));
    _318.pz = asfloat(_314.Load(vertex * 80 + 8));
    _318.nx = asfloat(_314.Load(vertex * 80 + 12));
    _318.ny = asfloat(_314.Load(vertex * 80 + 16));
    _318.nz = asfloat(_314.Load(vertex * 80 + 20));
    _318.u = asfloat(_314.Load(vertex * 80 + 24));
    _318.v = asfloat(_314.Load(vertex * 80 + 28));
    _318.tx = asfloat(_314.Load(vertex * 80 + 32));
    _318.ty = asfloat(_314.Load(vertex * 80 + 36));
    _318.tz = asfloat(_314.Load(vertex * 80 + 40));
    _318.tw = asfloat(_314.Load(vertex * 80 + 44));
    _318.r = asfloat(_314.Load(vertex * 80 + 48));
    _318.g = asfloat(_314.Load(vertex * 80 + 52));
    _318.b = asfloat(_314.Load(vertex * 80 + 56));
    _318.a = asfloat(_314.Load(vertex * 80 + 60));
    _318.d0 = _314.Load(vertex * 80 + 64);
    _318.d1 = _314.Load(vertex * 80 + 68);
    _318.d2 = _314.Load(vertex * 80 + 72);
    _318.d3 = _314.Load(vertex * 80 + 76);
    float3 position = float3(_318.px, _318.py, _318.pz);
    bool _397 = su_camera_position.w > 0.5f;
    bool _406;
    if (_397)
    {
        _406 = (_318.d1 | _318.d3) != 0u;
    }
    else
    {
        _406 = _397;
    }
    if (_406)
    {
        uint4 param = uint4(_318.d0, _318.d1, _318.d2, _318.d3);
        float3 param_1 = position;
        float3 param_2 = 0.0f.xxx;
        float3 param_3 = 0.0f.xxx;
        apply_deform(param, param_1, param_2, param_3);
        position = param_1;
    }
    return position;
}

float2 line_quad_corner(uint corner_index)
{
    return _435[corner_index];
}

void emit_line_quad(inout float4 first, inout float4 second, uint corner_index)
{
    uint param = corner_index;
    float2 _445 = line_quad_corner(param);
    float _448 = first.w;
    float _450 = first.z;
    float _451 = _448 - _450;
    float _457 = second.w - second.z;
    bool _459 = _451 < 0.0f;
    bool _461 = _457 < 0.0f;
    if (_459 && _461)
    {
        gl_Position = float4(2.0f, 2.0f, 2.0f, 1.0f);
        v_line = 0.0f.xxxx;
        return;
    }
    if (_459)
    {
        first = lerp(first, second, (_451 / (_451 - _457)).xxxx);
    }
    else
    {
        if (_461)
        {
            second = lerp(second, first, (_457 / (_457 - _451)).xxxx);
        }
    }
    float2 _511 = lu_params.xy * 0.5f;
    float2 _533 = ((second.xy / second.w.xx) * _511) - ((first.xy / first.w.xx) * _511);
    float _536 = length(_533);
    float2 _541;
    if (_536 > 9.9999997473787516355514526367188e-05f)
    {
        _541 = _533 / _536.xx;
    }
    else
    {
        _541 = float2(1.0f, 0.0f);
    }
    float _561 = 0.5f * lu_params.z;
    float _564 = max(_561, 0.5f);
    float _565 = _564 + 0.5f;
    float _568 = _445.x;
    bool _574 = _568 < 0.5f;
    bool4 _578 = _574.xxxx;
    float4 _579 = float4(_578.x ? first.x : second.x, _578.y ? first.y : second.y, _578.z ? first.z : second.z, _578.w ? first.w : second.w);
    float _585 = _445.y * _565;
    float2 _601 = _579.xy + ((((float2(-_541.y, _541.x) * _585) + (_541 * (((_568 * 2.0f) - 1.0f) * _565))) / _511) * _579.w);
    float4 _738 = _579;
    _738.x = _601.x;
    _738.y = _601.y;
    gl_Position = _738;
    float _615;
    if (_574)
    {
        _615 = (-0.5f) - _564;
    }
    else
    {
        _615 = _536 + _565;
    }
    v_line = float4(_585, _615, _536, _561);
}

void vert_main()
{
    uint _632 = uint(gl_VertexIndex);
    uint _635 = _632 / 6u;
    uint _646 = 2u * _635;
    uint param = _644.Load(_646 * 4 + 0);
    uint param_1 = _644.Load((_646 + 1u) * 4 + 0);
    float4 param_2 = mul(float4(line_corner_position(param), 1.0f), su_view_projection);
    float4 param_3 = mul(float4(line_corner_position(param_1), 1.0f), su_view_projection);
    uint param_4 = _632 - (_635 * 6u);
    emit_line_quad(param_2, param_3, param_4);
    v_color = su_selection_color;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_VertexIndex = int(stage_input.gl_VertexIndex);
    vert_main();
    SPIRV_Cross_Output stage_output;
    stage_output.gl_Position = gl_Position;
    stage_output.v_line = v_line;
    stage_output.v_color = v_color;
    return stage_output;
}
