// Ground-Truth Ambient Occlusion (GTAO). Two passes, both fullscreen triangles
// sharing the vertex shader:
//   * fs_gtao — horizon-based slice integration over a single-sample view-space
//               normal + depth G-buffer into an R8 visibility target.
//   * fs_blur — a bilateral blur that removes per-pixel-rotation noise while
//               preserving depth/normal edges.
// The composite (post.rs) applies the blurred AO only to ambient radiance.
//
// GTAO works in view space: the G-buffer stores the view-space normal (xyz) and
// the linear view-space Z (w, negative in front of the camera). For each slice
// direction the shader marches the screen-space horizon both ways, then integrates
// the cosine-weighted visible arc (the GTAO ground-truth integral), averaging over
// slices.

const PI: f32 = 3.14159265359;
const HALF_PI: f32 = 1.57079632679;

struct GtaoUniforms {
    // Projection matrix (view -> clip): projects sample points to screen and its
    // terms reconstruct view-space position from depth.
    proj: mat4x4<f32>,
    // x = sample radius (view units), y = intensity (power on visibility),
    // z = thickness heuristic (0..1), w = unused.
    params: vec4<f32>,
    // x = 1.0 when the projection is orthographic, else 0.0; y = slice count;
    // z = steps per slice; w = unused.
    config: vec4<f32>,
};

@group(0) @binding(0)
var gbuffer: texture_2d<f32>;
@group(0) @binding(1)
var raw_ao: texture_2d<f32>;
@group(0) @binding(2)
var gtao_sampler: sampler;
@group(0) @binding(3)
var<uniform> gtao: GtaoUniforms;

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
    if (gtao.config.x > 0.5) {
        // Orthographic: ndc = P00*x + P30 (x = (ndc - P30) / P00); Z is independent.
        let x = (ndc.x - gtao.proj[3][0]) / gtao.proj[0][0];
        let y = (ndc.y - gtao.proj[3][1]) / gtao.proj[1][1];
        return vec3<f32>(x, y, view_z);
    }
    // Perspective: ndc.x = P00 * x / (-view_z)  =>  x = ndc.x * (-view_z) / P00.
    let x = ndc.x * (-view_z) / gtao.proj[0][0];
    let y = ndc.y * (-view_z) / gtao.proj[1][1];
    return vec3<f32>(x, y, view_z);
}

