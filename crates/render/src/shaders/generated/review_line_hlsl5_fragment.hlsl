static float4 frag_color;
static float4 v_color;
static float4 frag_ambient;
static float3 v_normal;
static float2 v_uv;
static float3 v_world_position;
static float4 v_tangent;

struct SPIRV_Cross_Input
{
    float3 v_normal : TEXCOORD0;
    float2 v_uv : TEXCOORD1;
    float4 v_color : TEXCOORD2;
    float3 v_world_position : TEXCOORD3;
    float4 v_tangent : TEXCOORD4;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
    float4 frag_ambient : SV_Target1;
};

float3 srgb_to_linear(float3 c)
{
    return lerp(pow(max((c + 0.054999999701976776123046875f.xxx) * 0.947867333889007568359375f.xxx, 0.0f.xxx), 2.400000095367431640625f.xxx), c * 0.077399380505084991455078125f.xxx, step(c, 0.040449999272823333740234375f.xxx));
}

void frag_main()
{
    float3 param = v_color.xyz;
    frag_color = float4(srgb_to_linear(param), v_color.w);
    frag_ambient = float4(0.0f, 0.0f, 0.0f, v_color.w);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_color = stage_input.v_color;
    v_normal = stage_input.v_normal;
    v_uv = stage_input.v_uv;
    v_world_position = stage_input.v_world_position;
    v_tangent = stage_input.v_tangent;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    stage_output.frag_ambient = frag_ambient;
    return stage_output;
}
