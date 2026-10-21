struct ShadingResult
{
    float3 color;
    float3 ambient;
};

cbuffer material : register(b2)
{
    float4 _165_mat_base_color : packoffset(c0);
    float4 _165_mat_emissive : packoffset(c1);
    float4 _165_mat_params : packoffset(c2);
    float4 _165_mat_channels0 : packoffset(c3);
    float4 _165_mat_channels1 : packoffset(c4);
    float4 _165_mat_flags : packoffset(c5);
};

cbuffer scene_fs : register(b1)
{
    row_major float4x4 sc_view_projection : packoffset(c0);
    row_major float4x4 sc_inv_view_projection : packoffset(c4);
    float4 sc_render_options : packoffset(c8);
    float4 sc_camera_position : packoffset(c9);
    float4 sc_env_params : packoffset(c10);
    float4 sc_projection_params : packoffset(c11);
    row_major float4x4 sc_view : packoffset(c12);
    float4 sc_selection_color : packoffset(c16);
};

TextureCube<float4> irradiance_cube : register(t1);
SamplerState ibl_sampler : register(s1);
TextureCube<float4> prefilter_cube : register(t2);
Texture2D<float4> brdf_lut : register(t3);
Texture2D<float4> checker_texture : register(t0);
SamplerState checker_sampler : register(s0);
Texture2D<float4> base_color_tex : register(t4);
SamplerState material_sampler : register(s2);
Texture2D<float4> normal_tex : register(t5);
Texture2D<float4> roughness_tex : register(t6);
Texture2D<float4> metallic_tex : register(t7);
Texture2D<float4> ao_tex : register(t8);
Texture2D<float4> emissive_tex : register(t9);
Texture2D<float4> opacity_tex : register(t10);

static float4 v_color;
static float4 frag_ambient;
static float2 v_uv;
static float3 v_normal;
static float4 frag_color;
static float3 v_world_position;
static float4 v_tangent;

struct SPIRV_Cross_Input
{
    float3 v_normal : TEXCOORD0;
    float2 v_uv : TEXCOORD1;
    float4 v_color : TEXCOORD2;
    float3 v_world_position : TEXCOORD3;
    float4 v_tangent : TEXCOORD4;
};

struct SPIRV_Cross_Output
{
    float4 frag_color : SV_Target0;
    float4 frag_ambient : SV_Target1;
};

float2 material_uv(float2 uv)
{
    return float2(uv.x, 1.0f - uv.y);
}

float3 srgb_to_linear(float3 c)
{
    return lerp(pow(max((c + 0.054999999701976776123046875f.xxx) * 0.947867333889007568359375f.xxx, 0.0f.xxx), 2.400000095367431640625f.xxx), c * 0.077399380505084991455078125f.xxx, step(c, 0.040449999272823333740234375f.xxx));
}

float3 linear_to_srgb(float3 c)
{
    return lerp((pow(max(c, 0.0f.xxx), 0.4166666567325592041015625f.xxx) * 1.05499994754791259765625f) - 0.054999999701976776123046875f.xxx, c * 12.9200000762939453125f, step(c, 0.003130800090730190277099609375f.xxx));
}

float select_channel(float4 texel, float index)
{
    if (index < 0.5f)
    {
        return texel.x;
    }
    if (index < 1.5f)
    {
        return texel.y;
    }
    if (index < 2.5f)
    {
        return texel.z;
    }
    return texel.w;
}

float material_alpha(float4 tex_opacity)
{
    float alpha = _165_mat_base_color.w;
    if ((uint(_165_mat_params.z) & 64u) != 0u)
    {
        float4 param = tex_opacity;
        float param_1 = _165_mat_channels1.z;
        alpha *= select_channel(param, param_1);
    }
    return alpha;
}

float3 apply_normal_map(float3 n, float4 tangent, float3 sample_rgb)
{
    float3 _234 = normalize(n);
    float3 _244 = tangent.xyz - (_234 * dot(_234, tangent.xyz));
    float _248 = dot(_244, _244);
    if (_248 < 9.9999999392252902907785028219223e-09f)
    {
        return _234;
    }
    float3 _260 = _244 * rsqrt(_248);
    float3 _273 = (sample_rgb * 2.0f) - 1.0f.xxx;
    float3 _288 = ((_260 * _273.x) + ((cross(_234, _260) * tangent.w) * _273.y)) + (_234 * _273.z);
    float _292 = dot(_288, _288);
    float3 _296;
    if (_292 > 9.9999999600419720025001879548654e-13f)
    {
        _296 = _288 * rsqrt(_292);
    }
    else
    {
        _296 = _234;
    }
    return _296;
}

