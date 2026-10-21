cbuffer gtao_params : register(b0)
{
    row_major float4x4 _57_proj : packoffset(c0);
    float4 _57_params : packoffset(c4);
    float4 _57_config : packoffset(c5);
    float4 _57_temporal : packoffset(c6);
};

Texture2D<float4> depth_mip0 : register(t1);
SamplerState gtao_sampler : register(s0);
Texture2D<float4> depth_mip1 : register(t2);
Texture2D<float4> depth_mip2 : register(t3);
Texture2D<float4> depth_mip3 : register(t4);
Texture2D<float4> depth_mip4 : register(t5);
Texture2D<float4> gbuffer : register(t0);
Texture2D<float4> ao_history : register(t6);

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

bool is_background_depth(float depth)
{
    return !((depth > 0.0f) && (depth < 1000000015047466219876688855040.0f));
}

float3 reconstruct_view_pos(float2 uv, float view_z)
{
    float _78 = (uv.x * 2.0f) - 1.0f;
    float _83 = 1.0f - (uv.y * 2.0f);
    if (_57_config.x > 0.5f)
    {
        return float3((_78 - _57_proj[3].x) / _57_proj[0].x, (_83 - _57_proj[3].y) / _57_proj[1].y, view_z);
    }
    return float3((_78 * (-view_z)) / _57_proj[0].x, (_83 * (-view_z)) / _57_proj[1].y, view_z);
}

float2 target_dims()
{
    return float2(max(_57_params.w, 1.0f), max(_57_config.w, 1.0f));
}

float2 project_to_uv(float3 view_pos)
{
    float4 _150 = mul(float4(view_pos, 1.0f), _57_proj);
    float2 _153 = _150.xy;
    float2 ndc = _153;
    if (_57_config.x <= 0.5f)
    {
        ndc = _153 / _150.w.xx;
    }
    return float2((ndc.x * 0.5f) + 0.5f, 0.5f - (ndc.y * 0.5f));
}

float2 spatial_dither(uint2 pix)
{
    return float2(0.0625f * float((((pix.x + pix.y) & 3u) << 2u) + (pix.x & 3u)), 0.25f * float((pix.y - pix.x) & 3u));
}

float sample_depth_mip(float2 uv, int level)
{
    float depth = 0.0f;
    if (level <= 0)
    {
        depth = depth_mip0.SampleLevel(gtao_sampler, uv, 0.0f).x;
    }
    else
    {
        if (level == 1)
        {
            depth = depth_mip1.SampleLevel(gtao_sampler, uv, 0.0f).x;
        }
        else
        {
            if (level == 2)
            {
                depth = depth_mip2.SampleLevel(gtao_sampler, uv, 0.0f).x;
            }
            else
            {
                if (level == 3)
                {
                    depth = depth_mip3.SampleLevel(gtao_sampler, uv, 0.0f).x;
                }
                else
                {
                    depth = depth_mip4.SampleLevel(gtao_sampler, uv, 0.0f).x;
                }
            }
        }
    }
    return depth;
}

