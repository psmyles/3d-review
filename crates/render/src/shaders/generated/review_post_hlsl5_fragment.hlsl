cbuffer post_params : register(b0)
{
    int _416_gtao_enabled : packoffset(c0);
    int _416_tonemap_enabled : packoffset(c0.y);
    int _416_tonemap_op : packoffset(c0.z);
    int _416_passthrough : packoffset(c0.w);
    float4 _416_bg_top : packoffset(c1);
    float4 _416_bg_bottom : packoffset(c2);
};

Texture2D<float4> scene_color : register(t0);
SamplerState scene_sampler : register(s0);
Texture2D<float4> gtao_texture : register(t1);
Texture2D<float4> ambient_texture : register(t2);

static float2 v_uv;
static float4 frag_color;

struct SPIRV_Cross_Input
{
    float2 v_uv : TEXCOORD0;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
};

float4 over_background(float3 surface, float coverage, float3 bg, float bg_alpha)
{
    float _388 = coverage + (bg_alpha * (1.0f - coverage));
    return float4(((surface * coverage) + ((bg * bg_alpha) * (1.0f - coverage))) / max(_388, 9.9999997473787516355514526367188e-05f).xxx, _388);
}

float3 pbr_neutral_tonemap(float3 color_in)
{
    float3 color = color_in;
    float _85 = min(color_in.x, min(color_in.y, color_in.z));
    float _91;
    if (_85 < 0.07999999821186065673828125f)
    {
        _91 = _85 - ((6.25f * _85) * _85);
    }
    else
    {
        _91 = 0.039999999105930328369140625f;
    }
    float3 _104 = color;
    float3 _107 = _104 - _91.xxx;
    color = _107;
    float _116 = max(_107.x, max(_107.y, _107.z));
    if (_116 < 0.7599999904632568359375f)
    {
        return color;
    }
    float _136 = 1.0f - (0.057599999010562896728515625f / (_116 + (-0.519999980926513671875f)));
    float3 _137 = color;
    float3 _141 = _137 * (_136 / _116);
    color = _141;
    return lerp(_141, 1.0f.xxx * _136, (1.0f - (1.0f / ((0.1500000059604644775390625f * (_116 - _136)) + 1.0f))).xxx);
}

float3 reinhard_tonemap(float3 color)
{
    return color / (1.0f.xxx + color);
}

float3 aces_rrt_odt_fit(float3 v)
{
    return ((v * (v + 0.02457859925925731658935546875f.xxx)) - 9.0537003416102379560470581054688e-05f.xxx) / ((v * ((v * 0.98372900485992431640625f) + 0.4329510033130645751953125f.xxx)) + 0.23808099329471588134765625f.xxx);
}

float3 aces_tonemap(float3 color_in)
{
    float3 param = mul(color_in * 1.66666662693023681640625f.xxx, float3x3(float3(0.59719002246856689453125f, 0.075999997556209564208984375f, 0.0284000001847743988037109375f), float3(0.354579985141754150390625f, 0.908339977264404296875f, 0.13382999598979949951171875f), float3(0.048229999840259552001953125f, 0.0156599991023540496826171875f, 0.837769985198974609375f)));
    return clamp(mul(aces_rrt_odt_fit(param), float3x3(float3(1.60475003719329833984375f, -0.10208000242710113525390625f, -0.00326999998651444911956787109375f), float3(-0.5310800075531005859375f, 1.108129978179931640625f, -0.07276000082492828369140625f), float3(-0.0736699998378753662109375f, -0.00604999996721744537353515625f, 1.0760200023651123046875f))), 0.0f.xxx, 1.0f.xxx);
}

float3 agx_contrast_approx(float3 x)
{
    float3 _246 = x * x;
    float3 _250 = _246 * _246;
    return (((((((_250 * 15.5f) * _246) - ((_250 * 40.1399993896484375f) * x)) + (_250 * 31.95999908447265625f)) - ((_246 * 6.868000030517578125f) * x)) + (_246 * 0.4298000037670135498046875f)) + (x * 0.119099996984004974365234375f)) - 0.002319999970495700836181640625f.xxx;
}

