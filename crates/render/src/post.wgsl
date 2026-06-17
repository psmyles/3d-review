// Post / composite pass: samples the resolved offscreen scene color and writes it
// into egui's framebuffer, behind the chrome (CLAUDE.md render roadmap, Phase 1).
//
// Phase 1 is a straight passthrough. The scene shader still owns lighting, tone
// mapping and the sRGB encode, so the offscreen target already holds final
// display-space color and this pass only blits it — keeping the composited image
// byte-for-byte what egui produced when the scene drew directly into its pass.
// This is the seam where tone mapping / bloom / FXAA move once the scene renders
// in linear HDR (see `targets.rs` SCENE_HDR_FORMAT).

@group(0) @binding(0)
var scene_color: texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Single oversized triangle covering the whole framebuffer (the standard
    // fullscreen-triangle trick — no vertex buffer needed).
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];
    var out: VertexOutput;
    out.clip_position = vec4<f32>(corner, 0.0, 1.0);
    // Map clip space to texture UV, flipping Y (texture origin is top-left).
    var uv = corner * 0.5 + vec2<f32>(0.5, 0.5);
    uv.y = 1.0 - uv.y;
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(scene_color, scene_sampler, input.uv).rgb, 1.0);
}
