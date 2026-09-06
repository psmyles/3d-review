cbuffer gtao_params : register(b0)
{
    row_major float4x4 _15_proj : packoffset(c0);
    float4 _15_params : packoffset(c4);
    float4 _15_config : packoffset(c5);
};

Texture2D<float4> gbuffer : register(t0);
SamplerState gtao_sampler : register(s0);
Texture2D<float4> raw_ao : register(t1);

static float2 v_uv;
static float frag_ao;

struct SPIRV_Cross_Input
{
    float2 v_uv : TEXCOORD0;
};

struct SPIRV_Cross_Output
{
    float frag_ao : SV_Target0;
};

float2 target_dims()
{
    return float2(max(_15_params.w, 1.0f), max(_15_config.w, 1.0f));
}

void frag_main()
{
    float4 _48 = gbuffer.SampleLevel(gtao_sampler, v_uv, 0.0f);
    float3 _53 = _48.xyz;
    float _57 = _48.w;
    if ((dot(_53, _53) < 0.25f) || (_57 >= (-9.9999997473787516355514526367188e-05f)))
    {
        frag_ao = 1.0f;
        return;
    }
    float2 _77 = 1.0f.xx / target_dims();
    float3 _80 = normalize(_53);
    float _88 = max(_15_params.x * 0.119999997317790985107421875f, 9.9999997473787516355514526367188e-05f);
    float sum = 0.0f;
    float weight_sum = 0.0f;
    for (int x = -2; x <= 2; x++)
    {
        for (int y = -2; y <= 2; y++)
        {
            float2 _116 = float2(float(x), float(y));
            float2 _122 = v_uv + (_116 * _77);
            float4 _128 = gbuffer.SampleLevel(gtao_sampler, _122, 0.0f);
            float3 _131 = _128.xyz;
            bool _136 = dot(_131, _131) < 0.25f;
            bool _143;
            if (!_136)
            {
                _143 = _128.w >= (-9.9999997473787516355514526367188e-05f);
            }
            else
            {
                _143 = _136;
            }
            if (_143)
            {
                continue;
            }
            float _152 = dot(_80, normalize(_131));
            if (_152 < 0.75f)
            {
                continue;
            }
            float _175 = abs(_128.w - _57);
            float _195 = (exp(dot(_116, _116) * (-0.125f)) * exp((-(_175 * _175)) / ((2.0f * _88) * _88))) * smoothstep(0.75f, 1.0f, _152);
            sum += (raw_ao.SampleLevel(gtao_sampler, _122, 0.0f).x * _195);
            weight_sum += _195;
        }
    }
    if (weight_sum <= 9.9999997473787516355514526367188e-06f)
    {
        frag_ao = raw_ao.SampleLevel(gtao_sampler, v_uv, 0.0f).x;
        return;
    }
    frag_ao = sum / weight_sum;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_uv = stage_input.v_uv;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_ao = frag_ao;
    return stage_output;
}
