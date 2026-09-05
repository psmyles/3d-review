cbuffer egui_params : register(b0)
{
    float2 _27_screen_size : packoffset(c0);
    float2 _27_egui_pad : packoffset(c0.z);
};


static float4 gl_Position;
static float2 in_pos;
static float2 v_uv;
static float2 in_uv;
static float4 v_color;
static float4 in_color;

struct SPIRV_Cross_Input
{
    float2 in_pos : TEXCOORD0;
    float2 in_uv : TEXCOORD1;
    float4 in_color : TEXCOORD2;
};

struct SPIRV_Cross_Output
{
    float2 v_uv : TEXCOORD0;
    float4 v_color : TEXCOORD1;
    float4 gl_Position : SV_Position;
};

void vert_main()
{
    gl_Position = float4(((2.0f * in_pos.x) / _27_screen_size.x) - 1.0f, 1.0f - ((2.0f * in_pos.y) / _27_screen_size.y), 0.0f, 1.0f);
    v_uv = in_uv;
    v_color = in_color;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    in_pos = stage_input.in_pos;
    in_uv = stage_input.in_uv;
    in_color = stage_input.in_color;
    vert_main();
    SPIRV_Cross_Output stage_output;
    stage_output.gl_Position = gl_Position;
    stage_output.v_uv = v_uv;
    stage_output.v_color = v_color;
    return stage_output;
}
