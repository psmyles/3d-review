// Bloom passes (CLAUDE.md render roadmap, Phase 4): a bright-pass extract from the
// scene's linear-HDR bloom source (MRT location 1 of the scene pass), then a
// separable Gaussian blur. All three entry points share one fullscreen-triangle
// vertex shader and one bind group (input texture + sampler + uniform). The
// composite/post pass adds the blurred result over the linear scene color before
// tone mapping.
//
// Bloom runs at half framebuffer resolution: the bright-pass downsamples the
// full-res source into the half-res target (the fullscreen draw + linear sampler
// does the 2× minification), and the blur ping-pongs between two half-res
// textures. Working in linear HDR is the whole point — the source carries the
// pre-tone-map radiance, so bright reflections / skybox above `threshold` glow.

struct BloomUniforms {
    // Bright-pass: x = threshold, y = soft knee. (Unused by the blur.)
    threshold: f32,
    knee: f32,
    // Blur: per-tap sampling offset in UV (texel size along the blur axis; zero
    // on the other axis). (Unused by the bright-pass.)
    offset: vec2<f32>,
};

@group(0) @binding(0)
var src_texture: texture_2d<f32>;
@group(0) @binding(1)
var src_sampler: sampler;
@group(0) @binding(2)
var<uniform> bloom: BloomUniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> VertexOutput {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];
    var out: VertexOutput;
    out.clip_position = vec4<f32>(corner, 0.0, 1.0);
    var uv = corner * 0.5 + vec2<f32>(0.5, 0.5);
    uv.y = 1.0 - uv.y;
    out.uv = uv;
    return out;
}

// Soft-knee bright-pass (Karis): pixels brighter than `threshold` pass through
// with a smooth ramp over a `knee`-wide band so the bloom onset isn't a hard
// edge. Returns the (linear) over-threshold radiance.
@fragment
fn fs_brightpass(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(src_texture, src_sampler, input.uv).rgb;
    let brightness = max(color.r, max(color.g, color.b));
    let knee = max(bloom.knee, 1e-4);
    let soft = clamp(brightness - bloom.threshold + knee, 0.0, 2.0 * knee);
    let soft_contrib = soft * soft / (4.0 * knee);
    let contribution = max(soft_contrib, brightness - bloom.threshold);
    let factor = max(contribution, 0.0) / max(brightness, 1e-4);
    return vec4<f32>(color * factor, 1.0);
}

// Separable 9-tap Gaussian using 5 linearly-interpolated samples (the standard
// weight/offset trick: each off-center tap samples between two texels so one
// bilinear fetch covers two of the nine kernel weights).
@fragment
fn fs_blur(input: VertexOutput) -> @location(0) vec4<f32> {
    let weights = array<f32, 3>(0.227027, 0.3162162, 0.0702703);
    let offsets = array<f32, 3>(0.0, 1.3846154, 3.2307693);

    var result = textureSample(src_texture, src_sampler, input.uv).rgb * weights[0];
    for (var i = 1; i < 3; i = i + 1) {
        let off = bloom.offset * offsets[i];
        result = result + textureSample(src_texture, src_sampler, input.uv + off).rgb * weights[i];
        result = result + textureSample(src_texture, src_sampler, input.uv - off).rgb * weights[i];
    }
    return vec4<f32>(result, 1.0);
}
