#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct ShadingResult
{
    float3 color;
    float3 ambient;
};

struct scene_fs
{
    float4x4 view_projection;
    float4x4 inv_view_projection;
    float4 render_options;
    float4 camera_position;
    float4 env_params;
    float4 projection_params;
    float4x4 view;
    float4 selection_color;
};

struct material
{
    float4 mat_base_color;
    float4 mat_emissive;
    float4 mat_params;
    float4 mat_channels0;
    float4 mat_channels1;
    float4 mat_flags;
};

struct main0_out
{
    float4 frag_color [[color(0)]];
    float4 frag_ambient [[color(1)]];
};

struct main0_in
{
    float3 v_normal [[user(locn0)]];
    float2 v_uv [[user(locn1)]];
    float4 v_color [[user(locn2)]];
    float3 v_world_position [[user(locn3)]];
    float4 v_tangent [[user(locn4)]];
};

static inline __attribute__((always_inline))
float3 srgb_to_linear(thread const float3& c)
{
    return mix(powr(fast::max((c + float3(0.054999999701976776123046875)) * float3(0.947867333889007568359375), float3(0.0)), float3(2.400000095367431640625)), c * float3(0.077399380505084991455078125), step(c, float3(0.040449999272823333740234375)));
}

static inline __attribute__((always_inline))
float3 linear_to_srgb(thread const float3& c)
{
    return mix((powr(fast::max(c, float3(0.0)), float3(0.4166666567325592041015625)) * 1.05499994754791259765625) - float3(0.054999999701976776123046875), c * 12.9200000762939453125, step(c, float3(0.003130800090730190277099609375)));
}

static inline __attribute__((always_inline))
float select_channel(thread const float4& texel, thread const float& index)
{
    if (index < 0.5)
    {
        return texel.x;
    }
    if (index < 1.5)
    {
        return texel.y;
    }
    if (index < 2.5)
    {
        return texel.z;
    }
    return texel.w;
}

static inline __attribute__((always_inline))
float3 apply_normal_map(thread const float3& n, thread const float4& tangent, thread const float3& sample_rgb)
{
    float3 _137 = fast::normalize(n);
    float3 _148 = fast::normalize(tangent.xyz - (_137 * dot(_137, tangent.xyz)));
    if (dot(_148, _148) < 9.9999999392252902907785028219223e-09)
    {
        return _137;
    }
    float3 _171 = (sample_rgb * 2.0) - float3(1.0);
    return fast::normalize(((_148 * _171.x) + ((cross(_137, _148) * tangent.w) * _171.y)) + (_137 * _171.z));
}

static inline __attribute__((always_inline))
float3 env_sample_dir(thread const float3& dir, constant scene_fs& sc)
{
    float _199 = -sc.projection_params.y;
    float _202 = sin(_199);
    float _205 = cos(_199);
    return float3((_205 * dir.x) + (_202 * dir.z), dir.y, ((-_202) * dir.x) + (_205 * dir.z));
}

static inline __attribute__((always_inline))
float3 fresnel_schlick_roughness(thread const float& cos_theta, thread const float3& f0, thread const float& roughness)
{
    return f0 + ((fast::max(float3(1.0 - roughness), f0) - f0) * powr(fast::clamp(1.0 - cos_theta, 0.0, 1.0), 5.0));
}

static inline __attribute__((always_inline))
ShadingResult shade_ibl(thread const float3& albedo, thread const float3& world_normal, thread const float3& world_pos, thread const float& roughness, thread const float& metallic, constant scene_fs& sc, texturecube<float> irradiance_cube, sampler ibl_sampler, texturecube<float> prefilter_cube, texture2d<float> brdf_lut)
{
    float3 _251 = fast::normalize(world_normal);
    float3 _260 = fast::normalize(sc.camera_position.xyz - world_pos);
    float _271 = fast::max(dot(_251, _260), 9.9999997473787516355514526367188e-05);
    float3 param = _251;
    float3 param_1 = reflect(-_260, _251);
    float4 _329 = brdf_lut.sample(ibl_sampler, float2(_271, roughness), level(0.0));
    float param_2 = _271;
    float3 param_3 = mix(float3(0.039999999105930328369140625), albedo, float3(metallic));
    float param_4 = roughness;
    float3 _338 = fresnel_schlick_roughness(param_2, param_3, param_4);
    float3 _363 = (((float3(1.0) - _338) * (1.0 - metallic)) * (irradiance_cube.sample(ibl_sampler, env_sample_dir(param, sc), level(0.0)).xyz * albedo)) * sc.env_params.y;
    return ShadingResult{ _363 + ((prefilter_cube.sample(ibl_sampler, env_sample_dir(param_1, sc), level(roughness * sc.env_params.w)).xyz * ((_338 * _329.x) + float3(_329.y))) * sc.env_params.y), _363 };
}