float vertex_color_alpha(float alpha)
{
    float result = alpha;
    if (sc_render_options.w >= 0.0f)
    {
        float _207;
        if (sc_render_options.w < 1.5f)
        {
            _207 = 1.0f;
        }
        else
        {
            _207 = v_color.w;
        }
        result = _207;
    }
    return result;
}

bool alpha_clipped(float alpha)
{
    bool _222 = _165_mat_params.w > 1.5f;
    bool _229;
    if (_222)
    {
        _229 = alpha < _165_mat_channels1.w;
    }
    else
    {
        _229 = _222;
    }
    return _229;
}

float3 env_sample_dir(float3 dir)
{
    float _312 = -sc_projection_params.y;
    float _315 = sin(_312);
    float _318 = cos(_312);
    return float3((_318 * dir.x) + (_315 * dir.z), dir.y, ((-_315) * dir.x) + (_318 * dir.z));
}

float3 fresnel_schlick_roughness(float cos_theta, float3 f0, float roughness)
{
    return f0 + ((max((1.0f - roughness).xxx, f0) - f0) * pow(clamp(1.0f - cos_theta, 0.0f, 1.0f), 5.0f));
}

ShadingResult shade_ibl(float3 albedo, float3 world_normal, float3 world_pos, float roughness, float metallic)
{
    float3 _364 = normalize(world_normal);
    float3 _373 = normalize(sc_camera_position.xyz - world_pos);
    float _384 = max(dot(_364, _373), 9.9999997473787516355514526367188e-05f);
    float3 param = _364;
    float3 param_1 = reflect(-_373, _364);
    float4 _439 = brdf_lut.SampleLevel(ibl_sampler, float2(_384, roughness), 0.0f);
    float param_2 = _384;
    float3 param_3 = lerp(0.039999999105930328369140625f.xxx, albedo, metallic.xxx);
    float param_4 = roughness;
    float3 _448 = fresnel_schlick_roughness(param_2, param_3, param_4);
    float3 _473 = (((1.0f.xxx - _448) * (1.0f - metallic)) * (irradiance_cube.SampleLevel(ibl_sampler, env_sample_dir(param), 0.0f).xyz * albedo)) * sc_env_params.y;
    ShadingResult _1239 = { _473 + ((prefilter_cube.SampleLevel(ibl_sampler, env_sample_dir(param_1), roughness * sc_env_params.w).xyz * ((_448 * _439.x) + _439.y.xxx)) * sc_env_params.y), _473 };
    return _1239;
}

