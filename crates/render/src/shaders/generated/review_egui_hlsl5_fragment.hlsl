Texture2D<float4> egui_texture : register(t0);
SamplerState egui_sampler : register(s0);

static float4 frag_color;
static float4 v_color;
static float2 v_uv;

struct SPIRV_Cross_Input
{
    float2 v_uv : TEXCOORD0;
    float4 v_color : TEXCOORD1;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
};

void frag_main()
{
    frag_color = v_color * egui_texture.Sample(egui_sampler, v_uv);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_color = stage_input.v_color;
    v_uv = stage_input.v_uv;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    return stage_output;
}
