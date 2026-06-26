// Tex viewport shader (HLSL / Shader Model 5.0) — the Direct3D 11 port of
// `tex.wgsl`. Two pixel paths over a shared fullscreen-triangle vertex shader:
// `fs_image` draws one pooled texture (channel-isolated, pan/zoom via the uniform)
// and `fs_checker` paints the transparency checkerboard background. Deliberately
// outside the scene MRT / tone-map path so the displayed texel equals the stored
// texel (a faithful image viewer); the source texture is uploaded `Rgba8Unorm` so
// the sample returns the raw stored bytes for every channel alike.
//
// The backbuffer is `R8G8B8A8_UNORM` (gamma, non-sRGB-aware — see `rhi`), so the
// bytes are written verbatim (`target_srgb` is 0); the `srgb_to_linear` branch is
// the dormant compensation tex.wgsl carries for an sRGB framebuffer.

cbuffer TexUniforms : register(b0)
{
    // Image rectangle in framebuffer pixels: top-left corner + size. A fragment's
    // image UV is (frag_pixel - img_min) / img_size. (`fs_image`)
    float2 img_min;
    float2 img_size;
    // 0 = RGB (alpha composites over the background), 1..4 = R/G/B/A isolated as
    // opaque greyscale. (`fs_image`)
    uint   channel;
    // 1 when the framebuffer encodes linear->sRGB on write (0 for our UNORM
    // backbuffer). (`fs_image`)
    uint   target_srgb;
    // Checker cell size in framebuffer pixels. (`fs_checker`)
    float  checker_cell;
    uint   pad;
    // Checker colors (gamma-space, written verbatim). (`fs_checker`)
    float4 bg_light;
    float4 bg_dark;
};

Texture2D    tex  : register(t0);
SamplerState samp : register(s0);

struct VsOut
{
    float4 pos : SV_Position;
};

// Oversized fullscreen triangle covering the whole backbuffer in one primitive.
VsOut vs_fullscreen(uint vertex_id : SV_VertexID)
{
    float2 corners[3] = {
        float2(-1.0, -1.0),
        float2( 3.0, -1.0),
        float2(-1.0,  3.0),
    };
    VsOut output;
    output.pos = float4(corners[vertex_id], 0.0, 1.0);
    return output;
}

// Per-channel sRGB decode (gamma -> linear). Pre-compensates an sRGB framebuffer;
// `max(.., 0)` guards fxc's negative-base `pow` warning (/WX) — the base is >= 0
// for any valid color, so it changes nothing for real inputs.
float3 srgb_to_linear(float3 c)
{
    float3 lo = c / 12.92;
    float3 hi = pow(max((c + 0.055) / 1.055, 0.0), 2.4);
    float3 cutoff = step(c, 0.04045);
    return lerp(hi, lo, cutoff);
}

float4 fs_image(VsOut input) : SV_Target
{
    float2 uv = (input.pos.xy - img_min) / img_size;
    // Sample unconditionally (uniform control flow) so the implicit-derivative LOD
    // is valid, then discard fragments outside the image so the background shows.
    float4 raw = tex.Sample(samp, uv);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0)
    {
        discard;
    }

    float3 rgb;
    float alpha = 1.0;
    switch (channel)
    {
        case 0u:
            rgb = raw.rgb;
            alpha = raw.a;
            break;
        case 1u:
            rgb = (float3)raw.r;
            break;
        case 2u:
            rgb = (float3)raw.g;
            break;
        case 3u:
            rgb = (float3)raw.b;
            break;
        default:
            rgb = (float3)raw.a;
            break;
    }

    // `rgb` holds the stored source bytes (UNORM sample, no decode). On an sRGB
    // framebuffer the GPU would encode linear->sRGB on write, so pre-apply the
    // inverse; our UNORM backbuffer stores verbatim (`target_srgb` is 0).
    float3 out_rgb = (target_srgb == 1u) ? srgb_to_linear(rgb) : rgb;
    return float4(out_rgb, alpha);
}

float4 fs_checker(VsOut input) : SV_Target
{
    float cell = max(checker_cell, 1.0);
    float2 c = floor(input.pos.xy / cell);
    float parity = fmod(c.x + c.y, 2.0);
    return (parity < 0.5) ? bg_light : bg_dark;
}
