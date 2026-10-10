cbuffer quad_params : register(b4)
{
    float4 qu_params : packoffset(c0);
};


static float4 gl_FragCoord;
static float3 v_corner_b;
static float3 v_corner_c;
static float3 v_corner_a;
static float4 frag_count;

struct SPIRV_Cross_Input
{
    nointerpolation float3 v_corner_a : TEXCOORD0;
    nointerpolation float3 v_corner_b : TEXCOORD1;
    nointerpolation float3 v_corner_c : TEXCOORD2;
    float4 gl_FragCoord : SV_Position;
};

struct SPIRV_Cross_Output
{
    float4 frag_count : SV_Target0;
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
    float3 _15 = cross(v_corner_b, v_corner_c);
    float3 _20 = cross(v_corner_c, v_corner_a);
    float3 _24 = cross(v_corner_a, v_corner_b);
    float _29 = dot(v_corner_a, _15);
    float2 _45 = max(qu_params.xy, 1.0f.xx);
    float2 _51 = floor(gl_FragCoord.xy);
    float2 _58 = _51 - mod(_51, 2.0f.xx);
    float covered = 0.0f;
    for (int i = 0; i < 4; i++)
    {
        float2 _85 = (_58 + float2(float(i & 1), float(i >> 1))) + 0.5f.xx;
        float3 _104 = float3(((_85.x / _45.x) * 2.0f) - 1.0f, 1.0f - ((_85.y / _45.y) * 2.0f), 1.0f);
        float3 _118 = float3(dot(_15, _104), dot(_20, _104), dot(_24, _104)) * sign(_29);
        bool _121 = _118.x >= 0.0f;
        bool _127;
        if (_121)
        {
            _127 = _118.y >= 0.0f;
        }
        else
        {
            _127 = _121;
        }
        bool _134;
        if (_127)
        {
            _134 = _118.z >= 0.0f;
        }
        else
        {
            _134 = _127;
        }
        covered += float(_134);
    }
    float _145;
    if (abs(_29) > 9.9999999600419720025001879548654e-13f)
    {
        _145 = max(covered, 1.0f);
    }
    else
    {
        _145 = 4.0f;
    }
    frag_count = float4(4.0f / _145, 0.0f, 0.0f, 1.0f);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_FragCoord = stage_input.gl_FragCoord;
    gl_FragCoord.w = 1.0 / gl_FragCoord.w;
    v_corner_b = stage_input.v_corner_b;
    v_corner_c = stage_input.v_corner_c;
    v_corner_a = stage_input.v_corner_a;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_count = frag_count;
    return stage_output;
}
