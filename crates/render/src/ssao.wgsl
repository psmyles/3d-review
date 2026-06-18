// Screen-space ambient occlusion (CLAUDE.md render roadmap, Phase 5). Two passes,
// both fullscreen triangles sharing the vertex shader:
//   * fs_ssao  — hemisphere-kernel occlusion from a single-sample view-space
//                normal + depth G-buffer into an R8 AO target.
//   * fs_blur  — a bilateral blur that removes per-pixel-rotation noise while
//                preserving depth/normal edges.
// The composite (post.rs) applies the blurred AO only to ambient radiance.
//
// SSAO works in view space: the G-buffer stores the view-space normal (xyz) and
// the linear view-space Z (w, negative in front of the camera). The view position
// of any pixel is reconstructed from its uv + Z through the projection matrix
// (handling both perspective and orthographic), and kernel sample points are
// projected back through it to look up the occluding geometry's depth.

const KERNEL_SIZE: u32 = 16u;

struct SsaoUniforms {
    // Projection matrix (view -> clip): projects kernel sample points to screen and
    // its terms reconstruct view-space position from depth.
    proj: mat4x4<f32>,
    // x = sample radius (view units), y = depth bias (view units), z = intensity,
    // w = active sample count.
    params: vec4<f32>,
    // x = 1.0 when the projection is orthographic, else 0.0 (perspective).
    config: vec4<f32>,
    // Hemisphere kernel (tangent space, +Z), xyz used; w is padding.
    kernel: array<vec4<f32>, KERNEL_SIZE>,
};

@group(0) @binding(0)
var gbuffer: texture_2d<f32>;
@group(0) @binding(1)
var raw_ao: texture_2d<f32>;
@group(0) @binding(2)
var ssao_sampler: sampler;
@group(0) @binding(3)
var<uniform> ssao: SsaoUniforms;

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

// Reconstruct view-space position from a pixel's uv + its stored view-space Z.
fn reconstruct_view_pos(uv: vec2<f32>, view_z: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    if (ssao.config.x > 0.5) {
        // Orthographic: ndc = P00*x + P30 (x = (ndc - P30) / P00); Z is independent.
        let x = (ndc.x - ssao.proj[3][0]) / ssao.proj[0][0];
        let y = (ndc.y - ssao.proj[3][1]) / ssao.proj[1][1];
        return vec3<f32>(x, y, view_z);
    }
    // Perspective: ndc.x = P00 * x / (-view_z)  =>  x = ndc.x * (-view_z) / P00.
    let x = ndc.x * (-view_z) / ssao.proj[0][0];
    let y = ndc.y * (-view_z) / ssao.proj[1][1];
    return vec3<f32>(x, y, view_z);
}

// Project a view-space point to texture uv via the projection matrix.
fn project_to_uv(view_pos: vec3<f32>) -> vec2<f32> {
    let clip = ssao.proj * vec4<f32>(view_pos, 1.0);
    var ndc = clip.xy;
    if (ssao.config.x <= 0.5) {
        ndc = clip.xy / clip.w;
    }
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// Hash a screen coordinate to [0,1); used for the per-pixel kernel rotation so the
// occlusion noise is high-frequency (and removed by the blur) rather than banded.
fn hash12(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_ssao(input: VertexOutput) -> @location(0) f32 {
    let sample = textureSampleLevel(gbuffer, ssao_sampler, input.uv, 0.0);
    let normal = sample.xyz;
    let view_z = sample.w;

    // Background (overlays / skybox / cleared frame write 0): no occlusion.
    if (dot(normal, normal) < 0.25 || view_z >= -1e-4) {
        return 1.0;
    }

    let n = normalize(normal);
    let origin = reconstruct_view_pos(input.uv, view_z);
    let radius = ssao.params.x;
    let bias = ssao.params.y;

    // Random rotation about the normal, per pixel.
    let angle = hash12(input.clip_position.xy) * 6.2831853;
    let rand = vec3<f32>(cos(angle), sin(angle), 0.0);
    let tangent = normalize(rand - n * dot(rand, n));
    let bitangent = cross(n, tangent);
    let tbn = mat3x3<f32>(tangent, bitangent, n);

    let count = u32(ssao.params.w);
    var occlusion = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        // Sample point in view space, inside the hemisphere over the surface.
        let sample_pos = origin + (tbn * ssao.kernel[i].xyz) * radius;
        let sample_uv = project_to_uv(sample_pos);
        if (sample_uv.x < 0.0 || sample_uv.x > 1.0 || sample_uv.y < 0.0 || sample_uv.y > 1.0) {
            continue;
        }
        // Depth of the real geometry at that screen location.
        let scene_z = textureSampleLevel(gbuffer, ssao_sampler, sample_uv, 0.0).w;
        // Occluded when the real geometry is in front of the sample point (view Z
        // increases toward the camera, so a *larger* scene_z means closer).
        let occluded = select(0.0, 1.0, scene_z >= sample_pos.z + bias);
        // Ignore occluders far outside the radius (depth discontinuities), so the
        // AO doesn't smear across silhouettes.
        let range = smoothstep(0.0, 1.0, radius / max(abs(origin.z - scene_z), 1e-4));
        occlusion = occlusion + occluded * range;
    }

    let ao = 1.0 - (occlusion / f32(count)) * ssao.params.z;
    return clamp(ao, 0.0, 1.0);
}

// 5x5 bilateral blur over the raw AO. Depth and normal weights keep occlusion
// from bleeding across silhouettes and hard creases.
@fragment
fn fs_blur(input: VertexOutput) -> @location(0) f32 {
    let center_g = textureSampleLevel(gbuffer, ssao_sampler, input.uv, 0.0);
    let center_n = center_g.xyz;
    let center_z = center_g.w;
    if (dot(center_n, center_n) < 0.25 || center_z >= -1e-4) {
        return 1.0;
    }

    let dims = vec2<f32>(textureDimensions(raw_ao));
    let texel = 1.0 / dims;
    let n0 = normalize(center_n);
    let depth_sigma = max(max(ssao.params.x * 0.12, ssao.params.y * 4.0), 1e-4);
    let spatial_sigma = 2.0;
    var sum = 0.0;
    var weight_sum = 0.0;
    for (var x = -2; x <= 2; x = x + 1) {
        for (var y = -2; y <= 2; y = y + 1) {
            let pixel_offset = vec2<f32>(f32(x), f32(y));
            let uv = input.uv + pixel_offset * texel;
            let g = textureSampleLevel(gbuffer, ssao_sampler, uv, 0.0);
            let n_len = dot(g.xyz, g.xyz);
            if (n_len < 0.25 || g.w >= -1e-4) {
                continue;
            }

            let normal_dot = dot(n0, normalize(g.xyz));
            if (normal_dot < 0.75) {
                continue;
            }
            let spatial_weight = exp(-dot(pixel_offset, pixel_offset) / (2.0 * spatial_sigma * spatial_sigma));
            let dz = abs(g.w - center_z);
            let depth_weight = exp(-(dz * dz) / (2.0 * depth_sigma * depth_sigma));
            let normal_weight = smoothstep(0.75, 1.0, normal_dot);
            let weight = spatial_weight * depth_weight * normal_weight;
            let ao = textureSampleLevel(raw_ao, ssao_sampler, uv, 0.0).r;
            sum = sum + ao * weight;
            weight_sum = weight_sum + weight;
        }
    }
    if (weight_sum <= 1e-5) {
        return textureSampleLevel(raw_ao, ssao_sampler, input.uv, 0.0).r;
    }
    return sum / weight_sum;
}
