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

bool is_background_depth(float depth)
{
    return !((depth > 0.0f) && (depth < 1000000015047466219876688855040.0f));
}

void frag_main()
{
    float4 _281 = gbuffer.SampleLevel(gtao_sampler, v_uv, 0.0f);
    float3 _284 = _281.xyz;
    float _287 = _281.w;
    if ((dot(_284, _284) < 0.25f) || (_287 >= (-9.9999997473787516355514526367188e-05f)))
    {
        frag_ao = 1.0f;
        return;
    }
    float3 _303 = normalize(_284);
    float2 param = v_uv;
    float param_1 = _287;
    float3 _309 = reconstruct_view_pos(param, param_1);
    float3 _313 = normalize(-_309);
    float _318 = max(_57_params.x, 9.9999997473787516355514526367188e-05f);
    float _325 = clamp(_57_params.z, 0.0f, 1.0f);
    uint _330 = max(uint(_57_config.y), 1u);
    uint _335 = max(uint(_57_config.z), 1u);
    float2 _337 = target_dims();
    float2 _341 = 1.0f.xx / _337;
    float3 param_2 = _309 + float3(_318, 0.0f, 0.0f);
    float _359 = abs(project_to_uv(param_2).x - v_uv.x) * _337.x;
    if (_359 < 1.2999999523162841796875f)
    {
        frag_ao = 1.0f;
        return;
    }
    float _368 = 1.2999999523162841796875f / _359;
    float _380 = (-1.62601625919342041015625f) / _318;
    float _385 = ((_318 * 0.3849999904632568359375f) / (0.6150000095367431640625f * _318)) + 1.0f;
    uint2 param_3 = uint2(uint(gl_FragCoord.x), uint(gl_FragCoord.y));
    float2 _397 = spatial_dither(param_3);
    float _404 = frac(_397.x + _57_temporal.x);
    float _411 = frac(_397.y + _57_temporal.y);
    float visibility = clamp((10.0f - _359) * 0.00999999977648258209228515625f, 0.0f, 1.0f) * 0.5f;
    float _677;
    float _694;
    for (uint s = 0u; s < _330; s++)
    {
        float _438 = ((float(s) + _404) * 3.1415927410125732421875f) / float(_330);
        float _441 = cos(_438);
        float _444 = sin(_438);
        float2 _449 = float2(_441, -_444);
        float3 _453 = float3(_441, _444, 0.0f);
        float3 _461 = _453 - (_313 * dot(_453, _313));
        float3 _466 = normalize(cross(_461, _313));
        float3 _474 = _303 - (_466 * dot(_303, _466));
        float _477 = length(_474);
        float proj_len = _477;
        if (_477 < 9.9999999747524270787835121154785e-07f)
        {
            continue;
        }
        float _493 = proj_len;
        float _495 = clamp(dot(_474, _313) / _493, 0.0f, 1.0f);
        float _500 = sign(dot(_474, _461)) * acos(_495);
        float _505 = cos(_500 + 1.57079637050628662109375f);
        float _509 = cos(_500 - 1.57079637050628662109375f);
        float horizon_cos0 = _505;
        float horizon_cos1 = _509;
        for (uint t = 0u; t < _335; t++)
        {
            float _542 = (float(t) + frac(_411 + (float(s + (t * _330)) * 0.61803400516510009765625f))) / float(_335);
            float2 _553 = (_449 * ((_542 * _542) + _368)) * _359;
            int _566 = int(clamp(log2(max(length(_553), 1.0f)) - 3.2999999523162841796875f, 0.0f, 4.0f) + 0.5f);
            float2 _571 = round(_553) * _341;
            float2 _575 = v_uv + _571;
            float2 _579 = v_uv - _571;
            float2 param_4 = _575;
            int param_5 = _566;
            float _585 = sample_depth_mip(param_4, param_5);
            float2 param_6 = _579;
            int param_7 = _566;
            float _591 = sample_depth_mip(param_6, param_7);
            float shc0 = _505;
            float param_8 = _585;
            if (!is_background_depth(param_8))
            {
                float2 param_9 = _575;
                float param_10 = -_585;
                float3 _608 = reconstruct_view_pos(param_9, param_10) - _309;
                float _611 = length(_608);
                if (_611 > 9.9999999747524270787835121154785e-07f)
                {
                    shc0 = lerp(_505, dot(_608, _313) / _611, clamp((_611 * _380) + _385, 0.0f, 1.0f));
                }
            }
            float shc1 = _509;
            float param_11 = _591;
            if (!is_background_depth(param_11))
            {
                float2 param_12 = _579;
                float param_13 = -_591;
                float3 _647 = reconstruct_view_pos(param_12, param_13) - _309;
                float _650 = length(_647);
                if (_650 > 9.9999999747524270787835121154785e-07f)
                {
                    shc1 = lerp(_509, dot(_647, _313) / _650, clamp((_650 * _380) + _385, 0.0f, 1.0f));
                }
            }
            float _673 = max(horizon_cos0, shc0);
            if (horizon_cos0 > shc0)
            {
                _677 = lerp(_673, shc0, _325);
            }
            else
            {
                _677 = _673;
            }
            horizon_cos0 = _677;
            float _690 = max(horizon_cos1, shc1);
            if (horizon_cos1 > shc1)
            {
                _694 = lerp(_690, shc1, _325);
            }
            else
            {
                _694 = _690;
            }
            horizon_cos1 = _694;
        }
        float _706 = proj_len;
        float _708 = lerp(_706, 1.0f, 0.0500000007450580596923828125f);
        proj_len = _708;
        float _720 = sin(_500);
        float _724 = acos(clamp(horizon_cos1, -1.0f, 1.0f)) * (-2.0f);
        float _738 = 2.0f * acos(clamp(horizon_cos0, -1.0f, 1.0f));
        visibility += (_708 * (0.25f * (((_495 + (_724 * _720)) - cos(_724 - _500)) + ((_495 + (_738 * _720)) - cos(_738 - _500)))));
    }
    float _758 = visibility;
    float _762 = clamp(_758 / float(_330), 0.0f, 1.0f);
    visibility = _762;
    float _768 = pow(max(_762, 0.0f), max(_57_params.y, 0.0f));
    if (_57_temporal.z >= 1.0f)
    {
        frag_ao = _768;
    }
    else
    {
        frag_ao = lerp(ao_history.SampleLevel(gtao_sampler, v_uv, 0.0f).x, _768, _57_temporal.z);
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
