cbuffer face_params_fs : register(b1)
{
    float4 face_forward : packoffset(c0);
    float4 face_right : packoffset(c1);
    float4 face_up : packoffset(c2);
    float4 face_params : packoffset(c3);
};

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

float radical_inverse_vdc(inout uint bits)
{
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 1431655765u) << 1u) | ((bits & 2863311530u) >> 1u);
    bits = ((bits & 858993459u) << 2u) | ((bits & 3435973836u) >> 2u);
    bits = ((bits & 252645135u) << 4u) | ((bits & 4042322160u) >> 4u);
    bits = ((bits & 16711935u) << 8u) | ((bits & 4278255360u) >> 8u);
    return float(bits) * 2.3283064365386962890625e-10f;
}

float2 hammersley(uint i, uint n)
{
    uint param = i;
    float _88 = radical_inverse_vdc(param);
    return float2(float(i) / float(n), _88);
}

float3 importance_sample_ggx(float2 xi, float3 n, float roughness)
{
    float _95 = roughness * roughness;
    float _101 = 6.283185482025146484375f * xi.x;
    float _116 = sqrt((1.0f - xi.y) / (1.0f + (((_95 * _95) - 1.0f) * xi.y)));
    float _122 = sqrt(1.0f - (_116 * _116));
    float3 up_axis = float3(0.0f, 0.0f, 1.0f);
    if (abs(n.z) > 0.999000012874603271484375f)
    {
        up_axis = float3(1.0f, 0.0f, 0.0f);
    }
    float3 _150 = normalize(cross(up_axis, n));
    return normalize(((_150 * (cos(_101) * _122)) + (cross(n, _150) * (sin(_101) * _122))) + (n * _116));
}

void frag_main()
{
    float3 _176 = normalize(v_local_dir);
    float3 prefiltered = 0.0f.xxx;
    float total_weight = 0.0f;
    for (uint i = 0u; i < 64u; i++)
    {
        uint param = i;
        uint param_1 = 64u;
        float2 param_2 = hammersley(param, param_1);
        float3 param_3 = _176;
        float param_4 = face_params.x;
        float3 _216 = importance_sample_ggx(param_2, param_3, param_4);
        float3 _227 = normalize((_216 * (2.0f * dot(_176, _216))) - _176);
        float _232 = max(dot(_176, _227), 0.0f);
        if (_232 > 0.0f)
        {
            prefiltered += (min(src_cube.SampleLevel(src_sampler, _227, 0.0f).xyz, 64.0f.xxx) * _232);
            total_weight += _232;
        }
    }
    if (total_weight > 0.0f)
    {
        prefiltered /= total_weight.xxx;
    }
    frag_color = float4(prefiltered, 1.0f);
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