// Project a view-space point to texture uv via the projection matrix.
fn project_to_uv(view_pos: vec3<f32>) -> vec2<f32> {
    let clip = gtao.proj * vec4<f32>(view_pos, 1.0);
    var ndc = clip.xy;
    if (gtao.config.x <= 0.5) {
        ndc = clip.xy / clip.w;
    }
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// Hash a screen coordinate to [0,1); used for the per-pixel slice rotation so the
// occlusion noise is high-frequency (and removed by the blur) rather than banded.
fn hash12(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Reconstruct the view-space position at `uv` (xyz) + a foreground flag (w): 0 for
// background (overlays / skybox / cleared frame, which wrote a zero normal / Z).
fn sample_view_pos(uv: vec2<f32>) -> vec4<f32> {
    let g = textureSampleLevel(gbuffer, gtao_sampler, uv, 0.0);
    if (dot(g.xyz, g.xyz) < 0.25 || g.w >= -1e-4) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return vec4<f32>(reconstruct_view_pos(uv, g.w), 1.0);
}

// March one side of a slice for the maximum horizon cosine (dot of the direction
// to the closest occluder with the view vector). `dir_px` is the per-step screen
// offset in pixels for that side; `inv_dims` converts pixels to uv.
fn horizon_cos(
    origin_uv: vec2<f32>,
    p: vec3<f32>,
    v: vec3<f32>,
    dir_px: vec2<f32>,
    radius_px: f32,
    radius: f32,
    thickness: f32,
    steps: u32,
    jitter: f32,
    inv_dims: vec2<f32>,
) -> f32 {
    var cos_h = -1.0;
    for (var t = 1u; t <= steps; t = t + 1u) {
        let frac = (f32(t) - jitter) / f32(steps);
        let uv = origin_uv + dir_px * (radius_px * frac) * inv_dims;
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            break;
        }
        let s = sample_view_pos(uv);
        if (s.w < 0.5) {
            continue;
        }
        let d = s.xyz - p;
        let len = length(d);
        if (len < 1e-5) {
            continue;
        }
        let c = dot(d, v) / len;
        // Radius falloff: ignore occluders past the radius; far ones inside the
        // radius are pulled back by the thickness heuristic so thin objects don't
        // over-occlude.
        let w = clamp(1.0 - len / radius, 0.0, 1.0);
        if (w <= 0.0) {
            continue;
        }
        cos_h = max(cos_h, c - (1.0 - w) * thickness);
    }
    return cos_h;
}

@fragment
fn fs_gtao(input: VertexOutput) -> @location(0) f32 {
    let g = textureSampleLevel(gbuffer, gtao_sampler, input.uv, 0.0);
    let raw_normal = g.xyz;
    let view_z = g.w;
    // Background (overlays / skybox / cleared frame write 0): no occlusion.
    if (dot(raw_normal, raw_normal) < 0.25 || view_z >= -1e-4) {
        return 1.0;
    }

    let n = normalize(raw_normal);
    let p = reconstruct_view_pos(input.uv, view_z);
    let v = normalize(-p);

    let radius = gtao.params.x;
    let intensity = gtao.params.y;
    let thickness = clamp(gtao.params.z, 0.0, 1.0);
    let slice_count = max(u32(gtao.config.y), 1u);
    let step_count = max(u32(gtao.config.z), 1u);

    let dims = vec2<f32>(textureDimensions(gbuffer));
    let inv_dims = 1.0 / dims;
    // Screen radius (px): UV span of `radius` view units along view X at this depth.
    let edge_uv = project_to_uv(p + vec3<f32>(radius, 0.0, 0.0));
    let radius_px = clamp(abs(edge_uv.x - input.uv.x) * dims.x, 1.0, max(dims.x, dims.y));

    // Per-pixel rotation + jitter so slices/steps decorrelate (blur removes noise).
    let noise = hash12(input.clip_position.xy);

    var visibility = 0.0;
    for (var s = 0u; s < slice_count; s = s + 1u) {
        let phi = (f32(s) + noise) * PI / f32(slice_count);
        let omega = vec2<f32>(cos(phi), sin(phi));

        // View-space slice direction: reconstruct a neighbor a few px along omega.
        let neighbor = sample_view_pos(input.uv + omega * (2.0 * inv_dims));
        var slice_dir = normalize(vec3<f32>(omega.x, omega.y, 0.0));
        if (neighbor.w > 0.5) {
            let d = neighbor.xyz - p;
            if (dot(d, d) > 1e-12) {
                slice_dir = normalize(d);
            }
        }

        // Project the normal onto the slice plane (spanned by v and slice_dir).
        let plane_normal = normalize(cross(v, slice_dir));
        let proj_n = n - plane_normal * dot(n, plane_normal);
        let proj_len = length(proj_n);
        if (proj_len < 1e-4) {
            continue;
        }
        let proj_n_dir = proj_n / proj_len;
        // Signed angle of the projected normal from the view vector in the plane.
        let sign_n = sign(dot(cross(slice_dir, proj_n_dir), plane_normal));
        let gamma = sign_n * acos(clamp(dot(proj_n_dir, v), -1.0, 1.0));

        // Search both horizons (positive omega and negative omega side).
        let cos_pos = horizon_cos(
            input.uv, p, v, omega, radius_px, radius, thickness, step_count, noise, inv_dims,
        );
        let cos_neg = horizon_cos(
            input.uv, p, v, -omega, radius_px, radius, thickness, step_count, noise, inv_dims,
        );

        // Clamp horizons into the hemisphere around the (projected) normal, then
        // integrate the visible cosine-weighted arc (GTAO inner integral).
        let h1 = gamma + max(-acos(clamp(cos_neg, -1.0, 1.0)) - gamma, -HALF_PI);
        let h2 = gamma + min(acos(clamp(cos_pos, -1.0, 1.0)) - gamma, HALF_PI);
        let cos_gamma = cos(gamma);
        let sin_gamma = sin(gamma);
        let arc = 0.25 * (
            (-cos(2.0 * h1 - gamma) + cos_gamma + 2.0 * h1 * sin_gamma)
            + (-cos(2.0 * h2 - gamma) + cos_gamma + 2.0 * h2 * sin_gamma)
        );
        visibility = visibility + proj_len * arc;
    }

    visibility = clamp(visibility / f32(slice_count), 0.0, 1.0);
    // Intensity sharpens the falloff (1 = ground truth, >1 darkens, 0 disables).
    return pow(visibility, max(intensity, 0.0));
}

// 5x5 bilateral blur over the raw AO. Depth and normal weights keep occlusion
// from bleeding across silhouettes and hard creases.
@fragment
fn fs_blur(input: VertexOutput) -> @location(0) f32 {
    let center_g = textureSampleLevel(gbuffer, gtao_sampler, input.uv, 0.0);
    let center_n = center_g.xyz;
    let center_z = center_g.w;
    if (dot(center_n, center_n) < 0.25 || center_z >= -1e-4) {
        return 1.0;
    }

    let dims = vec2<f32>(textureDimensions(raw_ao));
    let texel = 1.0 / dims;
    let n0 = normalize(center_n);
    let depth_sigma = max(gtao.params.x * 0.12, 1e-4);
    let spatial_sigma = 2.0;
    var sum = 0.0;
    var weight_sum = 0.0;
    for (var x = -2; x <= 2; x = x + 1) {
        for (var y = -2; y <= 2; y = y + 1) {
            let pixel_offset = vec2<f32>(f32(x), f32(y));
            let uv = input.uv + pixel_offset * texel;
            let g = textureSampleLevel(gbuffer, gtao_sampler, uv, 0.0);
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
            let ao = textureSampleLevel(raw_ao, gtao_sampler, uv, 0.0).r;
            sum = sum + ao * weight;
            weight_sum = weight_sum + weight;
        }
    }
    if (weight_sum <= 1e-5) {
        return textureSampleLevel(raw_ao, gtao_sampler, input.uv, 0.0).r;
    }
    return sum / weight_sum;
}
