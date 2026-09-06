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

TextureCube<float4> env_cube : register(t0);
SamplerState ibl_sampler : register(s1);

static float2 v_ndc;
static float4 frag_color;
static float4 frag_ambient;

struct SPIRV_Cross_Input
{
    float2 v_ndc : TEXCOORD0;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
    float4 frag_ambient : SV_Target1;
};

void frag_main()
{
    float4 _41 = mul(float4(v_ndc, (sc_projection_params.x > 0.5f) ? 0.0f : 1.0f, 1.0f), sc_inv_view_projection);
    float3 _60 = normalize((_41.xyz / _41.w.xxx) - sc_camera_position.xyz);
    float _65 = -sc_projection_params.y;
    float _68 = sin(_65);
    float _71 = cos(_65);
    float _75 = _60.x;
    float _80 = _60.z;
    frag_color = float4(min(env_cube.SampleLevel(ibl_sampler, float3((_71 * _75) + (_68 * _80), _60.y, ((-_68) * _75) + (_71 * _80)), 0.0f).xyz * sc_env_params.y, 65504.0f.xxx), 1.0f);
    frag_ambient = 0.0f.xxxx;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_ndc = stage_input.v_ndc;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    stage_output.frag_ambient = frag_ambient;
    return stage_output;
}
