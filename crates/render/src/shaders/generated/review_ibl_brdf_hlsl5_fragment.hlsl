static float2 v_uv;
static float2 frag_brdf;
static float3 v_local_dir;

struct SPIRV_Cross_Input
{
    float3 v_local_dir : TEXCOORD0;
    float2 v_uv : TEXCOORD1;
};

struct SPIRV_Cross_Output
{
    float2 frag_brdf : SV_Target0;
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
    float _99 = radical_inverse_vdc(param);
    return float2(float(i) / float(n), _99);
}

float3 importance_sample_ggx(float2 xi, float3 n, float roughness)
{
    float _106 = roughness * roughness;
    float _112 = 6.283185482025146484375f * xi.x;
    float _127 = sqrt((1.0f - xi.y) / (1.0f + (((_106 * _106) - 1.0f) * xi.y)));
    float _133 = sqrt(1.0f - (_127 * _127));
    float3 up_axis = float3(0.0f, 0.0f, 1.0f);
    if (abs(n.z) > 0.999000012874603271484375f)
    {
        up_axis = float3(1.0f, 0.0f, 0.0f);
    }
    float3 _161 = normalize(cross(up_axis, n));
    return normalize(((_161 * (cos(_112) * _133)) + (cross(n, _161) * (sin(_112) * _133))) + (n * _127));
}

float geometry_schlick_ggx(float n_dot_v, float roughness)
{
    float _188 = (roughness * roughness) * 0.5f;
    return n_dot_v / ((n_dot_v * (1.0f - _188)) + _188);
}

float geometry_smith(float n_dot_v, float n_dot_l, float roughness)
{
    float param = n_dot_v;
    float param_1 = roughness;
    float param_2 = n_dot_l;
    float param_3 = roughness;
    return geometry_schlick_ggx(param, param_1) * geometry_schlick_ggx(param_2, param_3);
}

void frag_main()
{
    float _219 = max(v_uv.x, 9.9999997473787516355514526367188e-05f);
    float3 _230 = float3(sqrt(1.0f - (_219 * _219)), 0.0f, _219);
    float a = 0.0f;
    float b = 0.0f;
    for (uint i = 0u; i < 512u; i++)
    {
        uint param = i;
        uint param_1 = 512u;
        float2 param_2 = hammersley(param, param_1);
        float3 param_3 = float3(0.0f, 0.0f, 1.0f);
        float param_4 = v_uv.y;
        float3 _258 = importance_sample_ggx(param_2, param_3, param_4);
        float _262 = dot(_230, _258);
        float _272 = max(normalize((_258 * (2.0f * _262)) - _230).z, 0.0f);
        float _281 = max(_262, 0.0f);
        if (_272 > 0.0f)
        {
            float param_5 = _219;
            float param_6 = _272;
            float param_7 = v_uv.y;
            float _301 = (geometry_smith(param_5, param_6, param_7) * _281) / (max(_258.z, 0.0f) * _219);
            float _307 = pow(max(1.0f - _281, 0.0f), 5.0f);
            a += ((1.0f - _307) * _301);
            b += (_307 * _301);
        }
    }
    frag_brdf = float2(a, b) * 0.001953125f.xx;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_uv = stage_input.v_uv;
    v_local_dir = stage_input.v_local_dir;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_brdf = frag_brdf;
    return stage_output;
}
