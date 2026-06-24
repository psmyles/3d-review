// The Tex viewport shader: a fullscreen-triangle pass that paints one pooled
// texture (channel-isolated) into egui's framebuffer, with pan/zoom carried in
// the uniform. It is deliberately separate from `scene.wgsl` — it never goes
// through the linear-HDR / tone-map / MRT scene pipeline, so the displayed texel
// equals the stored texel (a faithful image viewer).
//
// Faithfulness across the sRGB seam: the texture is uploaded as `Rgba8Unorm`, so
// `textureSample` returns the raw stored bytes (no sRGB decode) for every channel
// alike. `target_srgb` then compensates for the framebuffer's encode-on-write so
// the on-screen byte equals the source byte regardless of the surface format.

struct TexUniforms {
    // Image rectangle in framebuffer pixels: top-left corner + size. A fragment's
    // image UV is (frag_pixel - img_min) / img_size.
    img_min: vec2<f32>,
    img_size: vec2<f32>,
    // 0 = RGB (alpha composites over the egui background fill), 1..4 = R/G/B/A
    // isolated and shown as opaque greyscale.
    channel: u32,
    // 1 when the framebuffer format is sRGB (the GPU encodes linear->sRGB on
    // write), 0 when it stores bytes verbatim. Selects the pre-compensation below.
    target_srgb: u32,
    pad: vec2<u32>,
};

@group(0) @binding(0) var<uniform> u: TexUniforms;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

// Oversized fullscreen triangle covering the whole viewport in one primitive.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(corners[vertex_index], 0.0, 1.0);
    return out;
}

// Per-channel sRGB decode (gamma -> linear). Pre-compensates an sRGB framebuffer:
// the value we return is encoded linear->sRGB on write, so feeding it the decoded
// byte lands the original byte back in the framebuffer.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let cutoff = step(c, vec3<f32>(0.04045));
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return mix(high, low, cutoff);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (frag.xy - u.img_min) / u.img_size;
    // Sample unconditionally (uniform control flow) so the implicit-derivative LOD
    // is valid, then discard fragments outside the image so the background shows.
    let raw = textureSample(tex, samp, uv);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        discard;
    }

    var rgb: vec3<f32>;
    var alpha: f32 = 1.0;
    switch (u.channel) {
        case 0u: {
            rgb = raw.rgb;
            alpha = raw.a;
        }
        case 1u: {
            rgb = vec3<f32>(raw.r);
        }
        case 2u: {
            rgb = vec3<f32>(raw.g);
        }
        case 3u: {
            rgb = vec3<f32>(raw.b);
        }
        default: {
            rgb = vec3<f32>(raw.a);
        }
    }

    // `rgb` holds the stored source bytes (the texture is UNORM, so no decode
    // happened on sample). If the framebuffer is sRGB, the GPU will encode
    // linear->sRGB on write, so pre-apply the inverse to land the same byte;
    // otherwise the bytes are stored verbatim.
    var out_rgb = rgb;
    if (u.target_srgb == 1u) {
        out_rgb = srgb_to_linear(rgb);
    }
    return vec4<f32>(out_rgb, alpha);
}
