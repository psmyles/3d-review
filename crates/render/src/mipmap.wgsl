// Mipmap downsample blit: produces one mip level by bilinearly sampling the level
// above. Paired with mipmap.rs (one pipeline per slot texture format). The source
// view exposes a single mip level, so a plain bilinear sample averages the 2x2
// footprint; for sRGB targets the sample decodes to linear and the render target
// re-encodes on write, so color averages in linear space.

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle from the vertex index (no vertex buffer).
@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    // Flip Y so the sampled UV matches the rendered texel orientation.
    out.uv.y = 1.0 - out.uv.y;
    return out;
}

@group(0) @binding(0) var src_texture: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_texture, src_sampler, in.uv);
}
