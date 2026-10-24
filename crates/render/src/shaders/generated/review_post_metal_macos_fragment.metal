#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct post_params
{
    int gtao_enabled;
    int tonemap_enabled;
    int tonemap_op;
    int passthrough;
    float4 bg_top;
    float4 bg_bottom;
};

struct main0_out
{
    float4 frag_color [[color(0)]];
};

struct main0_in
{
    float2 v_uv [[user(locn0)]];
};

static inline __attribute__((always_inline))
float4 over_background(thread const float3& surface, thread const float& coverage, thread const float3& bg, thread const float& bg_alpha)
{
    float _388 = coverage + (bg_alpha * (1.0 - coverage));
    return float4(((surface * coverage) + ((bg * bg_alpha) * (1.0 - coverage))) / float3(fast::max(_388, 9.9999997473787516355514526367188e-05)), _388);
}

static inline __attribute__((always_inline))
float3 pbr_neutral_tonemap(thread const float3& color_in)
{
    float3 color = color_in;
    float _85 = fast::min(color_in.x, fast::min(color_in.y, color_in.z));
    float _91;
    if (_85 < 0.07999999821186065673828125)
    {
        _91 = _85 - ((6.25 * _85) * _85);
    }
    else
    {
        _91 = 0.039999999105930328369140625;
    }
    float3 _104 = color;
    float3 _107 = _104 - float3(_91);
    color = _107;
    float _116 = fast::max(_107.x, fast::max(_107.y, _107.z));
    if (_116 < 0.7599999904632568359375)
    {
        return color;
    }
    float _136 = 1.0 - (0.057599999010562896728515625 / (_116 + (-0.519999980926513671875)));
    float3 _137 = color;
    float3 _141 = _137 * (_136 / _116);
    color = _141;
    return mix(_141, float3(1.0) * _136, float3(1.0 - (1.0 / ((0.1500000059604644775390625 * (_116 - _136)) + 1.0))));
}

static inline __attribute__((always_inline))
float3 reinhard_tonemap(thread const float3& color)
{
    return color / (float3(1.0) + color);
}

static inline __attribute__((always_inline))
float3 aces_rrt_odt_fit(thread const float3& v)
{
    return ((v * (v + float3(0.02457859925925731658935546875))) - float3(9.0537003416102379560470581054688e-05)) / ((v * ((v * 0.98372900485992431640625) + float3(0.4329510033130645751953125))) + float3(0.23808099329471588134765625));
}

static inline __attribute__((always_inline))
float3 aces_tonemap(thread const float3& color_in)
{
    float3 param = float3x3(float3(0.59719002246856689453125, 0.075999997556209564208984375, 0.0284000001847743988037109375), float3(0.354579985141754150390625, 0.908339977264404296875, 0.13382999598979949951171875), float3(0.048229999840259552001953125, 0.0156599991023540496826171875, 0.837769985198974609375)) * (color_in * float3(1.66666662693023681640625));
    return fast::clamp(float3x3(float3(1.60475003719329833984375, -0.10208000242710113525390625, -0.00326999998651444911956787109375), float3(-0.5310800075531005859375, 1.108129978179931640625, -0.07276000082492828369140625), float3(-0.0736699998378753662109375, -0.00604999996721744537353515625, 1.0760200023651123046875)) * aces_rrt_odt_fit(param), float3(0.0), float3(1.0));
}

static inline __attribute__((always_inline))
float3 agx_contrast_approx(thread const float3& x)
{
    float3 _246 = x * x;
    float3 _250 = _246 * _246;
    return (((((((_250 * 15.5) * _246) - ((_250 * 40.1399993896484375) * x)) + (_250 * 31.95999908447265625)) - ((_246 * 6.868000030517578125) * x)) + (_246 * 0.4298000037670135498046875)) + (x * 0.119099996984004974365234375)) - float3(0.002319999970495700836181640625);
}

