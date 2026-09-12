cbuffer gtao_params : register(b0)
{
    row_major float4x4 _27_proj : packoffset(c0);
    float4 _27_params : packoffset(c4);
    float4 _27_config : packoffset(c5);
    float4 _27_temporal : packoffset(c6);
};

Texture2D<float4> gbuffer : register(t0);
SamplerState gtao_sampler : register(s0);
Texture2D<float4> ao_in : register(t1);

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
    return float2(max(_27_params.w, 1.0f), max(_27_config.w, 1.0f));
}

float3 reconstruct_view_pos(float2 uv, float view_z)
{
    float _50 = (uv.x * 2.0f) - 1.0f;
    float _55 = 1.0f - (uv.y * 2.0f);
    if (_27_config.x > 0.5f)
    {
        return float3((_50 - _27_proj[3].x) / _27_proj[0].x, (_55 - _27_proj[3].y) / _27_proj[1].y, view_z);
    }
    return float3((_50 * (-view_z)) / _27_proj[0].x, (_55 * (-view_z)) / _27_proj[1].y, view_z);
}

float view_pixel_size(float view_z)
{
    float _121;
    if (_27_config.x > 0.5f)
    {
        _121 = 2.0f;
    }
    else
    {
        _121 = 2.0f * abs(view_z);
    }
    return _121 / (max(abs(_27_proj[1].y), 9.9999999747524270787835121154785e-07f) * max(_27_config.w, 1.0f));
}

void frag_main()
{
    float4 _156 = gbuffer.SampleLevel(gtao_sampler, v_uv, 0.0f);
    float3 _160 = _156.xyz;
    float _163 = _156.w;
    if ((dot(_160, _160) < 0.25f) || (_163 >= (-9.9999997473787516355514526367188e-05f)))
    {
        frag_ao = 1.0f;
        return;
    }
    float2 _181 = 1.0f.xx / target_dims();
    float3 _184 = normalize(_160);
    float2 param = v_uv;
    float param_1 = _163;
    float3 _190 = reconstruct_view_pos(param, param_1);
    float param_2 = _163;
    float _196 = max(2.0f * view_pixel_size(param_2), 9.9999999747524270787835121154785e-07f);
    float sum = 0.0f;
    float weight_sum = 0.0f;
    for (int x = -2; x <= 2; x++)
    {
        for (int y = -2; y <= 2; y++)
        {
            float2 _222 = float2(float(x), float(y));
            float2 _228 = v_uv + (_222 * _181);
            float4 _234 = gbuffer.SampleLevel(gtao_sampler, _228, 0.0f);
            float3 _236 = _234.xyz;
            bool _240 = dot(_236, _236) < 0.25f;
            bool _247;
            if (!_240)
            {
                _247 = _234.w >= (-9.9999997473787516355514526367188e-05f);
            }
            else
            {
                _247 = _240;
            }
            if (_247)
            {
                continue;
            }
            float2 param_3 = _228;
            float param_4 = _234.w;
            float _261 = dot(_184, reconstruct_view_pos(param_3, param_4) - _190);
            float _295 = (exp(dot(_222, _222) * (-0.22222222387790679931640625f)) * exp((-(_261 * _261)) / ((2.0f * _196) * _196))) * pow(max(dot(_184, normalize(_236)), 0.0f), 8.0f);
            sum += (ao_in.SampleLevel(gtao_sampler, _228, 0.0f).x * _295);
            weight_sum += _295;
        }
    }
    if (weight_sum <= 9.9999997473787516355514526367188e-06f)
    {
        frag_ao = ao_in.SampleLevel(gtao_sampler, v_uv, 0.0f).x;
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
