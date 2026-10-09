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

static const float2 _436[6] = { float2(0.0f, -1.0f), float2(1.0f, -1.0f), 1.0f.xx, float2(0.0f, -1.0f), 1.0f.xx, float2(0.0f, 1.0f) };

ByteAddressBuffer _120 : register(t2);
ByteAddressBuffer _153 : register(t3);
ByteAddressBuffer _216 : register(t0);
ByteAddressBuffer _233 : register(t1);
ByteAddressBuffer _315 : register(t4);
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

ByteAddressBuffer _642 : register(t5);

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
    float4 _56 = float4(p, 1.0f);
    return float3(dot(m.r0, _56), dot(m.r1, _56), dot(m.r2, _56));
}

float3 palette_direction(PaletteEntry m, float3 v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

void apply_deform(uint4 lane, inout float3 position, inout float3 normal, inout float3 tangent)
{
    bool _100 = dot(normal, normal) > 9.9999999600419720025001879548654e-13f;
    for (uint m = 0u; m < lane.w; m++)
    {
        MorphEntry _128;
        _128.shape = _120.Load((lane.z + m) * 28 + 0);
        _128.px = asfloat(_120.Load((lane.z + m) * 28 + 4));
        _128.py = asfloat(_120.Load((lane.z + m) * 28 + 8));
        _128.pz = asfloat(_120.Load((lane.z + m) * 28 + 12));
        _128.nx = asfloat(_120.Load((lane.z + m) * 28 + 16));
        _128.ny = asfloat(_120.Load((lane.z + m) * 28 + 20));
        _128.nz = asfloat(_120.Load((lane.z + m) * 28 + 24));
        position += (float3(_128.px, _128.py, _128.pz) * asfloat(_153.Load(_128.shape * 4 + 0)));
        if (_100)
        {
            normal += (float3(_128.nx, _128.ny, _128.nz) * asfloat(_153.Load(_128.shape * 4 + 0)));
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
        InfluenceEntry _223;
        _223.entry = _216.Load((lane.x + i) * 8 + 0);
        _223.weight = asfloat(_216.Load((lane.x + i) * 8 + 4));
        PaletteEntry _238;
        _238.r0 = asfloat(_233.Load4(_223.entry * 48 + 0));
        _238.r1 = asfloat(_233.Load4(_223.entry * 48 + 16));
        _238.r2 = asfloat(_233.Load4(_223.entry * 48 + 32));
        PaletteEntry _698 = { _238.r0, _238.r1, _238.r2 };
        PaletteEntry param = _698;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _223.weight);
        PaletteEntry param_2 = _698;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _223.weight);
        PaletteEntry param_4 = _698;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _223.weight);
        total += _223.weight;
    }
    if (total <= 0.0f)
    {
        return;
    }
    if (abs(total - 1.0f) > 9.9999999747524270787835121154785e-07f)
    {
        float _295 = 1.0f / total;
        blended_position *= _295;
        blended_normal *= _295;
        blended_tangent *= _295;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

float3 line_corner_position(uint vertex)
{
    LineVertex _319;
    _319.px = asfloat(_315.Load(vertex * 80 + 0));
    _319.py = asfloat(_315.Load(vertex * 80 + 4));
    _319.pz = asfloat(_315.Load(vertex * 80 + 8));
    _319.nx = asfloat(_315.Load(vertex * 80 + 12));
    _319.ny = asfloat(_315.Load(vertex * 80 + 16));
    _319.nz = asfloat(_315.Load(vertex * 80 + 20));
    _319.u = asfloat(_315.Load(vertex * 80 + 24));
    _319.v = asfloat(_315.Load(vertex * 80 + 28));
    _319.tx = asfloat(_315.Load(vertex * 80 + 32));
    _319.ty = asfloat(_315.Load(vertex * 80 + 36));
    _319.tz = asfloat(_315.Load(vertex * 80 + 40));
    _319.tw = asfloat(_315.Load(vertex * 80 + 44));
    _319.r = asfloat(_315.Load(vertex * 80 + 48));
    _319.g = asfloat(_315.Load(vertex * 80 + 52));
    _319.b = asfloat(_315.Load(vertex * 80 + 56));
    _319.a = asfloat(_315.Load(vertex * 80 + 60));
    _319.d0 = _315.Load(vertex * 80 + 64);
    _319.d1 = _315.Load(vertex * 80 + 68);
    _319.d2 = _315.Load(vertex * 80 + 72);
    _319.d3 = _315.Load(vertex * 80 + 76);
    float3 position = float3(_319.px, _319.py, _319.pz);
    bool _398 = su_camera_position.w > 0.5f;
    bool _407;
    if (_398)
    {
        _407 = (_319.d1 | _319.d3) != 0u;
    }
    else
    {
        _407 = _398;
    }
    if (_407)
    {
        uint4 param = uint4(_319.d0, _319.d1, _319.d2, _319.d3);
        float3 param_1 = position;
        float3 param_2 = 0.0f.xxx;
        float3 param_3 = 0.0f.xxx;
        apply_deform(param, param_1, param_2, param_3);
        position = param_1;
    }
    return position;
}

float2 line_quad_corner(int corner_index)
{
    return _436[corner_index];
}

void emit_line_quad(inout float4 first, inout float4 second, int corner_index)
{
    int param = corner_index;
    float2 _446 = line_quad_corner(param);
    float _449 = first.w;
    float _451 = first.z;
    float _452 = _449 - _451;
    float _458 = second.w - second.z;
    bool _460 = _452 < 0.0f;
    bool _462 = _458 < 0.0f;
    if (_460 && _462)
    {
        gl_Position = float4(2.0f, 2.0f, 2.0f, 1.0f);
        v_line = 0.0f.xxxx;
        return;
    }
    if (_460)
    {
        first = lerp(first, second, (_452 / (_452 - _458)).xxxx);
    }
    else
    {
        if (_462)
        {
            second = lerp(second, first, (_458 / (_458 - _452)).xxxx);
        }
    }
    float2 _512 = lu_params.xy * 0.5f;
    float2 _534 = ((second.xy / second.w.xx) * _512) - ((first.xy / first.w.xx) * _512);
    float _537 = length(_534);
    float2 _542;
    if (_537 > 9.9999997473787516355514526367188e-05f)
    {
        _542 = _534 / _537.xx;
    }
    else
    {
        _542 = float2(1.0f, 0.0f);
    }
    float _562 = 0.5f * lu_params.z;
    float _565 = max(_562, 0.5f);
    float _566 = _565 + 0.5f;
    float _569 = _446.x;
    bool _575 = _569 < 0.5f;
    bool4 _579 = _575.xxxx;
    float4 _580 = float4(_579.x ? first.x : second.x, _579.y ? first.y : second.y, _579.z ? first.z : second.z, _579.w ? first.w : second.w);
    float _586 = _446.y * _566;
    float2 _602 = _580.xy + ((((float2(-_542.y, _542.x) * _586) + (_542 * (((_569 * 2.0f) - 1.0f) * _566))) / _512) * _580.w);
    float4 _736 = _580;
    _736.x = _602.x;
    _736.y = _602.y;
    gl_Position = _736;
    float _616;
    if (_575)
    {
        _616 = (-0.5f) - _565;
    }
    else
    {
        _616 = _537 + _566;
    }
    v_line = float4(_586, _616, _537, _562);
}

void vert_main()
{
    int _633 = gl_VertexIndex / 6;
    int _644 = 2 * _633;
    uint param = _642.Load(_644 * 4 + 0);
    uint param_1 = _642.Load((_644 + 1) * 4 + 0);
    float4 param_2 = mul(float4(line_corner_position(param), 1.0f), su_view_projection);
    float4 param_3 = mul(float4(line_corner_position(param_1), 1.0f), su_view_projection);
    int param_4 = gl_VertexIndex - (_633 * 6);
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