static inline __attribute__((always_inline))
float3 agx_tonemap(thread const float3& color_in)
{
    float3 param = fast::clamp((log2(fast::max(float3x3(float3(0.856627166271209716796875, 0.13731896877288818359375, 0.1118982136249542236328125), float3(0.095121242105960845947265625, 0.761241972446441650390625, 0.076799415051937103271484375), float3(0.048251606523990631103515625, 0.101439036428928375244140625, 0.811302363872528076171875)) * (float3x3(float3(0.627399981021881103515625, 0.069099999964237213134765625, 0.01640000008046627044677734375), float3(0.329299986362457275390625, 0.91949999332427978515625, 0.087999999523162841796875), float3(0.0432999990880489349365234375, 0.011300000362098217010498046875, 0.895600020885467529296875)) * color_in), float3(1.0000000133514319600180897396058e-10))) - float3(-12.47393035888671875)) * float3(0.0606060661375522613525390625), float3(0.0), float3(1.0));
    return fast::clamp(float3x3(float3(1.660500049591064453125, -0.124600000679492950439453125, -0.01820000074803829193115234375), float3(-0.5875999927520751953125, 1.1328999996185302734375, -0.100599996745586395263671875), float3(-0.072800002992153167724609375, -0.008299999870359897613525390625, 1.1187000274658203125)) * powr(fast::max(float3x3(float3(1.12710058689117431640625, -0.14132976531982421875, -0.14132976531982421875), float3(-0.1106066405773162841796875, 1.15782368183135986328125, -0.1106066405773162841796875), float3(-0.016493938863277435302734375, -0.016493938863277435302734375, 1.251936435699462890625)) * agx_contrast_approx(param), float3(0.0)), float3(2.2000000476837158203125)), float3(0.0), float3(1.0));
}

static inline __attribute__((always_inline))
float3 apply_tonemap(thread const float3& color, constant post_params& _416)
{
    if (_416.tonemap_enabled == 0)
    {
        return color;
    }
    switch (_416.tonemap_op)
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

static inline __attribute__((always_inline))
float3 linear_to_srgb(thread const float3& c)
{
    return mix((powr(fast::max(c, float3(0.0)), float3(0.4166666567325592041015625)) * 1.05499994754791259765625) - float3(0.054999999701976776123046875), c * 12.9200000762939453125, step(c, float3(0.003130800090730190277099609375)));
}

fragment main0_out main0(main0_in in [[stage_in]], constant post_params& _416 [[buffer(0)]], texture2d<float> scene_color [[texture(0)]], texture2d<float> gtao_texture [[texture(1)]], texture2d<float> ambient_texture [[texture(2)]], sampler scene_sampler [[sampler(0)]])
{
    main0_out out = {};
    float4 _477 = scene_color.sample(scene_sampler, in.v_uv);
    float3 _480 = _477.xyz;
    float3 lit = _480;
    float _484 = _477.w;
    float3 _489 = float3(fast::max(_484, 9.9999997473787516355514526367188e-05));
    float3 surface = _480 / _489;
    float3 _506 = mix(_416.bg_top.xyz, _416.bg_bottom.xyz, float3(fast::clamp(in.v_uv.y, 0.0, 1.0)));
    if (_416.passthrough != 0)
    {
        float3 param = surface;
        float param_1 = _484;
        float3 param_2 = _506;
        float param_3 = _416.bg_top.w;
        out.frag_color = over_background(param, param_1, param_2, param_3);
        return out;
    }
    if (_416.gtao_enabled != 0)
    {
        float3 _548 = lit;
        float3 _554 = fast::max(_548 - (ambient_texture.sample(scene_sampler, in.v_uv).xyz * (1.0 - gtao_texture.sample(scene_sampler, in.v_uv).x)), float3(0.0));
        lit = _554;
        surface = _554 / _489;
    }
    float3 param_4 = surface;
    float3 param_5 = apply_tonemap(param_4, _416);
    float3 param_6 = linear_to_srgb(param_5);
    float param_7 = _484;
    float3 param_8 = _506;
    float param_9 = _416.bg_top.w;
    out.frag_color = over_background(param_6, param_7, param_8, param_9);
    return out;
}

