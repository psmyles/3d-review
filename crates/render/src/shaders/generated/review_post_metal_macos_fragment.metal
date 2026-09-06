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
float3 pbr_neutral_tonemap(thread const float3& color_in)
{
    float3 color = color_in;
    float _77 = fast::min(color_in.x, fast::min(color_in.y, color_in.z));
    float _83;
    if (_77 < 0.07999999821186065673828125)
    {
        _83 = _77 - ((6.25 * _77) * _77);
    }
    else
    {
        _83 = 0.039999999105930328369140625;
    }
    float3 _96 = color;
    float3 _99 = _96 - float3(_83);
    color = _99;
    float _108 = fast::max(_99.x, fast::max(_99.y, _99.z));
    if (_108 < 0.7599999904632568359375)
    {
        return color;
    }
    float _128 = 1.0 - (0.057599999010562896728515625 / (_108 + (-0.519999980926513671875)));
    float3 _129 = color;
    float3 _133 = _129 * (_128 / _108);
    color = _133;
    return mix(_133, float3(1.0) * _128, float3(1.0 - (1.0 / ((0.1500000059604644775390625 * (_108 - _128)) + 1.0))));
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
    float3 _238 = x * x;
    float3 _242 = _238 * _238;
    return (((((((_242 * 15.5) * _238) - ((_242 * 40.1399993896484375) * x)) + (_242 * 31.95999908447265625)) - ((_238 * 6.868000030517578125) * x)) + (_238 * 0.4298000037670135498046875)) + (x * 0.119099996984004974365234375)) - float3(0.002319999970495700836181640625);
}

static inline __attribute__((always_inline))
float3 agx_tonemap(thread const float3& color_in)
{
    float3 param = fast::clamp((log2(fast::max(float3x3(float3(0.856627166271209716796875, 0.13731896877288818359375, 0.1118982136249542236328125), float3(0.095121242105960845947265625, 0.761241972446441650390625, 0.076799415051937103271484375), float3(0.048251606523990631103515625, 0.101439036428928375244140625, 0.811302363872528076171875)) * (float3x3(float3(0.627399981021881103515625, 0.069099999964237213134765625, 0.01640000008046627044677734375), float3(0.329299986362457275390625, 0.91949999332427978515625, 0.087999999523162841796875), float3(0.0432999990880489349365234375, 0.011300000362098217010498046875, 0.895600020885467529296875)) * color_in), float3(1.0000000133514319600180897396058e-10))) - float3(-12.47393035888671875)) * float3(0.0606060661375522613525390625), float3(0.0), float3(1.0));
    return fast::clamp(float3x3(float3(1.660500049591064453125, -0.124600000679492950439453125, -0.01820000074803829193115234375), float3(-0.5875999927520751953125, 1.1328999996185302734375, -0.100599996745586395263671875), float3(-0.072800002992153167724609375, -0.008299999870359897613525390625, 1.1187000274658203125)) * powr(fast::max(float3x3(float3(1.12710058689117431640625, -0.14132976531982421875, -0.14132976531982421875), float3(-0.1106066405773162841796875, 1.15782368183135986328125, -0.1106066405773162841796875), float3(-0.016493938863277435302734375, -0.016493938863277435302734375, 1.251936435699462890625)) * agx_contrast_approx(param), float3(0.0)), float3(2.2000000476837158203125)), float3(0.0), float3(1.0));
}

static inline __attribute__((always_inline))
float3 apply_tonemap(thread const float3& color, constant post_params& _378)
{
    if (_378.tonemap_enabled == 0)
    {
        return color;
    }
    switch (_378.tonemap_op)
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

fragment main0_out main0(main0_in in [[stage_in]], constant post_params& _378 [[buffer(0)]], texture2d<float> scene_color [[texture(0)]], texture2d<float> gtao_texture [[texture(1)]], texture2d<float> ambient_texture [[texture(2)]], sampler scene_sampler [[sampler(0)]])
{
    main0_out out = {};
    float4 _439 = scene_color.sample(scene_sampler, in.v_uv);
    float3 _442 = _439.xyz;
    float3 lit = _442;
    float _446 = _439.w;
    float3 _452 = float3(fast::max(_446, 9.9999997473787516355514526367188e-05));
    float3 surface = _442 / _452;
    float3 _469 = mix(_378.bg_top.xyz, _378.bg_bottom.xyz, float3(fast::clamp(in.v_uv.y, 0.0, 1.0)));
    if (_378.passthrough != 0)
    {
        out.frag_color = float4(mix(_469, surface, float3(_446)), 1.0);
        return out;
    }
    if (_378.gtao_enabled != 0)
    {
        float3 _509 = lit;
        float3 _515 = fast::max(_509 - (ambient_texture.sample(scene_sampler, in.v_uv).xyz * (1.0 - gtao_texture.sample(scene_sampler, in.v_uv).x)), float3(0.0));
        lit = _515;
        surface = _515 / _452;
    }
    float3 param = surface;
    float3 param_1 = apply_tonemap(param, _378);
    out.frag_color = float4(mix(_469, linear_to_srgb(param_1), float3(_446)), 1.0);
    return out;
}

