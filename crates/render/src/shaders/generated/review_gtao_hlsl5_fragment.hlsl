cbuffer gtao_params : register(b0)
{
    row_major float4x4 _53_proj : packoffset(c0);
    float4 _53_params : packoffset(c4);
    float4 _53_config : packoffset(c5);
};

Texture2D<float4> gbuffer : register(t0);
SamplerState gtao_sampler : register(s0);

static float4 gl_FragCoord;
static float2 v_uv;
static float frag_ao;

struct SPIRV_Cross_Input
{
    float2 v_uv : TEXCOORD0;
    float4 gl_FragCoord : SV_Position;
};

struct SPIRV_Cross_Output
{
    float frag_ao : SV_Target0;
};

float3 reconstruct_view_pos(float2 uv, float view_z)
{
    float _75 = (uv.x * 2.0f) - 1.0f;
    float _80 = 1.0f - (uv.y * 2.0f);
    if (_53_config.x > 0.5f)
    {
        return float3((_75 - _53_proj[3].x) / _53_proj[0].x, (_80 - _53_proj[3].y) / _53_proj[1].y, view_z);
    }
    return float3((_75 * (-view_z)) / _53_proj[0].x, (_80 * (-view_z)) / _53_proj[1].y, view_z);
}

float2 target_dims()
{
    return float2(max(_53_params.w, 1.0f), max(_53_config.w, 1.0f));
}

float2 project_to_uv(float3 view_pos)
{
    float4 _148 = mul(float4(view_pos, 1.0f), _53_proj);
    float2 _151 = _148.xy;
    float2 ndc = _151;
    if (_53_config.x <= 0.5f)
    {
        ndc = _151 / _148.w.xx;
    }
    return float2((ndc.x * 0.5f) + 0.5f, 0.5f - (ndc.y * 0.5f));
}

float2 spatial_dither(uint2 pix)
{
    return float2(0.0625f * float((((pix.x + pix.y) & 3u) << 2u) + (pix.x & 3u)), 0.25f * float((pix.y - pix.x) & 3u));
}

float4 sample_view_pos(float2 uv)
{
    float4 _187 = gbuffer.SampleLevel(gtao_sampler, uv, 0.0f);
    float3 _189 = _187.xyz;
    bool _194 = dot(_189, _189) < 0.25f;
    bool _202;
    if (!_194)
    {
        _202 = _187.w >= (-9.9999997473787516355514526367188e-05f);
    }
    else
    {
        _202 = _194;
    }
    if (_202)
    {
        return 0.0f.xxxx;
    }
    float2 param = uv;
    float param_1 = _187.w;
    return float4(reconstruct_view_pos(param, param_1), 1.0f);
}

float horizon_cos(float2 origin_uv, float3 p, float3 v, float2 dir_px, float radius_px, float radius, float thickness, uint steps, float jitter, float2 inv_dims)
{
    float cos_h = -1.0f;
    for (uint t = 1u; t <= steps; t++)
    {
        float2 _277 = origin_uv + ((dir_px * (radius_px * ((float(t) - jitter) / float(steps)))) * inv_dims);
        float _279 = _277.x;
        bool _280 = _279 < 0.0f;
        bool _287;
        if (!_280)
        {
            _287 = _279 > 1.0f;
        }
        else
        {
            _287 = _280;
        }
        bool _294;
        if (!_287)
        {
            _294 = _277.y < 0.0f;
        }
        else
        {
            _294 = _287;
        }
        bool _301;
        if (!_294)
        {
            _301 = _277.y > 1.0f;
        }
        else
        {
            _301 = _294;
        }
        if (_301)
        {
            break;
        }
        float2 param = _277;
        float4 _308 = sample_view_pos(param);
        if (_308.w < 0.5f)
        {
            continue;
        }
        float3 _319 = _308.xyz - p;
        float _322 = length(_319);
        if (_322 < 9.9999997473787516355514526367188e-06f)
        {
            continue;
        }
        float _340 = clamp(1.0f - (_322 / radius), 0.0f, 1.0f);
        if (_340 <= 0.0f)
        {
            continue;
        }
        cos_h = max(cos_h, (dot(_319, v) / _322) - ((1.0f - _340) * thickness));
    }
    return cos_h;
}

