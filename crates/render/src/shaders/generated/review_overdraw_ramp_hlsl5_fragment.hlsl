cbuffer overdraw_params : register(b0)
{
    float4 _17_rect : packoffset(c0);
    float4 _17_range : packoffset(c1);
    float4 _17_stop0 : packoffset(c2);
    float4 _17_stop1 : packoffset(c3);
    float4 _17_stop2 : packoffset(c4);
    float4 _17_stop3 : packoffset(c5);
    float4 _17_empty : packoffset(c6);
};

Texture2D<float4> count_tex : register(t0);
SamplerState count_smp : register(s0);

static float4 gl_FragCoord;
static float4 frag_color;

struct SPIRV_Cross_Input
{
    float4 gl_FragCoord : SV_Position;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
};

void frag_main()
{
    float4 _46 = count_tex.SampleLevel(count_smp, (gl_FragCoord.xy - _17_rect.xy) / max(_17_rect.zw, 1.0f.xx), 0.0f);
    float _49 = _46.x;
    if (_49 <= 0.0f)
    {
        frag_color = float4(_17_empty.xyz, 1.0f);
        return;
    }
    float _85 = clamp((_49 - _17_range.y) / max(_17_range.x - _17_range.y, 0.001000000047497451305389404296875f), 0.0f, 1.0f) * 3.0f;
    float3 _90;
    if (_85 < 1.0f)
    {
        _90 = lerp(_17_stop0.xyz, _17_stop1.xyz, _85.xxx);
    }
    else
    {
        float3 _108;
        if (_85 < 2.0f)
        {
            _108 = lerp(_17_stop1.xyz, _17_stop2.xyz, (_85 - 1.0f).xxx);
        }
        else
        {
            _108 = lerp(_17_stop2.xyz, _17_stop3.xyz, (_85 - 2.0f).xxx);
        }
        _90 = _108;
    }
    frag_color = float4(_90, 1.0f);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_FragCoord = stage_input.gl_FragCoord;
    gl_FragCoord.w = 1.0 / gl_FragCoord.w;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    return stage_output;
}
