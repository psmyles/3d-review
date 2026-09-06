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

ByteAddressBuffer _104 : register(t2);
ByteAddressBuffer _137 : register(t3);
ByteAddressBuffer _200 : register(t0);
ByteAddressBuffer _217 : register(t1);
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


static float4 gl_Position;
static float3 in_position;
static float3 in_normal;
static float4 in_tangent;
static uint4 in_deform;
static float3 v_normal;
static float2 v_uv;
static float2 in_uv;
static float4 v_color;
static float4 in_color;
static float3 v_world_position;
static float4 v_tangent;

struct SPIRV_Cross_Input
{
    float3 in_position : TEXCOORD0;
    float3 in_normal : TEXCOORD1;
    float2 in_uv : TEXCOORD2;
    float4 in_tangent : TEXCOORD3;
    float4 in_color : TEXCOORD4;
    uint4 in_deform : TEXCOORD5;
};

struct SPIRV_Cross_Output
{
    float3 v_normal : TEXCOORD0;
    float2 v_uv : TEXCOORD1;
    float4 v_color : TEXCOORD2;
    float3 v_world_position : TEXCOORD3;
    float4 v_tangent : TEXCOORD4;
    float4 gl_Position : SV_Position;
};

float3 palette_point(PaletteEntry m, float3 p)
{
    float4 _38 = float4(p, 1.0f);
    return float3(dot(m.r0, _38), dot(m.r1, _38), dot(m.r2, _38));
}

float3 palette_direction(PaletteEntry m, float3 v)
{
    return float3(dot(m.r0.xyz, v), dot(m.r1.xyz, v), dot(m.r2.xyz, v));
}

void apply_deform(uint4 lane, inout float3 position, inout float3 normal, inout float3 tangent)
{
    bool _83 = dot(normal, normal) > 9.9999999600419720025001879548654e-13f;
    for (uint m = 0u; m < lane.w; m++)
    {
        MorphEntry _112;
        _112.shape = _104.Load((lane.z + m) * 28 + 0);
        _112.px = asfloat(_104.Load((lane.z + m) * 28 + 4));
        _112.py = asfloat(_104.Load((lane.z + m) * 28 + 8));
        _112.pz = asfloat(_104.Load((lane.z + m) * 28 + 12));
        _112.nx = asfloat(_104.Load((lane.z + m) * 28 + 16));
        _112.ny = asfloat(_104.Load((lane.z + m) * 28 + 20));
        _112.nz = asfloat(_104.Load((lane.z + m) * 28 + 24));
        position += (float3(_112.px, _112.py, _112.pz) * asfloat(_137.Load(_112.shape * 4 + 0)));
        if (_83)
        {
            normal += (float3(_112.nx, _112.ny, _112.nz) * asfloat(_137.Load(_112.shape * 4 + 0)));
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
        InfluenceEntry _207;
        _207.entry = _200.Load((lane.x + i) * 8 + 0);
        _207.weight = asfloat(_200.Load((lane.x + i) * 8 + 4));
        PaletteEntry _222;
        _222.r0 = asfloat(_217.Load4(_207.entry * 48 + 0));
        _222.r1 = asfloat(_217.Load4(_207.entry * 48 + 16));
        _222.r2 = asfloat(_217.Load4(_207.entry * 48 + 32));
        PaletteEntry _415 = { _222.r0, _222.r1, _222.r2 };
        PaletteEntry param = _415;
        float3 param_1 = position;
        blended_position += (palette_point(param, param_1) * _207.weight);
        PaletteEntry param_2 = _415;
        float3 param_3 = normal;
        blended_normal += (palette_direction(param_2, param_3) * _207.weight);
        PaletteEntry param_4 = _415;
        float3 param_5 = tangent;
        blended_tangent += (palette_direction(param_4, param_5) * _207.weight);
        total += _207.weight;
    }
    if (total <= 0.0f)
    {
        return;
    }
    if (abs(total - 1.0f) > 9.9999999747524270787835121154785e-07f)
    {
        float _279 = 1.0f / total;
        blended_position *= _279;
        blended_normal *= _279;
        blended_tangent *= _279;
    }
    position = blended_position;
    normal = blended_normal;
    tangent = blended_tangent;
}

void vert_main()
{
    float3 position = in_position;
    float3 normal = in_normal;
    float3 tangent = in_tangent.xyz;
    bool _311 = su_camera_position.w > 0.5f;
    bool _323;
    if (_311)
    {
        _323 = (in_deform.y | in_deform.w) != 0u;
    }
    else
    {
        _323 = _311;
    }
    if (_323)
    {
        uint4 param = in_deform;
        float3 param_1 = position;
        float3 param_2 = normal;
        float3 param_3 = tangent;
        apply_deform(param, param_1, param_2, param_3);
        position = param_1;
        normal = param_2;
        tangent = param_3;
        float _341 = dot(param_2, param_2);
        if (_341 > 9.9999999600419720025001879548654e-13f)
        {
            normal *= rsqrt(_341);
        }
        float _353 = dot(tangent, tangent);
        if (_353 > 9.9999999600419720025001879548654e-13f)
        {
            tangent *= rsqrt(_353);
        }
    }
    gl_Position = mul(float4(position, 1.0f), su_view_projection);
    v_normal = normal;
    v_uv = in_uv;
    v_color = in_color;
    v_world_position = position;
    v_tangent = float4(tangent, in_tangent.w);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    in_position = stage_input.in_position;
    in_normal = stage_input.in_normal;
    in_tangent = stage_input.in_tangent;
    in_deform = stage_input.in_deform;
    in_uv = stage_input.in_uv;
    in_color = stage_input.in_color;
    vert_main();
    SPIRV_Cross_Output stage_output;
    stage_output.gl_Position = gl_Position;
    stage_output.v_normal = v_normal;
    stage_output.v_uv = v_uv;
    stage_output.v_color = v_color;
    stage_output.v_world_position = v_world_position;
    stage_output.v_tangent = v_tangent;
    return stage_output;
}
