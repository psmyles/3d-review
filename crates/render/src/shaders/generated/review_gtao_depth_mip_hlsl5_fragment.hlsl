cbuffer gtao_mip_params : register(b0)
{
    float4 _29_mip : packoffset(c0);
};

Texture2D<float4> depth_src : register(t0);
SamplerState gtao_sampler : register(s0);

static float4 gl_FragCoord;
static float frag_depth;

struct SPIRV_Cross_Input
{
    float4 gl_FragCoord : SV_Position;
};

struct SPIRV_Cross_Output
{
    float frag_depth : SV_Target0;
};

bool is_background_depth(float depth)
{
    return !((depth > 0.0f) && (depth < 1000000015047466219876688855040.0f));
}

void frag_main()
{
    float2 _38 = max(_29_mip.yz, 1.0f.xx);
    float2 _46 = floor(gl_FragCoord.xy) * 2.0f;
    float depths[4];
    for (int i = 0; i < 4; i++)
    {
        float4 _88 = depth_src.SampleLevel(gtao_sampler, (min(_46 + float2(float(i & 1), float(i >> 1)), _38 - 1.0f.xx) + 0.5f.xx) / _38, 0.0f);
        float _91 = _88.x;
        float param = _91;
        depths[i] = is_background_depth(param) ? 1000000015047466219876688855040.0f : _91;
    }
    float _118 = max(max(depths[0], depths[1]), max(depths[2], depths[3]));
    if (_118 >= 1000000015047466219876688855040.0f)
    {
        frag_depth = 1000000015047466219876688855040.0f;
        return;
    }
    float _132 = max(_29_mip.w, 9.9999997473787516355514526367188e-05f);
    float _148 = (-1.48800384998321533203125f) / _132;
    float _153 = ((_132 * 0.42070877552032470703125f) / (_132 * 0.67204129695892333984375f)) + 1.0f;
    float sum = 0.0f;
    float weight_sum = 0.0f;
    for (int i_1 = 0; i_1 < 4; i_1++)
    {
        float _169 = min(depths[i_1], _118);
        float _178 = clamp(((_118 - _169) * _148) + _153, 0.0f, 1.0f);
        sum += (_169 * _178);
        weight_sum += _178;
    }
    frag_depth = sum / max(weight_sum, 9.9999999747524270787835121154785e-07f);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_FragCoord = stage_input.gl_FragCoord;
    gl_FragCoord.w = 1.0 / gl_FragCoord.w;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_depth = frag_depth;
    return stage_output;
}
