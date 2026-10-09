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

static const float2 _495[6] = { float2(0.0f, -1.0f), float2(1.0f, -1.0f), 1.0f.xx, float2(0.0f, -1.0f), 1.0f.xx, float2(0.0f, 1.0f) };

ByteAddressBuffer _124 : register(t2);
ByteAddressBuffer _157 : register(t3);
ByteAddressBuffer _220 : register(t0);
ByteAddressBuffer _237 : register(t1);
ByteAddressBuffer _319 : register(t4);
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
    float4 _60 = float4(p, 1.0f);
    return float3(dot(m.r0, _60), dot(m.r1, _60), dot(m.r2, _60));
}

float3 palette_direction(PaletteEntry m, float3 v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

void apply_deform(uint4 lane, inout float3 position, inout float3 normal, inout float3 tangent)
{
    bool _104 = dot(normal, normal) > 9.9999999600419720025001879548654e-13f;
    for (uint m = 0u; m < lane.w; m++)
    {
        MorphEntry _132;
        _132.shape = _124.Load((lane.z + m) * 28 + 0);
        _132.px = asfloat(_124.Load((lane.z + m) * 28 + 4));
        _132.py = asfloat(_124.Load((lane.z + m) * 28 + 8));
        _132.pz = asfloat(_124.Load((lane.z + m) * 28 + 12));
        _132.nx = asfloat(_124.Load((lane.z + m) * 28 + 16));
        _132.ny = asfloat(_124.Load((lane.z + m) * 28 + 20));
        _132.nz = asfloat(_124.Load((lane.z + m) * 28 + 24));
        position += (float3(_132.px, _132.py, _132.pz) * asfloat(_157.Load(_132.shape * 4 + 0)));
        if (_104)
        {
            normal += (float3(_132.nx, _132.ny, _132.nz) * asfloat(_157.Load(_132.shape * 4 + 0)));
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
        InfluenceEntry _227;
        _227.entry = _220.Load((lane.x + i) * 8 + 0);
        _227.weight = asfloat(_220.Load((lane.x + i) * 8 + 4));
        PaletteEntry _242;
        _242.r0 = asfloat(_237.Load4(_227.entry * 48 + 0));
        _242.r1 = asfloat(_237.Load4(_227.entry * 48 + 16));
        _242.r2 = asfloat(_237.Load4(_227.entry * 48 + 32));
        PaletteEntry _758 = { _242.r0, _242.r1, _242.r2 };
        PaletteEntry param = _758;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _227.weight);
        PaletteEntry param_2 = _758;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _227.weight);
        PaletteEntry param_4 = _758;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _227.weight);
        total += _227.weight;
    }
    if (total <= 0.0f)
    {
        return;
    }
    if (abs(total - 1.0f) > 9.9999999747524270787835121154785e-07f)
    {
        float _299 = 1.0f / total;
        blended_position *= _299;
        blended_normal *= _299;
        blended_tangent *= _299;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

float3 line_corner_position(uint vertex)
{
    LineVertex _323;
    _323.px = asfloat(_319.Load(vertex * 80 + 0));
    _323.py = asfloat(_319.Load(vertex * 80 + 4));
    _323.pz = asfloat(_319.Load(vertex * 80 + 8));
    _323.nx = asfloat(_319.Load(vertex * 80 + 12));
    _323.ny = asfloat(_319.Load(vertex * 80 + 16));
    _323.nz = asfloat(_319.Load(vertex * 80 + 20));
    _323.u = asfloat(_319.Load(vertex * 80 + 24));
    _323.v = asfloat(_319.Load(vertex * 80 + 28));
    _323.tx = asfloat(_319.Load(vertex * 80 + 32));
    _323.ty = asfloat(_319.Load(vertex * 80 + 36));
    _323.tz = asfloat(_319.Load(vertex * 80 + 40));
    _323.tw = asfloat(_319.Load(vertex * 80 + 44));
    _323.r = asfloat(_319.Load(vertex * 80 + 48));
    _323.g = asfloat(_319.Load(vertex * 80 + 52));
    _323.b = asfloat(_319.Load(vertex * 80 + 56));
    _323.a = asfloat(_319.Load(vertex * 80 + 60));
    _323.d0 = _319.Load(vertex * 80 + 64);
    _323.d1 = _319.Load(vertex * 80 + 68);
    _323.d2 = _319.Load(vertex * 80 + 72);
    _323.d3 = _319.Load(vertex * 80 + 76);
    float3 position = float3(_323.px, _323.py, _323.pz);
    bool _402 = su_camera_position.w > 0.5f;
    bool _411;
    if (_402)
    {
        _411 = (_323.d1 | _323.d3) != 0u;
    }
    else
    {
        _411 = _402;
    }
    if (_411)
    {
        uint4 param = uint4(_323.d0, _323.d1, _323.d2, _323.d3);
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
    return _495[corner_index];
}

void emit_line_quad(inout float4 first, inout float4 second, int corner_index)
{
    int param = corner_index;
    float2 _505 = line_quad_corner(param);
    float _508 = first.w;
    float _510 = first.z;
    float _511 = _508 - _510;
    float _517 = second.w - second.z;
    bool _519 = _511 < 0.0f;
    bool _521 = _517 < 0.0f;
    if (_519 && _521)
    {
        gl_Position = float4(2.0f, 2.0f, 2.0f, 1.0f);
        v_line = 0.0f.xxxx;
        return;
    }
    if (_519)
    {
        first = lerp(first, second, (_511 / (_511 - _517)).xxxx);
    }
    else
    {
        if (_521)
        {
            second = lerp(second, first, (_517 / (_517 - _511)).xxxx);
        }
    }
    float2 _571 = lu_params.xy * 0.5f;
    float2 _593 = ((second.xy / second.w.xx) * _571) - ((first.xy / first.w.xx) * _571);
    float _596 = length(_593);
    float2 _601;
    if (_596 > 9.9999997473787516355514526367188e-05f)
    {
        _601 = _593 / _596.xx;
    }
    else
    {
        _601 = float2(1.0f, 0.0f);
    }
    float _621 = 0.5f * lu_params.z;
    float _624 = max(_621, 0.5f);
    float _625 = _624 + 0.5f;
    float _628 = _505.x;
    bool _634 = _628 < 0.5f;
    bool4 _638 = _634.xxxx;
    float4 _639 = float4(_638.x ? first.x : second.x, _638.y ? first.y : second.y, _638.z ? first.z : second.z, _638.w ? first.w : second.w);
    float _645 = _505.y * _625;
    float2 _661 = _639.xy + ((((float2(-_601.y, _601.x) * _645) + (_601 * (((_628 * 2.0f) - 1.0f) * _625))) / _571) * _639.w);
    float4 _816 = _639;
    _816.x = _661.x;
    _816.y = _661.y;
    gl_Position = _816;
    float _675;
    if (_634)
    {
        _675 = (-0.5f) - _624;
    }
    else
    {
        _675 = _596 + _625;
    }
    v_line = float4(_645, _675, _596, _621);
}

float4 line_corner_color(uint vertex)
{
    LineVertex _434;
    _434.px = asfloat(_319.Load(vertex * 80 + 0));
    _434.py = asfloat(_319.Load(vertex * 80 + 4));
    _434.pz = asfloat(_319.Load(vertex * 80 + 8));
    _434.nx = asfloat(_319.Load(vertex * 80 + 12));
    _434.ny = asfloat(_319.Load(vertex * 80 + 16));
    _434.nz = asfloat(_319.Load(vertex * 80 + 20));
    _434.u = asfloat(_319.Load(vertex * 80 + 24));
    _434.v = asfloat(_319.Load(vertex * 80 + 28));
    _434.tx = asfloat(_319.Load(vertex * 80 + 32));
    _434.ty = asfloat(_319.Load(vertex * 80 + 36));
    _434.tz = asfloat(_319.Load(vertex * 80 + 40));
    _434.tw = asfloat(_319.Load(vertex * 80 + 44));
    _434.r = asfloat(_319.Load(vertex * 80 + 48));
    _434.g = asfloat(_319.Load(vertex * 80 + 52));
    _434.b = asfloat(_319.Load(vertex * 80 + 56));
    _434.a = asfloat(_319.Load(vertex * 80 + 60));
    _434.d0 = _319.Load(vertex * 80 + 64);
    _434.d1 = _319.Load(vertex * 80 + 68);
    _434.d2 = _319.Load(vertex * 80 + 72);
    _434.d3 = _319.Load(vertex * 80 + 76);
    return float4(_434.r, _434.g, _434.b, _434.a);
}

void vert_main()
{
    int _692 = gl_VertexIndex / 6;
    uint _696 = uint(2 * _692);
    uint param = _696;
    uint param_1 = _696 + 1u;
    int _725 = gl_VertexIndex - (_692 * 6);
    float4 param_2 = mul(float4(line_corner_position(param), 1.0f), su_view_projection);
    float4 param_3 = mul(float4(line_corner_position(param_1), 1.0f), su_view_projection);
    int param_4 = _725;
    emit_line_quad(param_2, param_3, param_4);
    int param_5 = _725;
    uint param_6 = _696 + uint(line_quad_corner(param_5).x);
    v_color = line_corner_color(param_6);
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
