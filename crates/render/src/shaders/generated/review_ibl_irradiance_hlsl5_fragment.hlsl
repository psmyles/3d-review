TextureCube<float4> src_cube : register(t0);
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

void frag_main()
{
    float3 _13 = normalize(v_local_dir);
    float3 up_axis = float3(0.0f, 1.0f, 0.0f);
    if (abs(_13.y) > 0.999000012874603271484375f)
    {
        up_axis = float3(0.0f, 0.0f, 1.0f);
    }
    float3 _34 = normalize(cross(up_axis, _13));
    float3 _38 = cross(_13, _34);
    float3 irradiance = 0.0f.xxx;
    float samples = 0.0f;
    for (float phi = 0.0f; phi < 6.283185482025146484375f; phi += 0.0500000007450580596923828125f)
    {
        for (float theta = 0.0f; theta < 1.57079637050628662109375f; theta += 0.0500000007450580596923828125f)
        {
            float _62 = sin(theta);
            float _72 = cos(theta);
            irradiance += ((min(src_cube.SampleLevel(src_sampler, ((_34 * (_62 * cos(phi))) + (_38 * (_62 * sin(phi)))) + (_13 * _72), 0.0f).xyz, 64.0f.xxx) * _72) * _62);
            samples += 1.0f;
        }
    }
    float3 _126 = irradiance;
    float3 _131 = (_126 * 3.1415927410125732421875f) / max(samples, 1.0f).xxx;
    irradiance = _131;
    frag_color = float4(_131, 1.0f);
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
