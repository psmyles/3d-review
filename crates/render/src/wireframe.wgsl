// Final model-wireframe overlay. Runs after post processing into egui's
// framebuffer, so colors are display-space UI colors and never enter bloom/SSAO.
//
// The overlay draws into egui's existing 4x-MSAA framebuffer. Each edge is
// expanded into a camera-facing ribbon a hair wider than the requested line so
// the fragment shader can compute an analytic 1px coverage falloff, which is
// fed to `alpha_to_coverage`. That turns the MSAA samples we already pay for
// into real per-sample wireframe antialiasing (no extra target or pass), and it
// is self-limiting on dense meshes: coverage caps how dark a pixel can get
// instead of building into a shimmering solid mass.

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
    // x = signed perpendicular distance from the line center in pixels,
    // y = the line's true (un-padded) half-width in pixels (the coverage core).
    @location(2) edge: vec2<f32>,
    // On-screen length of the whole edge in pixels (constant across the ribbon),
    // used to fade dense/zoomed-out wireframe so it stops fighting the grid.
    @location(3) edge_len_px: f32,
};

// Half-width (px) of the coverage falloff. The line is full inside
// (half - AA_BAND) and zero outside (half + AA_BAND), a 2*AA_BAND ~ 1px edge.
// Kept small so a 1px line stays ~1px instead of bloating into a soft band.
const AA_BAND: f32 = 0.5;

// Density fade: when an edge's on-screen length drops toward a few pixels the
// mesh is dense relative to the view, so individual edges can no longer be
// resolved and packing them at full strength produces a crawling moire. Edges
// shorter than DENSITY_FADE_MIN_PX fade to DENSITY_FADE_FLOOR; edges longer than
// DENSITY_FADE_FULL_PX stay at full strength. The fade lowers alpha, which (via
// alpha_to_coverage) also thins the sample mask, so dense regions settle into a
// stable wash instead of shimmering.
const DENSITY_FADE_MIN_PX: f32 = 3.0;
const DENSITY_FADE_FULL_PX: f32 = 14.0;
const DENSITY_FADE_FLOOR: f32 = 0.15;

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

    // Framebuffer resolution in pixels (params.xy carry its reciprocal).
    let viewport = vec2<f32>(1.0 / uniforms.params.x, 1.0 / uniforms.params.y);
    let half_viewport = viewport * 0.5;

    var clip_pos = clip_start;
    var view_pos = view_start;
    var view_z = view_start.z;
    if (endpoint == 1u) {
        clip_pos = clip_end;
        view_pos = view_end;
        view_z = view_end.z;
    }

    // Screen-space edge direction in pixels (aspect-correct, so the perpendicular
    // is a true pixel-space normal regardless of viewport shape).
    let ndc_start = clip_start.xy / max(clip_start.w, 1e-6);
    let ndc_end = clip_end.xy / max(clip_end.w, 1e-6);
    let screen_dir = safe_normalize_2d((ndc_end - ndc_start) * half_viewport);
    let perp = vec2<f32>(-screen_dir.y, screen_dir.x);

    // Target line half-width in pixels. World units convert per-vertex through the
    // projection so the on-screen thickness tracks perspective depth; screen units
    // are already pixels.
    var desired_half_px: f32;
    if (uniforms.flags.x != 0u) {
        let view_dir = safe_normalize_2d(view_end.xy - view_start.xy);
        let view_perp = vec2<f32>(-view_dir.y, view_dir.x);
        let world_half = uniforms.params.z * 0.5;
        let off_view = vec4<f32>(view_pos.xy + view_perp * world_half, view_pos.zw);
        let clip_off = uniforms.projection * off_view;
        let ndc_a = clip_pos.xy / max(clip_pos.w, 1e-6);
        let ndc_b = clip_off.xy / max(clip_off.w, 1e-6);
        desired_half_px = length((ndc_b - ndc_a) * half_viewport);
    } else {
        desired_half_px = uniforms.params.z * 0.5;
    }

    // Expand the ribbon just past the falloff band so the rasterizer always has
    // fragments to evaluate coverage over (even for a sub-pixel line); the
    // fragment shader recovers the true coverage from `desired_half_px`.
    let geom_half_px = desired_half_px + AA_BAND;
    let offset_px = perp * side * geom_half_px;
    let ndc_offset = offset_px / half_viewport;
    clip_pos = vec4<f32>(clip_pos.xy + ndc_offset * clip_pos.w, clip_pos.zw);

    var output: VertexOutput;
    output.color = input.color;
    output.clip_position = clip_pos;
    output.view_z = view_z;
    output.edge = vec2<f32>(side * geom_half_px, desired_half_px);
    output.edge_len_px = length((ndc_end - ndc_start) * half_viewport);
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
    // Analytic coverage: full inside (half - AA_BAND), zero outside
    // (half + AA_BAND), linear across the ~1px band. Sub-pixel lines (half <
    // AA_BAND) never reach full coverage, so they dim smoothly with their true
    // width instead of flickering on and off as they cross the pixel grid.
    let dist = abs(input.edge.x);
    let half = input.edge.y;
    let coverage = clamp(half - dist + AA_BAND, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    // Density fade by on-screen edge length: full strength for long edges, eased
    // down to DENSITY_FADE_FLOOR once edges shrink to a few pixels.
    let density = smoothstep(DENSITY_FADE_MIN_PX, DENSITY_FADE_FULL_PX, input.edge_len_px);
    let fade = mix(DENSITY_FADE_FLOOR, 1.0, density);
    // Coverage rides in alpha and drives alpha_to_coverage, which masks MSAA
    // samples so the existing 4x framebuffer antialiases the wireframe for free.
    return vec4<f32>(input.color.rgb, input.color.a * coverage * fade);
}