void frag_main()
{
    frag_ambient = 0.0f.xxxx;
    float2 param = v_uv;
    float2 _512 = material_uv(param);
    float4 _522 = checker_texture.Sample(checker_sampler, _512 * max(sc_render_options.z, 1.0f));
    float4 _530 = base_color_tex.Sample(material_sampler, _512);
    float4 _537 = normal_tex.Sample(material_sampler, _512);
    float4 _544 = roughness_tex.Sample(material_sampler, _512);
    float4 _551 = metallic_tex.Sample(material_sampler, _512);
    float4 _558 = ao_tex.Sample(material_sampler, _512);
    float4 _565 = emissive_tex.Sample(material_sampler, _512);
    float4 _572 = opacity_tex.Sample(material_sampler, _512);
    uint _577 = uint(_165_mat_params.z);
    if (dot(v_normal, v_normal) < 9.9999999747524270787835121154785e-07f)
    {
        float3 param_1 = v_color.xyz;
        frag_color = float4(srgb_to_linear(param_1), v_color.w);
        frag_ambient = float4(0.0f, 0.0f, 0.0f, v_color.w);
        return;
    }
    if (sc_projection_params.w > 0.5f)
    {
        float3 param_2 = v_color.xyz;
        float3 param_3 = lerp(float3(0.046999998390674591064453125f, 0.046999998390674591064453125f, 0.05200000107288360595703125f), srgb_to_linear(param_2), clamp(v_color.w, 0.0f, 1.0f).xxx) * (0.2800000011920928955078125f + (0.7200000286102294921875f * clamp(dot(normalize(v_normal), normalize(sc_camera_position.xyz - v_world_position)), 0.0f, 1.0f)));
        frag_color = float4(linear_to_srgb(param_3), 1.0f);
        frag_ambient = float4(0.0f, 0.0f, 0.0f, 1.0f);
        return;
    }
    bool _664 = (_577 & 2u) != 0u;
    bool _669 = (_577 & 4u) != 0u;
    bool _674 = (_577 & 8u) != 0u;
    bool _679 = (_577 & 16u) != 0u;
    bool _684 = (_577 & 32u) != 0u;
    float3 base_color = _165_mat_base_color.xyz;
    if ((_577 & 1u) != 0u)
    {
        if (_165_mat_channels0.x > 3.5f)
        {
            base_color *= _530.xyz;
        }
        else
        {
            float4 param_4 = _530;
            float param_5 = _165_mat_channels0.x;
            base_color *= select_channel(param_4, param_5);
        }
    }
    float4 param_6 = _572;
    float out_alpha = material_alpha(param_6);
    if (sc_projection_params.z >= 0.0f)
    {
        frag_ambient = float4(0.0f, 0.0f, 0.0f, 1.0f);
        float3 result = 0.0f.xxx;
        if (sc_projection_params.z < 0.5f)
        {
            float3 param_7 = base_color;
            result = linear_to_srgb(param_7);
        }
        else
        {
            if (sc_projection_params.z < 1.5f)
            {
                float3 wn = normalize(v_normal);
                if (_664)
                {
                    float3 param_8 = v_normal;
                    float4 param_9 = v_tangent;
                    float3 param_10 = _537.xyz;
                    wn = apply_normal_map(param_8, param_9, param_10);
                }
                result = (wn * 0.5f) + 0.5f.xxx;
            }
            else
            {
                if (sc_projection_params.z < 2.5f)
                {
                    result = _537.xyz;
                }
                else
                {
                    if (sc_projection_params.z < 3.5f)
                    {
                        result = (normalize(v_normal) * 0.5f) + 0.5f.xxx;
                    }
                    else
                    {
                        if (sc_projection_params.z < 4.5f)
                        {
                            result = ((normalize(v_tangent.xyz) * 0.5f) + 0.5f.xxx) * ((v_tangent.w < 0.0f) ? 0.5f : 1.0f);
                        }
                        else
                        {
                            if (sc_projection_params.z < 5.5f)
                            {
                                float r = _165_mat_params.y;
                                if (_669)
                                {
                                    float4 param_11 = _544;
                                    float param_12 = _165_mat_channels0.z;
                                    float _808 = select_channel(param_11, param_12);
                                    if (_165_mat_flags.x > 0.5f)
                                    {
                                        r = 1.0f - ((1.0f - r) * _808);
                                    }
                                    else
                                    {
                                        r *= _808;
                                    }
                                }
                                if (_165_mat_flags.x > 0.5f)
                                {
                                    r = 1.0f - r;
                                }
                                result = clamp(r, 0.0f, 1.0f).xxx;
                            }
                            else
                            {
                                if (sc_projection_params.z < 6.5f)
                                {
                                    float m = _165_mat_params.x;
                                    if (_674)
                                    {
                                        float4 param_13 = _551;
                                        float param_14 = _165_mat_channels0.w;
                                        m *= select_channel(param_13, param_14);
                                    }
                                    result = clamp(m, 0.0f, 1.0f).xxx;
                                }
                                else
                                {
                                    if (sc_projection_params.z < 7.5f)
                                    {
                                        float ao = 1.0f;
                                        if (_679)
                                        {
                                            float4 param_15 = _558;
                                            float param_16 = _165_mat_channels1.x;
                                            ao = select_channel(param_15, param_16);
                                        }
                                        result = clamp(ao, 0.0f, 1.0f).xxx;
                                    }
                                    else
                                    {
                                        if (sc_projection_params.z < 8.5f)
                                        {
                                            float3 em = _165_mat_emissive.xyz;
                                            if (_684)
                                            {
                                                float3 factor = _165_mat_emissive.xyz;
                                                if (all(bool3(_165_mat_emissive.xyz.x <= 0.0f.xxx.x, _165_mat_emissive.xyz.y <= 0.0f.xxx.y, _165_mat_emissive.xyz.z <= 0.0f.xxx.z)))
                                                {
                                                    factor = 1.0f.xxx;
                                                }
                                                if (_165_mat_channels1.y > 3.5f)
                                                {
                                                    em = _565.xyz * factor;
                                                }
                                                else
                                                {
                                                    float4 param_17 = _565;
                                                    float param_18 = _165_mat_channels1.y;
                                                    em = select_channel(param_17, param_18).xxx * factor;
                                                }
                                            }
                                            float3 param_19 = em;
                                            result = linear_to_srgb(param_19);
                                        }
                                        else
                                        {
                                            if (sc_projection_params.z < 9.5f)
                                            {
                                                result = clamp(out_alpha, 0.0f, 1.0f).xxx;
                                            }
                                            else
                                            {
                                                result = float3(v_uv.x, v_uv.y, 0.0f);
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
        frag_color = float4(result, 1.0f);
        return;
    }
    if (sc_render_options.y > 0.5f)
    {
        base_color = _522.xyz;
    }
    if (sc_render_options.w >= 0.0f)
    {
        if ((sc_render_options.w > 0.5f) && (sc_render_options.w < 1.5f))
        {
            base_color = v_color.w.xxx;
        }
        else
        {
            float3 param_20 = v_color.xyz;
            base_color = srgb_to_linear(param_20);
        }
    }
    float param_21 = out_alpha;
    float _967 = vertex_color_alpha(param_21);
    out_alpha = _967;
    float param_22 = _967;
    if (alpha_clipped(param_22))
    {
        discard;
    }
    if (sc_render_options.x < 1.5f)
    {
        frag_color = float4(base_color, out_alpha);
        return;
    }
    float3 world_normal = normalize(v_normal);
    if (_664)
    {
        float3 param_23 = v_normal;
        float4 param_24 = v_tangent;
        float3 param_25 = _537.xyz;
        world_normal = apply_normal_map(param_23, param_24, param_25);
    }
    float metallic = _165_mat_params.x;
    if (_674)
    {
        float4 param_26 = _551;
        float param_27 = _165_mat_channels0.w;
        metallic *= select_channel(param_26, param_27);
    }
    metallic = clamp(metallic, 0.0f, 1.0f);
    float roughness_value = _165_mat_params.y;
    if (_669)
    {
        float4 param_28 = _544;
        float param_29 = _165_mat_channels0.z;
        float _1029 = select_channel(param_28, param_29);
        if (_165_mat_flags.x > 0.5f)
        {
            roughness_value = 1.0f - ((1.0f - roughness_value) * _1029);
        }
        else
        {
            roughness_value *= _1029;
        }
    }
    float ao_1 = 1.0f;
    if (_679)
    {
        float4 param_30 = _558;
        float param_31 = _165_mat_channels1.x;
        ao_1 = select_channel(param_30, param_31);
    }
    float3 color_linear;
    float3 ambient_linear;
    if (sc_env_params.x > 0.5f)
    {
        float3 param_32 = base_color;
        float3 param_33 = world_normal;
        float3 param_34 = v_world_position;
        float param_35 = clamp(roughness_value, 0.039999999105930328369140625f, 1.0f);
        float param_36 = metallic;
        ShadingResult _1075 = shade_ibl(param_32, param_33, param_34, param_35, param_36);
        color_linear = _1075.color;
        ambient_linear = _1075.ambient;
    }
    else
    {
        float _1086 = clamp(1.0f - roughness_value, 0.0f, 1.0f);
        float _1096 = max(dot(world_normal, float3(0.3520300388336181640625f, 0.824756085872650146484375f, 0.442552030086517333984375f)), 0.0f);
        float3 _1121 = (lerp(0.10999999940395355224609375f.xxx, 0.62999999523162841796875f.xxx, clamp((world_normal.y * 0.5f) + 0.5f, 0.0f, 1.0f).xxx) * 0.550000011920928955078125f) + 0.20000000298023223876953125f.xxx;
        color_linear = (base_color * (_1121 + (1.0f.xxx * (_1096 * 0.75f)))) + ((pow(max(dot(world_normal, normalize(float3(0.3520300388336181640625f, 0.824756085872650146484375f, 0.442552030086517333984375f) + normalize(sc_camera_position.xyz - v_world_position))), 0.0f), exp2(1.0f + (_1086 * 10.0f))) * _1086) * step(0.0f, _1096)).xxx;
        ambient_linear = base_color * _1121;
    }
    color_linear += (ambient_linear * (ao_1 - 1.0f));
    ambient_linear *= ao_1;
    float3 emissive = _165_mat_emissive.xyz;
    if (_684)
    {
        float3 factor_1 = _165_mat_emissive.xyz;
        if (all(bool3(_165_mat_emissive.xyz.x <= 0.0f.xxx.x, _165_mat_emissive.xyz.y <= 0.0f.xxx.y, _165_mat_emissive.xyz.z <= 0.0f.xxx.z)))
        {
            factor_1 = 1.0f.xxx;
        }
        if (_165_mat_channels1.y > 3.5f)
        {
            emissive = _565.xyz * factor_1;
        }
        else
        {
            float4 param_37 = _565;
            float param_38 = _165_mat_channels1.y;
            emissive = select_channel(param_37, param_38).xxx * factor_1;
        }
    }
    float3 _1216 = color_linear;
    float3 _1218 = _1216 + emissive;
    color_linear = _1218;
    frag_color = float4(_1218, out_alpha);
    frag_ambient = float4(ambient_linear, out_alpha);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_color = stage_input.v_color;
    v_uv = stage_input.v_uv;
    v_normal = stage_input.v_normal;
    v_world_position = stage_input.v_world_position;
    v_tangent = stage_input.v_tangent;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_ambient = frag_ambient;
    stage_output.frag_color = frag_color;
    return stage_output;
}
