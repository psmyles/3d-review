struct ShadingResult
{
    float3 color;
    float3 ambient;
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

cbuffer material : register(b2)
{
    float4 _473_mat_base_color : packoffset(c0);
    float4 _473_mat_emissive : packoffset(c1);
    float4 _473_mat_params : packoffset(c2);
    float4 _473_mat_channels0 : packoffset(c3);
    float4 _473_mat_channels1 : packoffset(c4);
    float4 _473_mat_flags : packoffset(c5);
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

static float4 frag_ambient;
static float2 v_uv;
static float3 v_normal;
static float4 frag_color;
static float4 v_color;
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

float3 apply_normal_map(float3 n, float4 tangent, float3 sample_rgb)
{
    float3 _137 = normalize(n);
    float3 _148 = normalize(tangent.xyz - (_137 * dot(_137, tangent.xyz)));
    if (dot(_148, _148) < 9.9999999392252902907785028219223e-09f)
    {
        return _137;
    }
    float3 _171 = (sample_rgb * 2.0f) - 1.0f.xxx;
    return normalize(((_148 * _171.x) + ((cross(_137, _148) * tangent.w) * _171.y)) + (_137 * _171.z));
}

float3 env_sample_dir(float3 dir)
{
    float _199 = -sc_projection_params.y;
    float _202 = sin(_199);
    float _205 = cos(_199);
    return float3((_205 * dir.x) + (_202 * dir.z), dir.y, ((-_202) * dir.x) + (_205 * dir.z));
}

float3 fresnel_schlick_roughness(float cos_theta, float3 f0, float roughness)
{
    return f0 + ((max((1.0f - roughness).xxx, f0) - f0) * pow(clamp(1.0f - cos_theta, 0.0f, 1.0f), 5.0f));
}

ShadingResult shade_ibl(float3 albedo, float3 world_normal, float3 world_pos, float roughness, float metallic)
{
    float3 _251 = normalize(world_normal);
    float3 _260 = normalize(sc_camera_position.xyz - world_pos);
    float _271 = max(dot(_251, _260), 9.9999997473787516355514526367188e-05f);
    float3 param = _251;
    float3 param_1 = reflect(-_260, _251);
    float4 _329 = brdf_lut.SampleLevel(ibl_sampler, float2(_271, roughness), 0.0f);
    float param_2 = _271;
    float3 param_3 = lerp(0.039999999105930328369140625f.xxx, albedo, metallic.xxx);
    float param_4 = roughness;
    float3 _338 = fresnel_schlick_roughness(param_2, param_3, param_4);
    float3 _363 = (((1.0f.xxx - _338) * (1.0f - metallic)) * (irradiance_cube.SampleLevel(ibl_sampler, env_sample_dir(param), 0.0f).xyz * albedo)) * sc_env_params.y;
    ShadingResult _1169 = { _363 + ((prefilter_cube.SampleLevel(ibl_sampler, env_sample_dir(param_1), roughness * sc_env_params.w).xyz * ((_338 * _329.x) + _329.y.xxx)) * sc_env_params.y), _363 };
    return _1169;
}

void frag_main()
{
    frag_ambient = 0.0f.xxxx;
    float2 _408 = float2(v_uv.x, 1.0f - v_uv.y);
    float4 _418 = checker_texture.Sample(checker_sampler, _408 * max(sc_render_options.z, 1.0f));
    float4 _426 = base_color_tex.Sample(material_sampler, _408);
    float4 _433 = normal_tex.Sample(material_sampler, _408);
    float4 _440 = roughness_tex.Sample(material_sampler, _408);
    float4 _447 = metallic_tex.Sample(material_sampler, _408);
    float4 _454 = ao_tex.Sample(material_sampler, _408);
    float4 _461 = emissive_tex.Sample(material_sampler, _408);
    float4 _468 = opacity_tex.Sample(material_sampler, _408);
    uint _476 = uint(_473_mat_params.z);
    if (dot(v_normal, v_normal) < 9.9999999747524270787835121154785e-07f)
    {
        float3 param = v_color.xyz;
        frag_color = float4(srgb_to_linear(param), v_color.w);
        frag_ambient = float4(0.0f, 0.0f, 0.0f, v_color.w);
        return;
    }
    if (sc_projection_params.w > 0.5f)
    {
        float3 param_1 = v_color.xyz;
        float3 param_2 = lerp(float3(0.046999998390674591064453125f, 0.046999998390674591064453125f, 0.05200000107288360595703125f), srgb_to_linear(param_1), clamp(v_color.w, 0.0f, 1.0f).xxx) * (0.2800000011920928955078125f + (0.7200000286102294921875f * clamp(dot(normalize(v_normal), normalize(sc_camera_position.xyz - v_world_position)), 0.0f, 1.0f)));
        frag_color = float4(linear_to_srgb(param_2), 1.0f);
        frag_ambient = float4(0.0f, 0.0f, 0.0f, 1.0f);
        return;
    }
    bool _565 = (_476 & 2u) != 0u;
    bool _570 = (_476 & 4u) != 0u;
    bool _575 = (_476 & 8u) != 0u;
    bool _580 = (_476 & 16u) != 0u;
    bool _585 = (_476 & 32u) != 0u;
    float3 base_color = _473_mat_base_color.xyz;
    if ((_476 & 1u) != 0u)
    {
        if (_473_mat_channels0.x > 3.5f)
        {
            base_color *= _426.xyz;
        }
        else
        {
            float4 param_3 = _426;
            float param_4 = _473_mat_channels0.x;
            base_color *= select_channel(param_3, param_4);
        }
    }
    float out_alpha = _473_mat_base_color.w;
    if ((_476 & 64u) != 0u)
    {
        float4 param_5 = _468;
        float param_6 = _473_mat_channels1.z;
        out_alpha *= select_channel(param_5, param_6);
    }
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
                if (_565)
                {
                    float3 param_8 = v_normal;
                    float4 param_9 = v_tangent;
                    float3 param_10 = _433.xyz;
                    wn = apply_normal_map(param_8, param_9, param_10);
                }
                result = (wn * 0.5f) + 0.5f.xxx;
            }
            else
            {
                if (sc_projection_params.z < 2.5f)
                {
                    result = _433.xyz;
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
                                float r = _473_mat_params.y;
                                if (_570)
                                {
                                    float4 param_11 = _440;
                                    float param_12 = _473_mat_channels0.z;
                                    float _724 = select_channel(param_11, param_12);
                                    if (_473_mat_flags.x > 0.5f)
                                    {
                                        r = 1.0f - ((1.0f - r) * _724);
                                    }
                                    else
                                    {
                                        r *= _724;
                                    }
                                }
                                if (_473_mat_flags.x > 0.5f)
                                {
                                    r = 1.0f - r;
                                }
                                result = clamp(r, 0.0f, 1.0f).xxx;
                            }
                            else
                            {
                                if (sc_projection_params.z < 6.5f)
                                {
                                    float m = _473_mat_params.x;
                                    if (_575)
                                    {
                                        float4 param_13 = _447;
                                        float param_14 = _473_mat_channels0.w;
                                        m *= select_channel(param_13, param_14);
                                    }
                                    result = clamp(m, 0.0f, 1.0f).xxx;
                                }
                                else
                                {
                                    if (sc_projection_params.z < 7.5f)
                                    {
                                        float ao = 1.0f;
                                        if (_580)
                                        {
                                            float4 param_15 = _454;
                                            float param_16 = _473_mat_channels1.x;
                                            ao = select_channel(param_15, param_16);
                                        }
                                        result = clamp(ao, 0.0f, 1.0f).xxx;
                                    }
                                    else
                                    {
                                        if (sc_projection_params.z < 8.5f)
                                        {
                                            float3 em = _473_mat_emissive.xyz;
                                            if (_585)
                                            {
                                                float3 factor = _473_mat_emissive.xyz;
                                                if (all(bool3(_473_mat_emissive.xyz.x <= 0.0f.xxx.x, _473_mat_emissive.xyz.y <= 0.0f.xxx.y, _473_mat_emissive.xyz.z <= 0.0f.xxx.z)))
                                                {
                                                    factor = 1.0f.xxx;
                                                }
                                                if (_473_mat_channels1.y > 3.5f)
                                                {
                                                    em = _461.xyz * factor;
                                                }
                                                else
                                                {
                                                    float4 param_17 = _461;
                                                    float param_18 = _473_mat_channels1.y;
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
        base_color = _418.xyz;
    }
    if (sc_render_options.w >= 0.0f)
    {
        if (sc_render_options.w < 0.5f)
        {
            float3 param_20 = v_color.xyz;
            base_color = srgb_to_linear(param_20);
            out_alpha = 1.0f;
        }
        else
        {
            if (sc_render_options.w < 1.5f)
            {
                base_color = v_color.w.xxx;
                out_alpha = 1.0f;
            }
            else
            {
                float3 param_21 = v_color.xyz;
                base_color = srgb_to_linear(param_21);
                out_alpha = v_color.w;
            }
        }
    }
    bool _893 = _473_mat_params.w > 1.5f;
    bool _900;
    if (_893)
    {
        _900 = out_alpha < _473_mat_channels1.w;
    }
    else
    {
        _900 = _893;
    }
    if (_900)
    {
        discard;
    }
    if (sc_render_options.x < 1.5f)
    {
        frag_color = float4(base_color, out_alpha);
        return;
    }
    float3 world_normal = normalize(v_normal);
    if (_565)
    {
        float3 param_22 = v_normal;
        float4 param_23 = v_tangent;
        float3 param_24 = _433.xyz;
        world_normal = apply_normal_map(param_22, param_23, param_24);
    }
    float metallic = _473_mat_params.x;
    if (_575)
    {
        float4 param_25 = _447;
        float param_26 = _473_mat_channels0.w;
        metallic *= select_channel(param_25, param_26);
    }
    metallic = clamp(metallic, 0.0f, 1.0f);
    float roughness_value = _473_mat_params.y;
    if (_570)
    {
        float4 param_27 = _440;
        float param_28 = _473_mat_channels0.z;
        float _959 = select_channel(param_27, param_28);
        if (_473_mat_flags.x > 0.5f)
        {
            roughness_value = 1.0f - ((1.0f - roughness_value) * _959);
        }
        else
        {
            roughness_value *= _959;
        }
    }
    float ao_1 = 1.0f;
    if (_580)
    {
        float4 param_29 = _454;
        float param_30 = _473_mat_channels1.x;
        ao_1 = select_channel(param_29, param_30);
    }
    float3 color_linear;
    float3 ambient_linear;
    if (sc_env_params.x > 0.5f)
    {
        float3 param_31 = base_color;
        float3 param_32 = world_normal;
        float3 param_33 = v_world_position;
        float param_34 = clamp(roughness_value, 0.039999999105930328369140625f, 1.0f);
        float param_35 = metallic;
        ShadingResult _1005 = shade_ibl(param_31, param_32, param_33, param_34, param_35);
        color_linear = _1005.color;
        ambient_linear = _1005.ambient;
    }
    else
    {
        float _1016 = clamp(1.0f - roughness_value, 0.0f, 1.0f);
        float _1026 = max(dot(world_normal, float3(0.3520300388336181640625f, 0.824756085872650146484375f, 0.442552030086517333984375f)), 0.0f);
        float3 _1051 = (lerp(0.10999999940395355224609375f.xxx, 0.62999999523162841796875f.xxx, clamp((world_normal.y * 0.5f) + 0.5f, 0.0f, 1.0f).xxx) * 0.550000011920928955078125f) + 0.20000000298023223876953125f.xxx;
        color_linear = (base_color * (_1051 + (1.0f.xxx * (_1026 * 0.75f)))) + ((pow(max(dot(world_normal, normalize(float3(0.3520300388336181640625f, 0.824756085872650146484375f, 0.442552030086517333984375f) + normalize(sc_camera_position.xyz - v_world_position))), 0.0f), exp2(1.0f + (_1016 * 10.0f))) * _1016) * step(0.0f, _1026)).xxx;
        ambient_linear = base_color * _1051;
    }
    color_linear += (ambient_linear * (ao_1 - 1.0f));
    ambient_linear *= ao_1;
    float3 emissive = _473_mat_emissive.xyz;
    if (_585)
    {
        float3 factor_1 = _473_mat_emissive.xyz;
        if (all(bool3(_473_mat_emissive.xyz.x <= 0.0f.xxx.x, _473_mat_emissive.xyz.y <= 0.0f.xxx.y, _473_mat_emissive.xyz.z <= 0.0f.xxx.z)))
        {
            factor_1 = 1.0f.xxx;
        }
        if (_473_mat_channels1.y > 3.5f)
        {
            emissive = _461.xyz * factor_1;
        }
        else
        {
            float4 param_36 = _461;
            float param_37 = _473_mat_channels1.y;
            emissive = select_channel(param_36, param_37).xxx * factor_1;
        }
    }
    float3 _1146 = color_linear;
    float3 _1148 = _1146 + emissive;
    color_linear = _1148;
    frag_color = float4(_1148, out_alpha);
    frag_ambient = float4(ambient_linear, out_alpha);
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_uv = stage_input.v_uv;
    v_normal = stage_input.v_normal;
    v_color = stage_input.v_color;
    v_world_position = stage_input.v_world_position;
    v_tangent = stage_input.v_tangent;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_ambient = frag_ambient;
    stage_output.frag_color = frag_color;
    return stage_output;
}
