cbuffer scene_fs : register(b1)
{
    row_major float4x4 sc_view_projection : packoffset(c0);
    row_major float4x4 sc_inv_view_projection : packoffset(c4);
    float4 sc_render_options : packoffset(c8);
    float4 sc_camera_position : packoffset(c9);
    float4 sc_env_params : packoffset(c10);
    float4 sc_projection_params : packoffset(c11);
    row_major float4x4 sc_view : packoffset(c12);
    float4 sc_selection_color : packoffset(c16);
};


static float3 v_normal;
static float4 frag_gbuffer;
static float3 v_world_position;
static float2 v_uv;
static float4 v_color;
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
    float4 frag_gbuffer : SV_Target0;
};

void frag_main()
{
    if (dot(v_normal, v_normal) < 9.9999999747524270787835121154785e-07f)
    {
        frag_gbuffer = 0.0f.xxxx;
        return;
    }
    frag_gbuffer = float4(normalize(mul(float4(v_normal, 0.0f), sc_view).xyz), mul(float4(v_world_position, 1.0f), sc_view).z);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_normal = stage_input.v_normal;
    v_world_position = stage_input.v_world_position;
    v_uv = stage_input.v_uv;
    v_color = stage_input.v_color;
    v_tangent = stage_input.v_tangent;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_gbuffer = frag_gbuffer;
    return stage_output;
}
