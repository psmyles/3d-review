Texture2D<float4> src_equirect : register(t0);
SamplerState src_sampler : register(s0);

static float3 v_local_dir;
static float4 frag_color;
static float2 v_uv;

struct SPIRV_Cross_Input
{
    float3 v_local_dir : TEXCOORD0;
    float2 v_uv : TEXCOORD1;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
};

float2 dir_to_equirect_uv(float3 dir)
{
    float3 _16 = normalize(dir);
    return float2((atan2(_16.z, _16.x) * 0.15915493667125701904296875f) + 0.5f, acos(clamp(_16.y, -1.0f, 1.0f)) * 0.3183098733425140380859375f);
}

void frag_main()
{
    float3 param = v_local_dir;
    frag_color = float4(src_equirect.SampleLevel(src_sampler, dir_to_equirect_uv(param), 0.0f).xyz, 1.0f);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_local_dir = stage_input.v_local_dir;
    v_uv = stage_input.v_uv;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    return stage_output;
}
