cbuffer material : register(b2)
{
    float4 _83_mat_base_color : packoffset(c0);
    float4 _83_mat_emissive : packoffset(c1);
    float4 _83_mat_params : packoffset(c2);
    float4 _83_mat_channels0 : packoffset(c3);
    float4 _83_mat_channels1 : packoffset(c4);
    float4 _83_mat_flags : packoffset(c5);
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

Texture2D<float4> opacity_tex : register(t0);
SamplerState material_sampler : register(s2);

static float4 v_color;
static float3 v_normal;
static float2 v_uv;
static float4 frag_gbuffer;
static float4 frag_depth;
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
    float4 frag_gbuffer : SV_Target0;
    float4 frag_depth : SV_Target1;
};

float2 material_uv(float2 uv)
{
    return float2(uv.x, 1.0f - uv.y);
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
    float alpha = _83_mat_base_color.w;
    if ((uint(_83_mat_params.z) & 64u) != 0u)
    {
        float4 param = tex_opacity;
        float param_1 = _83_mat_channels1.z;
        alpha *= select_channel(param, param_1);
    }
    return alpha;
}

float vertex_color_alpha(float alpha)
{
    float result = alpha;
    if (sc_render_options.w >= 0.0f)
    {
        float _126;
        if (sc_render_options.w < 1.5f)
        {
            _126 = 1.0f;
        }
        else
        {
            _126 = v_color.w;
        }
        result = _126;
    }
    return result;
}

bool alpha_clipped(float alpha)
{
    bool _141 = _83_mat_params.w > 1.5f;
    bool _148;
    if (_141)
    {
        _148 = alpha < _83_mat_channels1.w;
    }
    else
    {
        _148 = _141;
    }
    return _148;
}

bool surface_cut_out(float4 tex_opacity)
{
    bool _160 = dot(v_normal, v_normal) >= 9.9999999747524270787835121154785e-07f;
    bool _167;
    if (_160)
    {
        _167 = sc_projection_params.z < 0.0f;
    }
    else
    {
        _167 = _160;
    }
    bool _173;
    if (_167)
    {
        _173 = sc_projection_params.w <= 0.5f;
    }
    else
    {
        _173 = _167;
    }
    bool _184;
    if (_173)
    {
        float4 param = tex_opacity;
        float param_1 = material_alpha(param);
        float param_2 = vertex_color_alpha(param_1);
        _184 = alpha_clipped(param_2);
    }
    else
    {
        _184 = _173;
    }
    return _184;
}

void frag_main()
{
    float2 param = v_uv;
    float4 param_1 = opacity_tex.Sample(material_sampler, material_uv(param));
    if (surface_cut_out(param_1))
    {
        discard;
    }
    if (dot(v_normal, v_normal) < 9.9999999747524270787835121154785e-07f)
    {
        frag_gbuffer = 0.0f.xxxx;
        frag_depth = 1000000015047466219876688855040.0f.xxxx;
        return;
    }
    float4 _236 = mul(float4(v_world_position, 1.0f), sc_view);
    float _251 = _236.z;
    frag_gbuffer = float4(normalize(mul(float4(v_normal, 0.0f), sc_view).xyz), _251);
    frag_depth = (-_251).xxxx;
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    v_color = stage_input.v_color;
    v_normal = stage_input.v_normal;
    v_uv = stage_input.v_uv;
    v_world_position = stage_input.v_world_position;
    v_tangent = stage_input.v_tangent;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_gbuffer = frag_gbuffer;
    stage_output.frag_depth = frag_depth;
    return stage_output;
}
