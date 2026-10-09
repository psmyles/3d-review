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

static const float2 _494[6] = { float2(0.0f, -1.0f), float2(1.0f, -1.0f), 1.0f.xx, float2(0.0f, -1.0f), 1.0f.xx, float2(0.0f, 1.0f) };

ByteAddressBuffer _123 : register(t2);
ByteAddressBuffer _156 : register(t3);
ByteAddressBuffer _219 : register(t0);
ByteAddressBuffer _236 : register(t1);
ByteAddressBuffer _318 : register(t4);
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
    float4 _58 = float4(p, 1.0f);
    return float3(dot(m.r0, _58), dot(m.r1, _58), dot(m.r2, _58));
}

float3 palette_direction(PaletteEntry m, float3 v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

void apply_deform(uint4 lane, inout float3 position, inout float3 normal, inout float3 tangent)
{
    bool _103 = dot(normal, normal) > 9.9999999600419720025001879548654e-13f;
    for (uint m = 0u; m < lane.w; m++)
    {
        MorphEntry _131;
        _131.shape = _123.Load((lane.z + m) * 28 + 0);
        _131.px = asfloat(_123.Load((lane.z + m) * 28 + 4));
        _131.py = asfloat(_123.Load((lane.z + m) * 28 + 8));
        _131.pz = asfloat(_123.Load((lane.z + m) * 28 + 12));
        _131.nx = asfloat(_123.Load((lane.z + m) * 28 + 16));
        _131.ny = asfloat(_123.Load((lane.z + m) * 28 + 20));
        _131.nz = asfloat(_123.Load((lane.z + m) * 28 + 24));
        position += (float3(_131.px, _131.py, _131.pz) * asfloat(_156.Load(_131.shape * 4 + 0)));
        if (_103)
        {
            normal += (float3(_131.nx, _131.ny, _131.nz) * asfloat(_156.Load(_131.shape * 4 + 0)));
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
        InfluenceEntry _226;
        _226.entry = _219.Load((lane.x + i) * 8 + 0);
        _226.weight = asfloat(_219.Load((lane.x + i) * 8 + 4));
        PaletteEntry _241;
        _241.r0 = asfloat(_236.Load4(_226.entry * 48 + 0));
        _241.r1 = asfloat(_236.Load4(_226.entry * 48 + 16));
        _241.r2 = asfloat(_236.Load4(_226.entry * 48 + 32));
        PaletteEntry _759 = { _241.r0, _241.r1, _241.r2 };
        PaletteEntry param = _759;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _226.weight);
        PaletteEntry param_2 = _759;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _226.weight);
        PaletteEntry param_4 = _759;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _226.weight);
        total += _226.weight;
    }
    if (total <= 0.0f)
    {
        return;
    }
    if (abs(total - 1.0f) > 9.9999999747524270787835121154785e-07f)
    {
        float _298 = 1.0f / total;
        blended_position *= _298;
        blended_normal *= _298;
        blended_tangent *= _298;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

float3 line_corner_position(uint vertex)
{
    LineVertex _322;
    _322.px = asfloat(_318.Load(vertex * 80 + 0));
    _322.py = asfloat(_318.Load(vertex * 80 + 4));
    _322.pz = asfloat(_318.Load(vertex * 80 + 8));
    _322.nx = asfloat(_318.Load(vertex * 80 + 12));
    _322.ny = asfloat(_318.Load(vertex * 80 + 16));
    _322.nz = asfloat(_318.Load(vertex * 80 + 20));
    _322.u = asfloat(_318.Load(vertex * 80 + 24));
    _322.v = asfloat(_318.Load(vertex * 80 + 28));
    _322.tx = asfloat(_318.Load(vertex * 80 + 32));
    _322.ty = asfloat(_318.Load(vertex * 80 + 36));
    _322.tz = asfloat(_318.Load(vertex * 80 + 40));
    _322.tw = asfloat(_318.Load(vertex * 80 + 44));
    _322.r = asfloat(_318.Load(vertex * 80 + 48));
    _322.g = asfloat(_318.Load(vertex * 80 + 52));
    _322.b = asfloat(_318.Load(vertex * 80 + 56));
    _322.a = asfloat(_318.Load(vertex * 80 + 60));
    _322.d0 = _318.Load(vertex * 80 + 64);
    _322.d1 = _318.Load(vertex * 80 + 68);
    _322.d2 = _318.Load(vertex * 80 + 72);
    _322.d3 = _318.Load(vertex * 80 + 76);
    float3 position = float3(_322.px, _322.py, _322.pz);
    bool _401 = su_camera_position.w > 0.5f;
    bool _410;
    if (_401)
    {
        _410 = (_322.d1 | _322.d3) != 0u;
    }
    else
    {
        _410 = _401;
    }
    if (_410)
    {
        uint4 param = uint4(_322.d0, _322.d1, _322.d2, _322.d3);
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
    return _494[corner_index];
}

void emit_line_quad(inout float4 first, inout float4 second, uint corner_index)
{
    uint param = corner_index;
    float2 _504 = line_quad_corner(param);
    float _507 = first.w;
    float _509 = first.z;
    float _510 = _507 - _509;
    float _516 = second.w - second.z;
    bool _518 = _510 < 0.0f;
    bool _520 = _516 < 0.0f;
    if (_518 && _520)
    {
        gl_Position = float4(2.0f, 2.0f, 2.0f, 1.0f);
        v_line = 0.0f.xxxx;
        return;
    }
    if (_518)
    {
        first = lerp(first, second, (_510 / (_510 - _516)).xxxx);
    }
    else
    {
        if (_520)
        {
            second = lerp(second, first, (_516 / (_516 - _510)).xxxx);
        }
    }
    float2 _570 = lu_params.xy * 0.5f;
    float2 _592 = ((second.xy / second.w.xx) * _570) - ((first.xy / first.w.xx) * _570);
    float _595 = length(_592);
    float2 _600;
    if (_595 > 9.9999997473787516355514526367188e-05f)
    {
        _600 = _592 / _595.xx;
    }
    else
    {
        _600 = float2(1.0f, 0.0f);
    }
    float _620 = 0.5f * lu_params.z;
    float _623 = max(_620, 0.5f);
    float _624 = _623 + 0.5f;
    float _627 = _504.x;
    bool _633 = _627 < 0.5f;
    bool4 _637 = _633.xxxx;
    float4 _638 = float4(_637.x ? first.x : second.x, _637.y ? first.y : second.y, _637.z ? first.z : second.z, _637.w ? first.w : second.w);
    float _644 = _504.y * _624;
    float2 _660 = _638.xy + ((((float2(-_600.y, _600.x) * _644) + (_600 * (((_627 * 2.0f) - 1.0f) * _624))) / _570) * _638.w);
    float4 _817 = _638;
    _817.x = _660.x;
    _817.y = _660.y;
    gl_Position = _817;
    float _674;
    if (_633)
    {
        _674 = (-0.5f) - _623;
    }
    else
    {
        _674 = _595 + _624;
    }
    v_line = float4(_644, _674, _595, _620);
}

float4 line_corner_color(uint vertex)
{
    LineVertex _433;
    _433.px = asfloat(_318.Load(vertex * 80 + 0));
    _433.py = asfloat(_318.Load(vertex * 80 + 4));
    _433.pz = asfloat(_318.Load(vertex * 80 + 8));
    _433.nx = asfloat(_318.Load(vertex * 80 + 12));
    _433.ny = asfloat(_318.Load(vertex * 80 + 16));
    _433.nz = asfloat(_318.Load(vertex * 80 + 20));
    _433.u = asfloat(_318.Load(vertex * 80 + 24));
    _433.v = asfloat(_318.Load(vertex * 80 + 28));
    _433.tx = asfloat(_318.Load(vertex * 80 + 32));
    _433.ty = asfloat(_318.Load(vertex * 80 + 36));
    _433.tz = asfloat(_318.Load(vertex * 80 + 40));
    _433.tw = asfloat(_318.Load(vertex * 80 + 44));
    _433.r = asfloat(_318.Load(vertex * 80 + 48));
    _433.g = asfloat(_318.Load(vertex * 80 + 52));
    _433.b = asfloat(_318.Load(vertex * 80 + 56));
    _433.a = asfloat(_318.Load(vertex * 80 + 60));
    _433.d0 = _318.Load(vertex * 80 + 64);
    _433.d1 = _318.Load(vertex * 80 + 68);
    _433.d2 = _318.Load(vertex * 80 + 72);
    _433.d3 = _318.Load(vertex * 80 + 76);
    return float4(_433.r, _433.g, _433.b, _433.a);
}

void vert_main()
{
    uint _691 = uint(gl_VertexIndex);
    uint _694 = _691 / 6u;
    uint _697 = 2u * _694;
    uint param = _697;
    uint param_1 = _697 + 1u;
    uint _726 = _691 - (_694 * 6u);
    float4 param_2 = mul(float4(line_corner_position(param), 1.0f), su_view_projection);
    float4 param_3 = mul(float4(line_corner_position(param_1), 1.0f), su_view_projection);
    uint param_4 = _726;
    emit_line_quad(param_2, param_3, param_4);
    uint param_5 = _726;
    uint param_6 = _697 + uint(line_quad_corner(param_5).x);
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