void frag_main()
{
    float4 _366 = gbuffer.SampleLevel(gtao_sampler, v_uv, 0.0f);
    float3 _369 = _366.xyz;
    float _372 = _366.w;
    if ((dot(_369, _369) < 0.25f) || (_372 >= (-9.9999997473787516355514526367188e-05f)))
    {
        frag_ao = 1.0f;
        return;
    }
    float3 _387 = normalize(_369);
    float2 param = v_uv;
    float param_1 = _372;
    float3 _393 = reconstruct_view_pos(param, param_1);
    float3 _397 = normalize(-_393);
    float _407 = clamp(_53_params.z, 0.0f, 1.0f);
    uint _412 = max(uint(_53_config.y), 1u);
    uint _417 = max(uint(_53_config.z), 1u);
    float2 _419 = target_dims();
    float2 _423 = 1.0f.xx / _419;
    float3 param_2 = _393 + float3(_53_params.x, 0.0f, 0.0f);
    float _440 = _419.x;
    float _447 = clamp(abs(project_to_uv(param_2).x - v_uv.x) * _440, 1.0f, max(_440, _419.y));
    uint2 param_3 = uint2(uint(gl_FragCoord.x), uint(gl_FragCoord.y));
    float2 _459 = spatial_dither(param_3);
    float _462 = _459.x;
    float _465 = _459.y;
    float visibility = 0.0f;
    for (uint s = 0u; s < _412; s++)
    {
        float _485 = ((float(s) + _462) * 3.1415927410125732421875f) / float(_412);
        float _488 = cos(_485);
        float _490 = sin(_485);
        float2 _491 = float2(_488, _490);
        float2 param_4 = v_uv + (_491 * (_423 * 2.0f));
        float4 _500 = sample_view_pos(param_4);
        float3 slice_dir = normalize(float3(_488, _490, 0.0f));
        if (_500.w > 0.5f)
        {
            float3 _517 = _500.xyz - _393;
            if (dot(_517, _517) > 9.9999999600419720025001879548654e-13f)
            {
                slice_dir = normalize(_517);
            }
        }
        float3 _531 = normalize(cross(_397, slice_dir));
        float3 _539 = _387 - (_531 * dot(_387, _531));
        float _542 = length(_539);
        if (_542 < 9.9999997473787516355514526367188e-05f)
        {
            continue;
        }
        float3 _553 = _539 / _542.xxx;
        float _568 = sign(dot(cross(slice_dir, _553), _531)) * acos(clamp(dot(_553, _397), -1.0f, 1.0f));
        float2 param_5 = v_uv;
        float3 param_6 = _393;
        float3 param_7 = _397;
        float2 param_8 = _491;
        float param_9 = _447;
        float param_10 = _53_params.x;
        float param_11 = _407;
        uint param_12 = _417;
        float param_13 = _465;
        float2 param_14 = _423;
        float2 param_15 = v_uv;
        float3 param_16 = _393;
        float3 param_17 = _397;
        float2 param_18 = -_491;
        float param_19 = _447;
        float param_20 = _53_params.x;
        float param_21 = _407;
        uint param_22 = _417;
        float param_23 = _465;
        float2 param_24 = _423;
        float _637 = cos(_568);
        float _640 = sin(_568);
        float _643 = 2.0f * (_568 + max((-acos(clamp(horizon_cos(param_15, param_16, param_17, param_18, param_19, param_20, param_21, param_22, param_23, param_24), -1.0f, 1.0f))) - _568, -1.57079637050628662109375f));
        float _656 = 2.0f * (_568 + min(acos(clamp(horizon_cos(param_5, param_6, param_7, param_8, param_9, param_10, param_11, param_12, param_13, param_14), -1.0f, 1.0f)) - _568, 1.57079637050628662109375f));
        visibility += (_542 * (0.25f * ((((-cos(_643 - _568)) + _637) + (_643 * _640)) + (((-cos(_656 - _568)) + _637) + (_656 * _640)))));
    }
    float _677 = visibility;
    float _681 = clamp(_677 / float(_412), 0.0f, 1.0f);
    visibility = _681;
    frag_ao = pow(max(_681, 0.0f), max(_53_params.y, 0.0f));
}

SPIRV_Cross_Output main(SPIRV_Cross_Input stage_input)
{
    gl_FragCoord = stage_input.gl_FragCoord;
    gl_FragCoord.w = 1.0 / gl_FragCoord.w;
    v_uv = stage_input.v_uv;
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_ao = frag_ao;
    return stage_output;
}
