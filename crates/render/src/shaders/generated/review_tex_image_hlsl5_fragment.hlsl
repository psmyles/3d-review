cbuffer tex_params : register(b0)
{
    float2 _54_img_min : packoffset(c0);
    float2 _54_img_size : packoffset(c0.z);
    int _54_channel : packoffset(c1);
    int _54_target_srgb : packoffset(c1.y);
    float _54_checker_cell : packoffset(c1.z);
    int _54_pad : packoffset(c1.w);
    float4 _54_bg_light : packoffset(c2);
    float4 _54_bg_dark : packoffset(c3);
};

Texture2D<float4> tex : register(t0);
SamplerState samp : register(s0);

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

float3 srgb_to_linear(float3 c)
{
    return lerp(pow(max((c + 0.054999999701976776123046875f.xxx) * 0.947867333889007568359375f.xxx, 0.0f.xxx), 2.400000095367431640625f.xxx), c * 0.077399380505084991455078125f.xxx, step(c, 0.040449999272823333740234375f.xxx));
}

void frag_main()
{
    float2 _63 = (gl_FragCoord.xy - _54_img_min) / _54_img_size;
    float4 _77 = tex.Sample(samp, _63);
    float _83 = _63.x;
    bool _84 = _83 < 0.0f;
    bool _92;
    if (!_84)
    {
        _92 = _83 > 1.0f;
    }
    else
    {
        _92 = _84;
    }
    bool _100;
    if (!_92)
    {
        _100 = _63.y < 0.0f;
    }
    else
    {
        _100 = _92;
    }
    bool _107;
    if (!_100)
    {
        _107 = _63.y > 1.0f;
    }
    else
    {
        _107 = _100;
    }
    if (_107)
    {
        discard;
    }
    float alpha = 1.0f;
    float3 rgb;
    switch (_54_channel)
    {
        case 0:
        {
            rgb = _77.xyz;
            alpha = _77.w;
            break;
        }
        case 1:
        {
            rgb = _77.x.xxx;
            break;
        }
        case 2:
        {
            rgb = _77.y.xxx;
            break;
        }
        case 3:
        {
            rgb = _77.z.xxx;
            break;
        }
        default:
        {
            rgb = _77.w.xxx;
            break;
        }
    }
    float3 _152;
    if (_54_target_srgb == 1)
    {
        float3 param = rgb;
        _152 = srgb_to_linear(param);
    }
    else
    {
        _152 = rgb;
    }
    frag_color = float4(_152, alpha);
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
