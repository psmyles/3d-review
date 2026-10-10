static float4 v_line;
static float4 v_color;
static float4 frag_color;
static float4 frag_ambient;

struct SPIRV_Cross_Input
{
    noperspective float4 v_line : TEXCOORD0;
    float4 v_color : TEXCOORD1;
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
    float _81 = max(v_line.w, 0.5f) + 0.5f;
    float _98 = v_color.w * ((clamp(_81 - abs(v_line.x), 0.0f, 1.0f) * clamp(_81 - max(-v_line.y, v_line.y - v_line.z), 0.0f, 1.0f)) * min(v_line.w * 2.0f, 1.0f));
    float3 param = v_color.xyz;
    frag_color = float4(srgb_to_linear(param), _98);
    frag_ambient = float4(0.0f, 0.0f, 0.0f, _98);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_line = stage_input.v_line;
    v_color = stage_input.v_color;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    stage_output.frag_ambient = frag_ambient;
    return stage_output;
}
