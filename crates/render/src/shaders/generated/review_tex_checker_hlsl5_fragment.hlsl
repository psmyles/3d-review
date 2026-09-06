cbuffer tex_params : register(b0)
{
    float2 _14_img_min : packoffset(c0);
    float2 _14_img_size : packoffset(c0.z);
    int _14_channel : packoffset(c1);
    int _14_target_srgb : packoffset(c1.y);
    float _14_checker_cell : packoffset(c1.z);
    int _14_pad : packoffset(c1.w);
    float4 _14_bg_light : packoffset(c2);
    float4 _14_bg_dark : packoffset(c3);
};


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

float mod(float x, float y)
{
    return x - y * floor(x / y);
}

float2 mod(float2 x, float2 y)
{
    return x - y * floor(x / y);
}

float3 mod(float3 x, float3 y)
{
    return x - y * floor(x / y);
}

float4 mod(float4 x, float4 y)
{
    return x - y * floor(x / y);
}

void frag_main()
{
    float2 _30 = floor(gl_FragCoord.xy / max(_14_checker_cell, 1.0f).xx);
    float4 _49;
    if (mod(_30.x + _30.y, 2.0f) < 0.5f)
    {
        _49 = _14_bg_light;
    }
    else
    {
        _49 = _14_bg_dark;
    }
    frag_color = _49;
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