float3 agx_tonemap(float3 color_in)
{
    float3 param = clamp((log2(max(mul(mul(color_in, float3x3(float3(0.627399981021881103515625f, 0.069099999964237213134765625f, 0.01640000008046627044677734375f), float3(0.329299986362457275390625f, 0.91949999332427978515625f, 0.087999999523162841796875f), float3(0.0432999990880489349365234375f, 0.011300000362098217010498046875f, 0.895600020885467529296875f))), float3x3(float3(0.856627166271209716796875f, 0.13731896877288818359375f, 0.1118982136249542236328125f), float3(0.095121242105960845947265625f, 0.761241972446441650390625f, 0.076799415051937103271484375f), float3(0.048251606523990631103515625f, 0.101439036428928375244140625f, 0.811302363872528076171875f))), 1.0000000133514319600180897396058e-10f.xxx)) - (-12.47393035888671875f).xxx) * 0.0606060661375522613525390625f.xxx, 0.0f.xxx, 1.0f.xxx);
    return clamp(mul(pow(max(mul(agx_contrast_approx(param), float3x3(float3(1.12710058689117431640625f, -0.14132976531982421875f, -0.14132976531982421875f), float3(-0.1106066405773162841796875f, 1.15782368183135986328125f, -0.1106066405773162841796875f), float3(-0.016493938863277435302734375f, -0.016493938863277435302734375f, 1.251936435699462890625f))), 0.0f.xxx), 2.2000000476837158203125f.xxx), float3x3(float3(1.660500049591064453125f, -0.124600000679492950439453125f, -0.01820000074803829193115234375f), float3(-0.5875999927520751953125f, 1.1328999996185302734375f, -0.100599996745586395263671875f), float3(-0.072800002992153167724609375f, -0.008299999870359897613525390625f, 1.1187000274658203125f))), 0.0f.xxx, 1.0f.xxx);
}

float3 apply_tonemap(float3 color)
{
    if (_416_tonemap_enabled == 0)
    {
        return color;
    }
    switch (_416_tonemap_op)
    {
        case 0:
        {
            float3 param = color;
            return pbr_neutral_tonemap(param);
        }
        case 1:
        {
            return color;
        }
        case 2:
        {
            float3 param_1 = color;
            return reinhard_tonemap(param_1);
        }
        case 3:
        {
            float3 param_2 = color;
            return aces_tonemap(param_2);
        }
        case 4:
        {
            float3 param_3 = color;
            return agx_tonemap(param_3);
        }
        default:
        {
            float3 param_4 = color;
            return pbr_neutral_tonemap(param_4);
        }
    }
}

float3 linear_to_srgb(float3 c)
{
    return lerp((pow(max(c, 0.0f.xxx), 0.4166666567325592041015625f.xxx) * 1.05499994754791259765625f) - 0.054999999701976776123046875f.xxx, c * 12.9200000762939453125f, step(c, 0.003130800090730190277099609375f.xxx));
}

void frag_main()
{
    float4 _477 = scene_color.Sample(scene_sampler, v_uv);
    float3 _480 = _477.xyz;
    float3 lit = _480;
    float _484 = _477.w;
    float3 _489 = max(_484, 9.9999997473787516355514526367188e-05f).xxx;
    float3 surface = _480 / _489;
    float3 _506 = lerp(_416_bg_top.xyz, _416_bg_bottom.xyz, clamp(v_uv.y, 0.0f, 1.0f).xxx);
    if (_416_passthrough != 0)
    {
        float3 param = surface;
        float param_1 = _484;
        float3 param_2 = _506;
        float param_3 = _416_bg_top.w;
        frag_color = over_background(param, param_1, param_2, param_3);
        return;
    }
    if (_416_gtao_enabled != 0)
    {
        float3 _548 = lit;
        float3 _554 = max(_548 - (ambient_texture.Sample(scene_sampler, v_uv).xyz * (1.0f - gtao_texture.Sample(scene_sampler, v_uv).x)), 0.0f.xxx);
        lit = _554;
        surface = _554 / _489;
    }
    float3 param_4 = surface;
    float3 param_5 = apply_tonemap(param_4);
    float3 param_6 = linear_to_srgb(param_5);
    float param_7 = _484;
    float3 param_8 = _506;
    float param_9 = _416_bg_top.w;
    frag_color = over_background(param_6, param_7, param_8, param_9);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_uv = stage_input.v_uv;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_color = frag_color;
    return stage_output;
}
