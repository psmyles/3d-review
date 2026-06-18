// Final model-wireframe overlay. Runs after post processing into egui's
// framebuffer, so colors are display-space UI colors and never enter bloom/SSAO.

struct WireframeUniforms {
    projection: mat4x4<f32>,
    view: mat4x4<f32>,
    // x = 1 / framebuffer width, y = 1 / framebuffer height,
    // z = active thickness, w = view-Z occlusion bias.
    params: vec4<f32>,
    // x = use world units, y = occlusion enabled, z/w unused.
    flags: vec4<u32>,
};

@group(0) @binding(0)
var visibility_gbuffer: texture_2d<f32>;
@group(0) @binding(1)
var<uniform> uniforms: WireframeUniforms;

struct VertexInput {
    @location(0) start: vec4<f32>,
    @location(1) end: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) view_z: f32,
};

fn safe_normalize_2d(v: vec2<f32>) -> vec2<f32> {
    let len = length(v);
    if (len > 1e-6) {
        return v / len;
    }
    return vec2<f32>(1.0, 0.0);
}

fn side_for_vertex(index: u32) -> f32 {
    let sides = array<f32, 6>(-1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
    return sides[index];
}

fn endpoint_for_vertex(index: u32) -> u32 {
    let endpoints = array<u32, 6>(0u, 1u, 0u, 0u, 1u, 1u);
    return endpoints[index];
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let index = vertex_index % 6u;
    let endpoint = endpoint_for_vertex(index);
    let side = side_for_vertex(index);
    let view_start = uniforms.view * input.start;
    let view_end = uniforms.view * input.end;
    let clip_start = uniforms.projection * view_start;
    let clip_end = uniforms.projection * view_end;

    var output: VertexOutput;
    output.color = input.color;

    if (uniforms.flags.x != 0u) {
        var view_pos = view_start;
        if (endpoint == 1u) {
            view_pos = view_end;
        }
        let dir = safe_normalize_2d(view_end.xy - view_start.xy);
        let perp = vec2<f32>(-dir.y, dir.x);
        let offset = perp * side * (uniforms.params.z * 0.5);
        view_pos = vec4<f32>(view_pos.xy + offset, view_pos.zw);
        output.clip_position = uniforms.projection * view_pos;
        output.view_z = view_pos.z;
        return output;
    }

    var clip_pos = clip_start;
    var view_z = view_start.z;
    if (endpoint == 1u) {
        clip_pos = clip_end;
        view_z = view_end.z;
    }
    let ndc_start = clip_start.xy / max(clip_start.w, 1e-6);
    let ndc_end = clip_end.xy / max(clip_end.w, 1e-6);
    let dir = safe_normalize_2d(ndc_end - ndc_start);
    let perp = vec2<f32>(-dir.y, dir.x);
    let pixel_to_ndc = vec2<f32>(uniforms.params.x * 2.0, uniforms.params.y * 2.0);
    let offset = perp * side * (uniforms.params.z * 0.5) * pixel_to_ndc;
    clip_pos = vec4<f32>(clip_pos.xy + offset * clip_pos.w, clip_pos.zw);

    output.clip_position = clip_pos;
    output.view_z = view_z;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if (uniforms.flags.y != 0u) {
        let dims = vec2<i32>(textureDimensions(visibility_gbuffer));
        let max_pixel = max(dims - vec2<i32>(1, 1), vec2<i32>(0, 0));
        let pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0, 0), max_pixel);
        let gbuffer = textureLoad(visibility_gbuffer, pixel, 0);
        let normal_len_sq = dot(gbuffer.xyz, gbuffer.xyz);
        if (normal_len_sq > 1e-6 && gbuffer.w > input.view_z + uniforms.params.w) {
            discard;
        }
    }
    return input.color;
}
