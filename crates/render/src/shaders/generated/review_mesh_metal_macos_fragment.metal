#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct ShadingResult
{
    float3 color;
    float3 ambient;
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
float2 material_uv(thread const float2& uv)
{
    return float2(uv.x, 1.0 - uv.y);
}

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
float material_alpha(thread const float4& tex_opacity, constant material& _165)
{
    float alpha = _165.mat_base_color.w;
    if ((uint(_165.mat_params.z) & 64u) != 0u)
    {
        float4 param = tex_opacity;
        float param_1 = _165.mat_channels1.z;
        alpha *= select_channel(param, param_1);
    }
    return alpha;
}

static inline __attribute__((always_inline))
float3 apply_normal_map(thread const float3& n, thread const float4& tangent, thread const float3& sample_rgb)
{
    float3 _234 = fast::normalize(n);
    float3 _244 = tangent.xyz - (_234 * dot(_234, tangent.xyz));
    float _248 = dot(_244, _244);
    if (_248 < 9.9999999392252902907785028219223e-09)
    {
        return _234;
    }
    float3 _260 = _244 * rsqrt(_248);
    float3 _273 = (sample_rgb * 2.0) - float3(1.0);
    float3 _288 = ((_260 * _273.x) + ((cross(_234, _260) * tangent.w) * _273.y)) + (_234 * _273.z);
    float _292 = dot(_288, _288);
    float3 _296;
    if (_292 > 9.9999999600419720025001879548654e-13)
    {
        _296 = _288 * rsqrt(_292);
    }
    else
    {
        _296 = _234;
    }
    return _296;
}

static inline __attribute__((always_inline))
float vertex_color_alpha(thread const float& alpha, constant scene_fs& sc, thread float4& v_color)
{
    float result = alpha;
    if (sc.render_options.w >= 0.0)
    {
        float _207;
        if (sc.render_options.w < 1.5)
        {
            _207 = 1.0;
        }
        else
        {
            _207 = v_color.w;
        }
        result = _207;
    }
    return result;
}

static inline __attribute__((always_inline))
bool alpha_clipped(thread const float& alpha, constant material& _165)
{
    bool _222 = _165.mat_params.w > 1.5;
    bool _229;
    if (_222)
    {
        _229 = alpha < _165.mat_channels1.w;
    }
    else
    {
        _229 = _222;
    }
    return _229;
}

static inline __attribute__((always_inline))
float3 env_sample_dir(thread const float3& dir, constant scene_fs& sc)
{
    float _312 = -sc.projection_params.y;
    float _315 = sin(_312);
    float _318 = cos(_312);
    return float3((_318 * dir.x) + (_315 * dir.z), dir.y, ((-_315) * dir.x) + (_318 * dir.z));
}

static inline __attribute__((always_inline))
float3 fresnel_schlick_roughness(thread const float& cos_theta, thread const float3& f0, thread const float& roughness)
{
    return f0 + ((fast::max(float3(1.0 - roughness), f0) - f0) * powr(fast::clamp(1.0 - cos_theta, 0.0, 1.0), 5.0));
}

static inline __attribute__((always_inline))
ShadingResult shade_ibl(thread const float3& albedo, thread const float3& world_normal, thread const float3& world_pos, thread const float& roughness, thread const float& metallic, constant scene_fs& sc, texturecube<float> irradiance_cube, sampler ibl_sampler, texturecube<float> prefilter_cube, texture2d<float> brdf_lut)
{
    float3 _364 = fast::normalize(world_normal);
    float3 _373 = fast::normalize(sc.camera_position.xyz - world_pos);
    float _384 = fast::max(dot(_364, _373), 9.9999997473787516355514526367188e-05);
    float3 param = _364;
    float3 param_1 = reflect(-_373, _364);
    float4 _439 = brdf_lut.sample(ibl_sampler, float2(_384, roughness), level(0.0));
    float param_2 = _384;
    float3 param_3 = mix(float3(0.039999999105930328369140625), albedo, float3(metallic));
    float param_4 = roughness;
    float3 _448 = fresnel_schlick_roughness(param_2, param_3, param_4);
    float3 _473 = (((float3(1.0) - _448) * (1.0 - metallic)) * (irradiance_cube.sample(ibl_sampler, env_sample_dir(param, sc), level(0.0)).xyz * albedo)) * sc.env_params.y;
    return ShadingResult{ _473 + ((prefilter_cube.sample(ibl_sampler, env_sample_dir(param_1, sc), level(roughness * sc.env_params.w)).xyz * ((_448 * _439.x) + float3(_439.y))) * sc.env_params.y), _473 };
}

fragment main0_out main0(main0_in in [[stage_in]], constant scene_fs& sc [[buffer(1)]], constant material& _165 [[buffer(2)]], texture2d<float> checker_texture [[texture(0)]], texturecube<float> irradiance_cube [[texture(1)]], texturecube<float> prefilter_cube [[texture(2)]], texture2d<float> brdf_lut [[texture(3)]], texture2d<float> base_color_tex [[texture(4)]], texture2d<float> normal_tex [[texture(5)]], texture2d<float> roughness_tex [[texture(6)]], texture2d<float> metallic_tex [[texture(7)]], texture2d<float> ao_tex [[texture(8)]], texture2d<float> emissive_tex [[texture(9)]], texture2d<float> opacity_tex [[texture(10)]], sampler checker_sampler [[sampler(0)]], sampler ibl_sampler [[sampler(1)]], sampler material_sampler [[sampler(2)]])
{
    main0_out out = {};
    out.frag_ambient = float4(0.0);
    float2 param = in.v_uv;
    float2 _512 = material_uv(param);
    float4 _522 = checker_texture.sample(checker_sampler, (_512 * fast::max(sc.render_options.z, 1.0)));
    float4 _530 = base_color_tex.sample(material_sampler, _512);
    float4 _537 = normal_tex.sample(material_sampler, _512);
    float4 _544 = roughness_tex.sample(material_sampler, _512);
    float4 _551 = metallic_tex.sample(material_sampler, _512);
    float4 _558 = ao_tex.sample(material_sampler, _512);
    float4 _565 = emissive_tex.sample(material_sampler, _512);
    float4 _572 = opacity_tex.sample(material_sampler, _512);
    uint _577 = uint(_165.mat_params.z);
    if (dot(in.v_normal, in.v_normal) < 9.9999999747524270787835121154785e-07)
    {
        float3 param_1 = in.v_color.xyz;
        out.frag_color = float4(srgb_to_linear(param_1), in.v_color.w);
        out.frag_ambient = float4(0.0, 0.0, 0.0, in.v_color.w);
        return out;
    }
    if (sc.projection_params.w > 0.5)
    {
        float3 param_2 = in.v_color.xyz;
        float3 param_3 = mix(float3(0.046999998390674591064453125, 0.046999998390674591064453125, 0.05200000107288360595703125), srgb_to_linear(param_2), float3(fast::clamp(in.v_color.w, 0.0, 1.0))) * (0.2800000011920928955078125 + (0.7200000286102294921875 * fast::clamp(dot(fast::normalize(in.v_normal), fast::normalize(sc.camera_position.xyz - in.v_world_position)), 0.0, 1.0)));
        out.frag_color = float4(linear_to_srgb(param_3), 1.0);
        out.frag_ambient = float4(0.0, 0.0, 0.0, 1.0);
        return out;
    }
    bool _664 = (_577 & 2u) != 0u;
    bool _669 = (_577 & 4u) != 0u;
    bool _674 = (_577 & 8u) != 0u;
    bool _679 = (_577 & 16u) != 0u;
    bool _684 = (_577 & 32u) != 0u;
    float3 base_color = _165.mat_base_color.xyz;
    if ((_577 & 1u) != 0u)
    {
        if (_165.mat_channels0.x > 3.5)
        {
            base_color *= _530.xyz;
        }
        else
        {
            float4 param_4 = _530;
            float param_5 = _165.mat_channels0.x;
            base_color *= select_channel(param_4, param_5);
        }
    }
    float4 param_6 = _572;
    float out_alpha = material_alpha(param_6, _165);
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
                if (_664)
                {
                    float3 param_8 = in.v_normal;
                    float4 param_9 = in.v_tangent;
                    float3 param_10 = _537.xyz;
                    wn = apply_normal_map(param_8, param_9, param_10);
                }
                result = (wn * 0.5) + float3(0.5);
            }
            else
            {
                if (sc.projection_params.z < 2.5)
                {
                    result = _537.xyz;
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
                                float r = _165.mat_params.y;
                                if (_669)
                                {
                                    float4 param_11 = _544;
                                    float param_12 = _165.mat_channels0.z;
                                    float _808 = select_channel(param_11, param_12);
                                    if (_165.mat_flags.x > 0.5)
                                    {
                                        r = 1.0 - ((1.0 - r) * _808);
                                    }
                                    else
                                    {
                                        r *= _808;
                                    }
                                }
                                if (_165.mat_flags.x > 0.5)
                                {
                                    r = 1.0 - r;
                                }
                                result = float3(fast::clamp(r, 0.0, 1.0));
                            }
                            else
                            {
                                if (sc.projection_params.z < 6.5)
                                {
                                    float m = _165.mat_params.x;
                                    if (_674)
                                    {
                                        float4 param_13 = _551;
                                        float param_14 = _165.mat_channels0.w;
                                        m *= select_channel(param_13, param_14);
                                    }
                                    result = float3(fast::clamp(m, 0.0, 1.0));
                                }
                                else
                                {
                                    if (sc.projection_params.z < 7.5)
                                    {
                                        float ao = 1.0;
                                        if (_679)
                                        {
                                            float4 param_15 = _558;
                                            float param_16 = _165.mat_channels1.x;
                                            ao = select_channel(param_15, param_16);
                                        }
                                        result = float3(fast::clamp(ao, 0.0, 1.0));
                                    }
                                    else
                                    {
                                        if (sc.projection_params.z < 8.5)
                                        {
                                            float3 em = _165.mat_emissive.xyz;
                                            if (_684)
                                            {
                                                float3 factor = _165.mat_emissive.xyz;
                                                if (all(_165.mat_emissive.xyz <= float3(0.0)))
                                                {
                                                    factor = float3(1.0);
                                                }
                                                if (_165.mat_channels1.y > 3.5)
                                                {
                                                    em = _565.xyz * factor;
                                                }
                                                else
                                                {
                                                    float4 param_17 = _565;
                                                    float param_18 = _165.mat_channels1.y;
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
        base_color = _522.xyz;
    }
    if (sc.render_options.w >= 0.0)
    {
        if ((sc.render_options.w > 0.5) && (sc.render_options.w < 1.5))
        {
            base_color = float3(in.v_color.w);
        }
        else
        {
            float3 param_20 = in.v_color.xyz;
            base_color = srgb_to_linear(param_20);
        }
    }
    float param_21 = out_alpha;
    float _967 = vertex_color_alpha(param_21, sc, in.v_color);
    out_alpha = _967;
    float param_22 = _967;
    if (alpha_clipped(param_22, _165))
    {
        discard_fragment();
    }
    if (sc.render_options.x < 1.5)
    {
        out.frag_color = float4(base_color, out_alpha);
        return out;
    }
    float3 world_normal = fast::normalize(in.v_normal);
    if (_664)
    {
        float3 param_23 = in.v_normal;
        float4 param_24 = in.v_tangent;
        float3 param_25 = _537.xyz;
        world_normal = apply_normal_map(param_23, param_24, param_25);
    }
    float metallic = _165.mat_params.x;
    if (_674)
    {
        float4 param_26 = _551;
        float param_27 = _165.mat_channels0.w;
        metallic *= select_channel(param_26, param_27);
    }
    metallic = fast::clamp(metallic, 0.0, 1.0);
    float roughness_value = _165.mat_params.y;
    if (_669)
    {
        float4 param_28 = _544;
        float param_29 = _165.mat_channels0.z;
        float _1029 = select_channel(param_28, param_29);
        if (_165.mat_flags.x > 0.5)
        {
            roughness_value = 1.0 - ((1.0 - roughness_value) * _1029);
        }
        else
        {
            roughness_value *= _1029;
        }
    }
    float ao_1 = 1.0;
    if (_679)
    {
        float4 param_30 = _558;
        float param_31 = _165.mat_channels1.x;
        ao_1 = select_channel(param_30, param_31);
    }
    float3 color_linear;
    float3 ambient_linear;
    if (sc.env_params.x > 0.5)
    {
        float3 param_32 = base_color;
        float3 param_33 = world_normal;
        float3 param_34 = in.v_world_position;
        float param_35 = fast::clamp(roughness_value, 0.039999999105930328369140625, 1.0);
        float param_36 = metallic;
        ShadingResult _1075 = shade_ibl(param_32, param_33, param_34, param_35, param_36, sc, irradiance_cube, ibl_sampler, prefilter_cube, brdf_lut);
        color_linear = _1075.color;
        ambient_linear = _1075.ambient;
    }
    else
    {
        float _1086 = fast::clamp(1.0 - roughness_value, 0.0, 1.0);
        float _1096 = fast::max(dot(world_normal, float3(0.3520300388336181640625, 0.824756085872650146484375, 0.442552030086517333984375)), 0.0);
        float3 _1121 = (mix(float3(0.10999999940395355224609375), float3(0.62999999523162841796875), float3(fast::clamp((world_normal.y * 0.5) + 0.5, 0.0, 1.0))) * 0.550000011920928955078125) + float3(0.20000000298023223876953125);
        color_linear = (base_color * (_1121 + (float3(1.0) * (_1096 * 0.75)))) + float3((powr(fast::max(dot(world_normal, fast::normalize(float3(0.3520300388336181640625, 0.824756085872650146484375, 0.442552030086517333984375) + fast::normalize(sc.camera_position.xyz - in.v_world_position))), 0.0), exp2(1.0 + (_1086 * 10.0))) * _1086) * step(0.0, _1096));
        ambient_linear = base_color * _1121;
    }
    color_linear += (ambient_linear * (ao_1 - 1.0));
    ambient_linear *= ao_1;
    float3 emissive = _165.mat_emissive.xyz;
    if (_684)
    {
        float3 factor_1 = _165.mat_emissive.xyz;
        if (all(_165.mat_emissive.xyz <= float3(0.0)))
        {
            factor_1 = float3(1.0);
        }
        if (_165.mat_channels1.y > 3.5)
        {
            emissive = _565.xyz * factor_1;
        }
        else
        {
            float4 param_37 = _565;
            float param_38 = _165.mat_channels1.y;
            emissive = float3(select_channel(param_37, param_38)) * factor_1;
        }
    }
    float3 _1216 = color_linear;
    float3 _1218 = _1216 + emissive;
    color_linear = _1218;
    out.frag_color = float4(_1218, out_alpha);
    out.frag_ambient = float4(ambient_linear, out_alpha);
    return out;
}