void frag_main()
{
    float3 _284 = gbuffer.SampleLevel(gtao_sampler, v_uv, 0.0f).xyz;
    float4 _290 = depth_mip0.SampleLevel(gtao_sampler, v_uv, 0.0f);
    float _291 = _290.x;
    bool _295 = dot(_284, _284) < 0.25f;
    bool _302;
    if (!_295)
    {
        float param = _291;
        _302 = is_background_depth(param);
    }
    else
    {
        _302 = _295;
    }
    if (_302 || (_291 <= 9.9999997473787516355514526367188e-05f))
    {
        frag_ao = 1.0f;
        return;
    }
    float3 _317 = normalize(_284);
    float2 param_1 = v_uv;
    float param_2 = -_291;
    float3 _323 = reconstruct_view_pos(param_1, param_2);
    float3 _327 = normalize(-_323);
    float _331 = max(_57_params.x, 9.9999997473787516355514526367188e-05f);
    float _338 = clamp(_57_params.z, 0.0f, 1.0f);
    uint _343 = max(uint(_57_config.y), 1u);
    uint _348 = max(uint(_57_config.z), 1u);
    float2 _350 = target_dims();
    float2 _354 = 1.0f.xx / _350;
    float3 param_3 = _323 + float3(_331, 0.0f, 0.0f);
    float _372 = abs(project_to_uv(param_3).x - v_uv.x) * _350.x;
    if (_372 < 1.2999999523162841796875f)
    {
        frag_ao = 1.0f;
        return;
    }
    float _381 = 1.2999999523162841796875f / _372;
    float _393 = (-1.62601625919342041015625f) / _331;
    float _398 = ((_331 * 0.3849999904632568359375f) / (0.6150000095367431640625f * _331)) + 1.0f;
    uint2 param_4 = uint2(uint(gl_FragCoord.x), uint(gl_FragCoord.y));
    float2 _410 = spatial_dither(param_4);
    float _417 = frac(_410.x + _57_temporal.x);
    float _424 = frac(_410.y + _57_temporal.y);
    float visibility = clamp((10.0f - _372) * 0.00999999977648258209228515625f, 0.0f, 1.0f) * 0.5f;
    float _704;
    float _721;
    for (uint s = 0u; s < _343; s++)
    {
        float _451 = ((float(s) + _417) * 3.1415927410125732421875f) / float(_343);
        float _454 = cos(_451);
        float _457 = sin(_451);
        float2 _462 = float2(_454, -_457);
        float3 _466 = float3(_454, _457, 0.0f);
        float3 _474 = _466 - (_327 * dot(_466, _327));
        float3 _478 = cross(_474, _327);
        float _482 = dot(_478, _478);
        if (_482 < 9.9999999600419720025001879548654e-13f)
        {
            continue;
        }
        float3 _493 = _478 * rsqrt(_482);
        float3 _501 = _317 - (_493 * dot(_317, _493));
        float _504 = length(_501);
        float proj_len = _504;
        if (_504 < 9.9999999747524270787835121154785e-07f)
        {
            continue;
        }
        float _520 = proj_len;
        float _522 = clamp(dot(_501, _327) / _520, 0.0f, 1.0f);
        float _527 = sign(dot(_501, _474)) * acos(_522);
        float _532 = cos(_527 + 1.57079637050628662109375f);
        float _536 = cos(_527 - 1.57079637050628662109375f);
        float horizon_cos0 = _532;
        float horizon_cos1 = _536;
        for (uint t = 0u; t < _348; t++)
        {
            float _569 = (float(t) + frac(_424 + (float(s + (t * _343)) * 0.61803400516510009765625f))) / float(_348);
            float2 _580 = (_462 * ((_569 * _569) + _381)) * _372;
            int _593 = int(clamp(log2(max(length(_580), 1.0f)) - 3.2999999523162841796875f, 0.0f, 4.0f) + 0.5f);
            float2 _598 = round(_580) * _354;
            float2 _602 = v_uv + _598;
            float2 _606 = v_uv - _598;
            float2 param_5 = _602;
            int param_6 = _593;
            float _612 = sample_depth_mip(param_5, param_6);
            float2 param_7 = _606;
            int param_8 = _593;
            float _618 = sample_depth_mip(param_7, param_8);
            float shc0 = _532;
            float param_9 = _612;
            if (!is_background_depth(param_9))
            {
                float2 param_10 = _602;
                float param_11 = -_612;
                float3 _635 = reconstruct_view_pos(param_10, param_11) - _323;
                float _638 = length(_635);
                if (_638 > 9.9999999747524270787835121154785e-07f)
                {
                    shc0 = lerp(_532, dot(_635, _327) / _638, clamp((_638 * _393) + _398, 0.0f, 1.0f));
                }
            }
            float shc1 = _536;
            float param_12 = _618;
            if (!is_background_depth(param_12))
            {
                float2 param_13 = _606;
                float param_14 = -_618;
                float3 _674 = reconstruct_view_pos(param_13, param_14) - _323;
                float _677 = length(_674);
                if (_677 > 9.9999999747524270787835121154785e-07f)
                {
                    shc1 = lerp(_536, dot(_674, _327) / _677, clamp((_677 * _393) + _398, 0.0f, 1.0f));
                }
            }
            float _700 = max(horizon_cos0, shc0);
            if (horizon_cos0 > shc0)
            {
                _704 = lerp(_700, shc0, _338);
            }
            else
            {
                _704 = _700;
            }
            horizon_cos0 = _704;
            float _717 = max(horizon_cos1, shc1);
            if (horizon_cos1 > shc1)
            {
                _721 = lerp(_717, shc1, _338);
            }
            else
            {
                _721 = _717;
            }
            horizon_cos1 = _721;
        }
        float _733 = proj_len;
        float _735 = lerp(_733, 1.0f, 0.0500000007450580596923828125f);
        proj_len = _735;
        float _747 = sin(_527);
        float _751 = acos(clamp(horizon_cos1, -1.0f, 1.0f)) * (-2.0f);
        float _765 = 2.0f * acos(clamp(horizon_cos0, -1.0f, 1.0f));
        visibility += (_735 * (0.25f * (((_522 + (_751 * _747)) - cos(_751 - _527)) + ((_522 + (_765 * _747)) - cos(_765 - _527)))));
    }
    float _785 = visibility;
    float _789 = clamp(_785 / float(_343), 0.0f, 1.0f);
    visibility = _789;
    float _795 = pow(max(_789, 9.9999999747524270787835121154785e-07f), max(_57_params.y, 0.0f));
    if (_57_temporal.z >= 1.0f)
    {
        frag_ao = _795;
    }
    else
    {
        float4 _809 = ao_history.SampleLevel(gtao_sampler, v_uv, 0.0f);
        float _810 = _809.x;
        float _819;
        if ((_810 >= 0.0f) && (_810 <= 1.0f))
        {
            _819 = lerp(_810, _795, _57_temporal.z);
        }
        else
        {
            _819 = _795;
        }
        frag_ao = _819;
    }
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
