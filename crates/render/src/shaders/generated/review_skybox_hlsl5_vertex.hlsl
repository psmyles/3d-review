static const float2 _24[3] = { (-1.0f).xx, float2(3.0f, -1.0f), float2(-1.0f, 3.0f) };

static float4 gl_Position;
static int gl_VertexIndex;
static float2 v_ndc;

struct SPIRV_Cross_Input
{
    uint gl_VertexIndex : SV_VertexID;
};

struct SPIRV_Cross_Output
{
    float2 v_ndc : TEXCOORD0;
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
    gl_Position = float4(_36, 0.0f, 1.0f);
    v_ndc = _36;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_VertexIndex = int(stage_input.gl_VertexIndex);
    vert_main();
    SPIRV_Cross_Output stage_output;
    stage_output.gl_Position = gl_Position;
    stage_output.v_ndc = v_ndc;
    return stage_output;
}
