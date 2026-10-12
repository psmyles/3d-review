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

ByteAddressBuffer _108 : register(t2);
ByteAddressBuffer _141 : register(t3);
ByteAddressBuffer _204 : register(t0);
ByteAddressBuffer _221 : register(t1);
ByteAddressBuffer _303 : register(t4);
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

ByteAddressBuffer _431 : register(t5);

static float4 gl_Position;
static int gl_VertexIndex;
static float3 v_corner_a;
static float3 v_corner_b;
static float3 v_corner_c;

struct SPIRV_Cross_Input
{
    uint gl_VertexIndex : SV_VertexID;
};

struct SPIRV_Cross_Output
{
    nointerpolation float3 v_corner_a : TEXCOORD0;
    nointerpolation float3 v_corner_b : TEXCOORD1;
    nointerpolation float3 v_corner_c : TEXCOORD2;
    float4 gl_Position : SV_Position;
};

float3 palette_point(PaletteEntry m, float3 p)
{
    float4 _43 = float4(p, 1.0f);
    return float3(dot(m.r0, _43), dot(m.r1, _43), dot(m.r2, _43));
}

float3 palette_direction(PaletteEntry m, float3 v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

void apply_deform(uint4 lane, inout float3 position, inout float3 normal, inout float3 tangent)
{
    bool _88 = dot(normal, normal) > 9.9999999600419720025001879548654e-13f;
    for (uint m = 0u; m < lane.w; m++)
    {
        MorphEntry _116;
        _116.shape = _108.Load((lane.z + m) * 28 + 0);
        _116.px = asfloat(_108.Load((lane.z + m) * 28 + 4));
        _116.py = asfloat(_108.Load((lane.z + m) * 28 + 8));
        _116.pz = asfloat(_108.Load((lane.z + m) * 28 + 12));
        _116.nx = asfloat(_108.Load((lane.z + m) * 28 + 16));
        _116.ny = asfloat(_108.Load((lane.z + m) * 28 + 20));
        _116.nz = asfloat(_108.Load((lane.z + m) * 28 + 24));
        position += (float3(_116.px, _116.py, _116.pz) * asfloat(_141.Load(_116.shape * 4 + 0)));
        if (_88)
        {
            normal += (float3(_116.nx, _116.ny, _116.nz) * asfloat(_141.Load(_116.shape * 4 + 0)));
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
        InfluenceEntry _211;
        _211.entry = _204.Load((lane.x + i) * 8 + 0);
        _211.weight = asfloat(_204.Load((lane.x + i) * 8 + 4));
        PaletteEntry _226;
        _226.r0 = asfloat(_221.Load4(_211.entry * 48 + 0));
        _226.r1 = asfloat(_221.Load4(_211.entry * 48 + 16));
        _226.r2 = asfloat(_221.Load4(_211.entry * 48 + 32));
        PaletteEntry _525 = { _226.r0, _226.r1, _226.r2 };
        PaletteEntry param = _525;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _211.weight);
        PaletteEntry param_2 = _525;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _211.weight);
        PaletteEntry param_4 = _525;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _211.weight);
        total += _211.weight;
    }
    if (total <= 0.0f)
    {
        return;
    }
    if (abs(total - 1.0f) > 9.9999999747524270787835121154785e-07f)
    {
        float _283 = 1.0f / total;
        blended_position *= _283;
        blended_normal *= _283;
        blended_tangent *= _283;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

float3 line_corner_position(uint vertex)
{
    LineVertex _307;
    _307.px = asfloat(_303.Load(vertex * 80 + 0));
    _307.py = asfloat(_303.Load(vertex * 80 + 4));
    _307.pz = asfloat(_303.Load(vertex * 80 + 8));
    _307.nx = asfloat(_303.Load(vertex * 80 + 12));
    _307.ny = asfloat(_303.Load(vertex * 80 + 16));
    _307.nz = asfloat(_303.Load(vertex * 80 + 20));
    _307.u = asfloat(_303.Load(vertex * 80 + 24));
    _307.v = asfloat(_303.Load(vertex * 80 + 28));
    _307.tx = asfloat(_303.Load(vertex * 80 + 32));
    _307.ty = asfloat(_303.Load(vertex * 80 + 36));
    _307.tz = asfloat(_303.Load(vertex * 80 + 40));
    _307.tw = asfloat(_303.Load(vertex * 80 + 44));
    _307.r = asfloat(_303.Load(vertex * 80 + 48));
    _307.g = asfloat(_303.Load(vertex * 80 + 52));
    _307.b = asfloat(_303.Load(vertex * 80 + 56));
    _307.a = asfloat(_303.Load(vertex * 80 + 60));
    _307.d0 = _303.Load(vertex * 80 + 64);
    _307.d1 = _303.Load(vertex * 80 + 68);
    _307.d2 = _303.Load(vertex * 80 + 72);
    _307.d3 = _303.Load(vertex * 80 + 76);
    float3 position = float3(_307.px, _307.py, _307.pz);
    bool _386 = su_camera_position.w > 0.5f;
    bool _395;
    if (_386)
    {
        _395 = (_307.d1 | _307.d3) != 0u;
    }
    else
    {
        _395 = _386;
    }
    if (_395)
    {
        uint4 param = uint4(_307.d0, _307.d1, _307.d2, _307.d3);
        float3 param_1 = position;
        float3 param_2 = 0.0f.xxx;
        float3 param_3 = 0.0f.xxx;
        apply_deform(param, param_1, param_2, param_3);
        position = param_1;
    }
    return position;
}

void vert_main()
{
    uint _419 = uint(gl_VertexIndex);
    uint _422 = _419 / 3u;
    uint _433 = 3u * _422;
    uint param = _431.Load(_433 * 4 + 0);
    float4 _443 = mul(float4(line_corner_position(param), 1.0f), su_view_projection);
    uint param_1 = _431.Load((_433 + 1u) * 4 + 0);
    float4 _458 = mul(float4(line_corner_position(param_1), 1.0f), su_view_projection);
    uint param_2 = _431.Load((_433 + 2u) * 4 + 0);
    float4 _473 = mul(float4(line_corner_position(param_2), 1.0f), su_view_projection);
    uint _478 = _419 - (_422 * 3u);
    float4 _485;
    if (_478 == 0u)
    {
        _485 = _443;
    }
    else
    {
        bool4 _495 = (_478 == 1u).xxxx;
        _485 = float4(_495.x ? _458.x : _473.x, _495.y ? _458.y : _473.y, _495.z ? _458.z : _473.z, _495.w ? _458.w : _473.w);
    }
    gl_Position = _485;
    v_corner_a = _443.xyw;
    v_corner_b = _458.xyw;
    v_corner_c = _473.xyw;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_VertexIndex = int(stage_input.gl_VertexIndex);
    vert_main();
    SPIRV_Cross_Output stage_output;
    stage_output.gl_Position = gl_Position;
    stage_output.v_corner_a = v_corner_a;
    stage_output.v_corner_b = v_corner_b;
    stage_output.v_corner_c = v_corner_c;
    return stage_output;
}
