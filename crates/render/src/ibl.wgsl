// Image-based-lighting precompute shaders (CLAUDE.md render roadmap, Phase 3).
// One module, four fragment entry points, all driven by `ibl.rs`:
//   * fs_equirect_to_cube — project the loaded equirectangular HDR onto the six
//     faces of an environment cubemap.
//   * fs_irradiance        — cosine-convolve the env cube into a low-res diffuse
//     irradiance cube.
//   * fs_prefilter         — GGX importance-sample the env cube into the mip
//     chain of a prefiltered specular cube (one roughness per mip).
//   * fs_brdf              — integrate the split-sum BRDF response into a 2D LUT.
// Every pass renders a fullscreen triangle; the cube passes reconstruct the
// world-space sample direction from a per-face basis in `FaceUniform`. These run
// once per environment switch (see `ibl.rs`), never per frame.

const PI: f32 = 3.14159265359;

// Per-face / per-pass parameters. `forward/right/up` are the cube face's basis:
// a fullscreen-triangle position (clip xy in -1..1) maps to the world direction
// `forward + x*right + y*up`. `params.x` carries the prefilter roughness.
struct FaceUniform {
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> face: FaceUniform;
// Source for the equirect->cube pass (a 2D equirectangular texture)...
@group(0) @binding(1) var src_equirect: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;
// ...and for the irradiance / prefilter passes (the env cubemap). Declared in
// the same module but only referenced by the passes that use it, so each pass's
// explicit bind-group layout lists just the bindings it touches.
@group(0) @binding(3) var src_cube: texture_cube<f32>;

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    // World-space direction for this fragment (cube passes).
    @location(0) local_dir: vec3<f32>,
    // 0..1 screen coordinate (BRDF LUT pass): x = N·V, y = roughness.
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    // A single oversized triangle covering the framebuffer.
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let xy = positions[vertex_index];

    var out: VsOut;
    out.clip_position = vec4<f32>(xy, 0.0, 1.0);
    out.local_dir = face.forward.xyz + xy.x * face.right.xyz + xy.y * face.up.xyz;
    // Flip Y so texel row t (top = 0) maps to roughness = t for the BRDF LUT.
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 1.0 - (xy.y * 0.5 + 0.5));
    return out;
}

// Map a unit direction to equirectangular UV: u around the equator, v from the
// top (+Y) down. Matches the orientation `dir_to_equirect_uv` assumes when the
// HDR is uploaded row 0 = top.
fn dir_to_equirect_uv(dir: vec3<f32>) -> vec2<f32> {
    let n = normalize(dir);
    let u = atan2(n.z, n.x) / (2.0 * PI) + 0.5;
    let v = acos(clamp(n.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}

@fragment
fn fs_equirect_to_cube(in: VsOut) -> @location(0) vec4<f32> {
    let uv = dir_to_equirect_uv(in.local_dir);
    let color = textureSampleLevel(src_equirect, src_sampler, uv, 0.0).rgb;
    return vec4<f32>(color, 1.0);
}

@fragment
fn fs_irradiance(in: VsOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.local_dir);

    // Build a tangent frame around the surface normal.
    var up_axis = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) {
        up_axis = vec3<f32>(0.0, 0.0, 1.0);
    }
    let tangent = normalize(cross(up_axis, normal));
    let bitangent = cross(normal, tangent);

    var irradiance = vec3<f32>(0.0);
    var samples = 0.0;
    let sample_delta = 0.05;
    var phi = 0.0;
    loop {
        if (phi >= 2.0 * PI) { break; }
        var theta = 0.0;
        loop {
            if (theta >= 0.5 * PI) { break; }
            // Spherical -> tangent-space -> world.
            let tangent_sample = vec3<f32>(
                sin(theta) * cos(phi),
                sin(theta) * sin(phi),
                cos(theta),
            );
            let sample_vec = tangent_sample.x * tangent
                + tangent_sample.y * bitangent
                + tangent_sample.z * normal;
            let radiance = textureSampleLevel(src_cube, src_sampler, sample_vec, 0.0).rgb;
            irradiance += radiance * cos(theta) * sin(theta);
            samples += 1.0;
            theta += sample_delta;
        }
        phi += sample_delta;
    }
    irradiance = PI * irradiance / max(samples, 1.0);
    return vec4<f32>(irradiance, 1.0);
}

// Van der Corput / Hammersley low-discrepancy sequence for importance sampling.
fn radical_inverse_vdc(bits_in: u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}

// Sample a GGX half-vector around `n` for a given roughness, from a 2D random.
fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a = roughness * roughness;
    let phi = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);

    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);

    var up_axis = vec3<f32>(0.0, 0.0, 1.0);
    if (abs(n.z) > 0.999) {
        up_axis = vec3<f32>(1.0, 0.0, 0.0);
    }
    let tangent = normalize(cross(up_axis, n));
    let bitangent = cross(n, tangent);
    return normalize(tangent * h.x + bitangent * h.y + n * h.z);
}

@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.local_dir);
    let reflect_dir = normal;
    let view_dir = normal;
    let roughness = face.params.x;

    let sample_count = 64u;
    var prefiltered = vec3<f32>(0.0);
    var total_weight = 0.0;
    for (var i = 0u; i < sample_count; i = i + 1u) {
        let xi = hammersley(i, sample_count);
        let h = importance_sample_ggx(xi, normal, roughness);
        let l = normalize(2.0 * dot(view_dir, h) * h - view_dir);
        let n_dot_l = max(dot(normal, l), 0.0);
        if (n_dot_l > 0.0) {
            // Clamp the sampled radiance: importance sampling a tiny, very bright
            // texel otherwise leaves sparkle (fireflies) in the prefiltered mip.
            let radiance = min(
                textureSampleLevel(src_cube, src_sampler, l, 0.0).rgb,
                vec3<f32>(64.0),
            );
            prefiltered += radiance * n_dot_l;
            total_weight += n_dot_l;
        }
    }
    if (total_weight > 0.0) {
        prefiltered = prefiltered / total_weight;
    }
    return vec4<f32>(prefiltered, 1.0);
}

// Smith geometry term for IBL (k uses the IBL remap, not the direct-lighting one).
fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let k = (roughness * roughness) / 2.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

@fragment
fn fs_brdf(in: VsOut) -> @location(0) vec2<f32> {
    let n_dot_v = max(in.uv.x, 1e-4);
    let roughness = in.uv.y;

    let view = vec3<f32>(sqrt(1.0 - n_dot_v * n_dot_v), 0.0, n_dot_v);
    let normal = vec3<f32>(0.0, 0.0, 1.0);

    var a = 0.0;
    var b = 0.0;
    let sample_count = 512u;
    for (var i = 0u; i < sample_count; i = i + 1u) {
        let xi = hammersley(i, sample_count);
        let h = importance_sample_ggx(xi, normal, roughness);
        let l = normalize(2.0 * dot(view, h) * h - view);

        let n_dot_l = max(l.z, 0.0);
        let n_dot_h = max(h.z, 0.0);
        let v_dot_h = max(dot(view, h), 0.0);
        if (n_dot_l > 0.0) {
            let g = geometry_smith(n_dot_v, n_dot_l, roughness);
            let g_vis = (g * v_dot_h) / (n_dot_h * n_dot_v);
            let fc = pow(1.0 - v_dot_h, 5.0);
            a += (1.0 - fc) * g_vis;
            b += fc * g_vis;
        }
    }
    return vec2<f32>(a, b) / f32(sample_count);
}
