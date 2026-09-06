static const float2 _24[3] = { (-1.0f).xx, float2(3.0f, -1.0f), float2(-1.0f, 3.0f) };

cbuffer face_params : register(b0)
{
    float4 _57_forward : packoffset(c0);
    float4 _57_right : packoffset(c1);
    float4 _57_up : packoffset(c2);
    float4 _57_params : packoffset(c3);
};


static float4 gl_Position;
static int gl_VertexIndex;
static float3 v_local_dir;
static float2 v_uv;

struct SPIRV_Cross_Input
{
    uint gl_VertexIndex : SV_VertexID;
};

struct SPIRV_Cross_Output
{
    float3 v_local_dir : TEXCOORD0;
    float2 v_uv : TEXCOORD1;
    float4 gl_Position : SV_Position;
};

float2 fullscreen_corner(int index)
{
    return _24[index];
}

void vert_main()
{
    int param = gl_VertexIndex;
    float2 _36 = fullscreen_corner(param);
    float _47 = _36.x;
    float _48 = _36.y;
    gl_Position = float4(_47, _48, 0.0f, 1.0f);
    v_local_dir = (_57_forward.xyz + (_57_right.xyz * _47)) + (_57_up.xyz * _48);
    v_uv = float2((_47 * 0.5f) + 0.5f, 0.5f - (_48 * 0.5f));
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_VertexIndex = int(stage_input.gl_VertexIndex);
    vert_main();
    SPIRV_Cross_Output stage_output;
    stage_output.gl_Position = gl_Position;
    stage_output.v_local_dir = v_local_dir;
    stage_output.v_uv = v_uv;
    return stage_output;
}