fragment main0_out main0(main0_in in [[stage_in]], constant scene_fs& sc [[buffer(1)]], constant material& _473 [[buffer(2)]], texture2d<float> checker_texture [[texture(0)]], texturecube<float> irradiance_cube [[texture(1)]], texturecube<float> prefilter_cube [[texture(2)]], texture2d<float> brdf_lut [[texture(3)]], texture2d<float> base_color_tex [[texture(4)]], texture2d<float> normal_tex [[texture(5)]], texture2d<float> roughness_tex [[texture(6)]], texture2d<float> metallic_tex [[texture(7)]], texture2d<float> ao_tex [[texture(8)]], texture2d<float> emissive_tex [[texture(9)]], texture2d<float> opacity_tex [[texture(10)]], sampler checker_sampler [[sampler(0)]], sampler ibl_sampler [[sampler(1)]], sampler material_sampler [[sampler(2)]])
{
    main0_out out = {};
    out.frag_ambient = float4(0.0);
    float2 _408 = float2(in.v_uv.x, 1.0 - in.v_uv.y);
    float4 _418 = checker_texture.sample(checker_sampler, (_408 * fast::max(sc.render_options.z, 1.0)));
    float4 _426 = base_color_tex.sample(material_sampler, _408);
    float4 _433 = normal_tex.sample(material_sampler, _408);
    float4 _440 = roughness_tex.sample(material_sampler, _408);
    float4 _447 = metallic_tex.sample(material_sampler, _408);
    float4 _454 = ao_tex.sample(material_sampler, _408);
    float4 _461 = emissive_tex.sample(material_sampler, _408);
    float4 _468 = opacity_tex.sample(material_sampler, _408);
    uint _476 = uint(_473.mat_params.z);
    if (dot(in.v_normal, in.v_normal) < 9.9999999747524270787835121154785e-07)
    {
        float3 param = in.v_color.xyz;
        out.frag_color = float4(srgb_to_linear(param), in.v_color.w);
        out.frag_ambient = float4(0.0, 0.0, 0.0, in.v_color.w);
        return out;
    }
    if (sc.projection_params.w > 0.5)
    {
        float3 param_1 = in.v_color.xyz;
        float3 param_2 = mix(float3(0.046999998390674591064453125, 0.046999998390674591064453125, 0.05200000107288360595703125), srgb_to_linear(param_1), float3(fast::clamp(in.v_color.w, 0.0, 1.0))) * (0.2800000011920928955078125 + (0.7200000286102294921875 * fast::clamp(dot(fast::normalize(in.v_normal), fast::normalize(sc.camera_position.xyz - in.v_world_position)), 0.0, 1.0)));
        out.frag_color = float4(linear_to_srgb(param_2), 1.0);
        out.frag_ambient = float4(0.0, 0.0, 0.0, 1.0);
        return out;
    }
    bool _565 = (_476 & 2u) != 0u;
    bool _570 = (_476 & 4u) != 0u;
    bool _575 = (_476 & 8u) != 0u;
    bool _580 = (_476 & 16u) != 0u;
    bool _585 = (_476 & 32u) != 0u;
    float3 base_color = _473.mat_base_color.xyz;
    if ((_476 & 1u) != 0u)
    {
        if (_473.mat_channels0.x > 3.5)
        {
            base_color *= _426.xyz;
        }
        else
        {
            float4 param_3 = _426;
            float param_4 = _473.mat_channels0.x;
            base_color *= select_channel(param_3, param_4);
        }
    }
    float out_alpha = _473.mat_base_color.w;
    if ((_476 & 64u) != 0u)
    {
        float4 param_5 = _468;
        float param_6 = _473.mat_channels1.z;
        out_alpha *= select_channel(param_5, param_6);
    }
    if (sc.projection_params.z >= 0.0)
    {
        out.frag_ambient = float4(0.0, 0.0, 0.0, 1.0);
        float3 result = float3(0.0);
        if (sc.projection_params.z < 0.5)
        {
            float3 param_7 = base_color;
            result = linear_to_srgb(param_7);
        }
        else
        {
            if (sc.projection_params.z < 1.5)
            {
                float3 wn = fast::normalize(in.v_normal);
                if (_565)
                {
                    float3 param_8 = in.v_normal;
                    float4 param_9 = in.v_tangent;
                    float3 param_10 = _433.xyz;
                    wn = apply_normal_map(param_8, param_9, param_10);
                }
                result = (wn * 0.5) + float3(0.5);
            }
            else
            {
                if (sc.projection_params.z < 2.5)
                {
                    result = _433.xyz;
                }
                else
                {
                    if (sc.projection_params.z < 3.5)
                    {
                        result = (fast::normalize(in.v_normal) * 0.5) + float3(0.5);
                    }
                    else
                    {
                        if (sc.projection_params.z < 4.5)
                        {
                            result = ((fast::normalize(in.v_tangent.xyz) * 0.5) + float3(0.5)) * ((in.v_tangent.w < 0.0) ? 0.5 : 1.0);
                        }
                        else
                        {
                            if (sc.projection_params.z < 5.5)
                            {
                                float r = _473.mat_params.y;
                                if (_570)
                                {
                                    float4 param_11 = _440;
                                    float param_12 = _473.mat_channels0.z;
                                    float _724 = select_channel(param_11, param_12);
                                    if (_473.mat_flags.x > 0.5)
                                    {
                                        r = 1.0 - ((1.0 - r) * _724);
                                    }
                                    else
                                    {
                                        r *= _724;
                                    }
                                }
                                if (_473.mat_flags.x > 0.5)
                                {
                                    r = 1.0 - r;
                                }
                                result = float3(fast::clamp(r, 0.0, 1.0));
                            }
                            else
                            {
                                if (sc.projection_params.z < 6.5)
                                {
                                    float m = _473.mat_params.x;
                                    if (_575)
                                    {
                                        float4 param_13 = _447;
                                        float param_14 = _473.mat_channels0.w;
                                        m *= select_channel(param_13, param_14);
                                    }
                                    result = float3(fast::clamp(m, 0.0, 1.0));
                                }
                                else
                                {
                                    if (sc.projection_params.z < 7.5)
                                    {
                                        float ao = 1.0;
                                        if (_580)
                                        {
                                            float4 param_15 = _454;
                                            float param_16 = _473.mat_channels1.x;
                                            ao = select_channel(param_15, param_16);
                                        }
                                        result = float3(fast::clamp(ao, 0.0, 1.0));
                                    }
                                    else
                                    {
                                        if (sc.projection_params.z < 8.5)
                                        {
                                            float3 em = _473.mat_emissive.xyz;
                                            if (_585)
                                            {
                                                float3 factor = _473.mat_emissive.xyz;
                                                if (all(_473.mat_emissive.xyz <= float3(0.0)))
                                                {
                                                    factor = float3(1.0);
                                                }
                                                if (_473.mat_channels1.y > 3.5)
                                                {
                                                    em = _461.xyz * factor;
                                                }
                                                else
                                                {
                                                    float4 param_17 = _461;
                                                    float param_18 = _473.mat_channels1.y;
                                                    em = float3(select_channel(param_17, param_18)) * factor;
                                                }
                                            }
                                            float3 param_19 = em;
                                            result = linear_to_srgb(param_19);
                                        }
                                        else
                                        {
                                            if (sc.projection_params.z < 9.5)
                                            {
                                                result = float3(fast::clamp(out_alpha, 0.0, 1.0));
                                            }
                                            else
                                            {
                                                result = float3(in.v_uv.x, in.v_uv.y, 0.0);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out.frag_color = float4(result, 1.0);
        return out;
    }
    if (sc.render_options.y > 0.5)
    {
        base_color = _418.xyz;
    }
    if (sc.render_options.w >= 0.0)
    {
        if (sc.render_options.w < 0.5)
        {
            float3 param_20 = in.v_color.xyz;
            base_color = srgb_to_linear(param_20);
            out_alpha = 1.0;
        }
        else
        {
            if (sc.render_options.w < 1.5)
            {
                base_color = float3(in.v_color.w);
                out_alpha = 1.0;
            }
            else
            {
                float3 param_21 = in.v_color.xyz;
                base_color = srgb_to_linear(param_21);
                out_alpha = in.v_color.w;
            }
        }
    }
    bool _893 = _473.mat_params.w > 1.5;
    bool _900;
    if (_893)
    {
        _900 = out_alpha < _473.mat_channels1.w;
    }
    else
    {
        _900 = _893;
    }
    if (_900)
    {
        discard_fragment();
    }
    if (sc.render_options.x < 1.5)
    {
        out.frag_color = float4(base_color, out_alpha);
        return out;
    }
    float3 world_normal = fast::normalize(in.v_normal);
    if (_565)
    {
        float3 param_22 = in.v_normal;
        float4 param_23 = in.v_tangent;
        float3 param_24 = _433.xyz;
        world_normal = apply_normal_map(param_22, param_23, param_24);
    }
    float metallic = _473.mat_params.x;
    if (_575)
    {
        float4 param_25 = _447;
        float param_26 = _473.mat_channels0.w;
        metallic *= select_channel(param_25, param_26);
    }
    metallic = fast::clamp(metallic, 0.0, 1.0);
    float roughness_value = _473.mat_params.y;
    if (_570)
    {
        float4 param_27 = _440;
        float param_28 = _473.mat_channels0.z;
        float _959 = select_channel(param_27, param_28);
        if (_473.mat_flags.x > 0.5)
        {
            roughness_value = 1.0 - ((1.0 - roughness_value) * _959);
        }
        else
        {
            roughness_value *= _959;
        }
    }
    float ao_1 = 1.0;
    if (_580)
    {
        float4 param_29 = _454;
        float param_30 = _473.mat_channels1.x;
        ao_1 = select_channel(param_29, param_30);
    }
    float3 color_linear;
    float3 ambient_linear;
    if (sc.env_params.x > 0.5)
    {
        float3 param_31 = base_color;
        float3 param_32 = world_normal;
        float3 param_33 = in.v_world_position;
        float param_34 = fast::clamp(roughness_value, 0.039999999105930328369140625, 1.0);
        float param_35 = metallic;
        ShadingResult _1005 = shade_ibl(param_31, param_32, param_33, param_34, param_35, sc, irradiance_cube, ibl_sampler, prefilter_cube, brdf_lut);
        color_linear = _1005.color;
        ambient_linear = _1005.ambient;
    }
    else
    {
        float _1016 = fast::clamp(1.0 - roughness_value, 0.0, 1.0);
        float _1026 = fast::max(dot(world_normal, float3(0.3520300388336181640625, 0.824756085872650146484375, 0.442552030086517333984375)), 0.0);
        float3 _1051 = (mix(float3(0.10999999940395355224609375), float3(0.62999999523162841796875), float3(fast::clamp((world_normal.y * 0.5) + 0.5, 0.0, 1.0))) * 0.550000011920928955078125) + float3(0.20000000298023223876953125);
        color_linear = (base_color * (_1051 + (float3(1.0) * (_1026 * 0.75)))) + float3((powr(fast::max(dot(world_normal, fast::normalize(float3(0.3520300388336181640625, 0.824756085872650146484375, 0.442552030086517333984375) + fast::normalize(sc.camera_position.xyz - in.v_world_position))), 0.0), exp2(1.0 + (_1016 * 10.0))) * _1016) * step(0.0, _1026));
        ambient_linear = base_color * _1051;
    }
    color_linear += (ambient_linear * (ao_1 - 1.0));
    ambient_linear *= ao_1;
    float3 emissive = _473.mat_emissive.xyz;
    if (_585)
    {
        float3 factor_1 = _473.mat_emissive.xyz;
        if (all(_473.mat_emissive.xyz <= float3(0.0)))
        {
            factor_1 = float3(1.0);
        }
        if (_473.mat_channels1.y > 3.5)
        {
            emissive = _461.xyz * factor_1;
        }
        else
        {
            float4 param_36 = _461;
            float param_37 = _473.mat_channels1.y;
            emissive = float3(select_channel(param_36, param_37)) * factor_1;
        }
    }
    float3 _1146 = color_linear;
    float3 _1148 = _1146 + emissive;
    color_linear = _1148;
    out.frag_color = float4(_1148, out_alpha);
    out.frag_ambient = float4(ambient_linear, out_alpha);
    return out;
}

