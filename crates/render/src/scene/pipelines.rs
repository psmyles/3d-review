//! Scene render-pipeline construction: the mesh / line / UV-fill / selection-fill
//! pipelines (rebuilt per MSAA level), the skybox pipeline, and the single-sample
//! GTAO G-buffer pipeline. Pure builders over the shared pipeline layout + shader.

use super::SCENE_DEPTH_FORMAT;
use super::SceneVertex;
use crate::targets::SCENE_HDR_FORMAT;

/// The scene pass's four MRT color targets — the single source of truth for the
/// `FragOutput` lockstep (the CLAUDE.md "four color targets" gotcha): location 0
/// = linear HDR scene color, location 1 = linear-HDR bloom source, location 2 =
/// AO-eligible diffuse ambient radiance, location 3 = IBL specular (rgb) +
/// roughness (a) for bent-normal-aware specular occlusion. `blend` applies to all
/// four: the geometry pipelines alpha-blend (transparent coverage handled in
/// linear light; overlays write zero ambient/specular to mask the mesh terms so
/// AO doesn't darken them), the skybox draws opaque (`None`) over the cleared
/// frame. Any new scene-pass pipeline (e.g. Phase 7's alpha-sort write-off
/// variant) gets the right shape by calling this rather than re-listing four
/// targets.
fn scene_color_targets(blend: Option<wgpu::BlendState>) -> [Option<wgpu::ColorTargetState>; 4] {
    let target = wgpu::ColorTargetState {
        format: SCENE_HDR_FORMAT,
        blend,
        write_mask: wgpu::ColorWrites::ALL,
    };
    [
        Some(target.clone()),
        Some(target.clone()),
        Some(target.clone()),
        Some(target),
    ]
}

/// Depth behavior for a pipeline: whether it writes depth, and how much it biases
/// fragments. The mesh writes depth and pushes the surface back (so coplanar line
/// overlays win the depth test); the line pipeline does neither.
struct DepthConfig {
    write_enabled: bool,
    bias: wgpu::DepthBiasState,
}

/// Build the scene pipelines (mesh / double-sided mesh / line / UV-fill /
/// selection-fill / skybox) for a given MSAA sample count. Rebuilt whenever the
/// level changes, since the sample count is baked into pipeline state.
///
/// The mesh is built twice — once culling back faces (`mesh_pipeline`, the
/// default) and once double-sided (`mesh_pipeline_double_sided`, no culling) —
/// so the "Backface Rendering" toggle is a per-frame pipeline pick rather than a
/// rebuild (front faces are CCW: glam's `_rh` projection + wgpu's framebuffer
/// winding, unaffected by the reversed-Z / ortho near-far swap). Only the mesh
/// needs the pair: lines aren't subject to face culling, the UV fill and
/// selection flash stay double-sided so they always show.
///
/// The wireframe (and other line overlays) share vertex positions with the
/// shaded surface they trace, so they z-fight it: on curved faces edges sink
/// behind the surface and drop out, and MSAA partial occlusion leaves the
/// survivors uneven in opacity/thickness. We can't bias the lines directly — on
/// DX12 depth bias applies only to triangle primitives — so instead the mesh
/// pipeline pushes the shaded surface a hair *away* from the camera with a
/// slope-scaled depth bias. In Reversed-Z, away from the camera is a smaller
/// depth value, so the bias is negative. Lines then render at their true depth
/// and win the `GreaterEqual` test against the receded surface, while still being correctly
/// occluded by geometry genuinely in front of them (slope-scaled bias is in real
/// depth-buffer units, so it never over-pulls the far side through the front the
/// way a constant clip-space line offset did). The scene pipelines render into
/// the offscreen HDR target (`SCENE_HDR_FORMAT`), not egui's framebuffer; the
/// post pass composites the result back.
pub(super) fn build_scene_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    sample_count: u32,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    // The mesh writes depth and pushes the surface back so coplanar line overlays
    // win the depth test; both the culling and double-sided variants share it.
    let mesh_depth = || DepthConfig {
        write_enabled: true,
        bias: wgpu::DepthBiasState {
            constant: -2,
            slope_scale: -2.0,
            clamp: 0.0,
        },
    };
    let mesh_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        Some(wgpu::Face::Back),
        mesh_depth(),
        sample_count,
        "fs_main",
        "review_scene_mesh_pipeline",
    );
    // The "Backface Rendering" on variant: identical to the mesh pipeline but with
    // culling disabled, so both sides of the surface are drawn.
    let mesh_pipeline_double_sided = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        None,
        mesh_depth(),
        sample_count,
        "fs_main",
        "review_scene_mesh_double_sided_pipeline",
    );
    let line_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::LineList,
        None,
        DepthConfig {
            write_enabled: false,
            bias: wgpu::DepthBiasState::default(),
        },
        sample_count,
        "fs_main",
        "review_scene_line_pipeline",
    );
    // The UV island fill draws flat-color triangles in the 2D viewport. It never
    // writes depth (everything sits at z=0) so the grid below and the wireframe
    // above composite purely by draw order.
    let uv_fill_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        None,
        DepthConfig {
            write_enabled: false,
            bias: wgpu::DepthBiasState::default(),
        },
        sample_count,
        "fs_main",
        "review_scene_uv_fill_pipeline",
    );
    // The selection flash: a flat-color triangle fill redrawing the selected
    // triangles over the shaded mesh. Like the line overlays it depth-tests
    // (Reversed-Z `GreaterEqual`) but never writes depth, so it is occluded by
    // geometry genuinely in front of the selection yet wins over the coplanar
    // surface it tints. `fs_selection` emits the uniform highlight color × fade.
    let selection_fill_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        None,
        DepthConfig {
            write_enabled: false,
            bias: wgpu::DepthBiasState::default(),
        },
        sample_count,
        "fs_selection",
        "review_scene_selection_fill_pipeline",
    );
    let skybox_pipeline = create_skybox_pipeline(device, layout, shader, sample_count);
    (
        mesh_pipeline,
        mesh_pipeline_double_sided,
        line_pipeline,
        uv_fill_pipeline,
        selection_fill_pipeline,
        skybox_pipeline,
    )
}

/// Mesh-only pipeline that writes a single-sample view-space normal/Z buffer for
/// GTAO. It deliberately has no MSAA so geometry-edge normals/depths are not
/// averaged by a resolve before the AO shader samples them.
pub(super) fn create_gtao_gbuffer_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("review_gtao_gbuffer_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[SceneVertex::layout()],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SCENE_DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_gtao_gbuffer"),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

/// The skybox pipeline: a fullscreen triangle (no vertex buffer) sampling the
/// environment cubemap. It draws first in the offscreen pass over the cleared
/// frame, so it never writes depth and always passes the depth test; scene
/// geometry then composites on top. Renders into the HDR target at the scene's
/// MSAA level, so it is rebuilt alongside the other pipelines when MSAA changes.
fn create_skybox_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("review_scene_skybox_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_skybox"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SCENE_DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_skybox"),
            // Opaque MRT (the skybox draws first over the cleared frame); writes
            // zero ambient so GTAO never darkens the background.
            targets: &scene_color_targets(None),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

// Independent pipeline knobs (device + layout + shader + topology + cull + depth +
// MSAA level + fragment entry + label); none is redundant and a params struct
// would only rename them, so the wide signature is intentional. `cull_mode` is
// `Some(Back)` for the default mesh pipeline and `None` everywhere a face must
// always draw (the double-sided mesh, lines, the UV/selection fills).
#[allow(clippy::too_many_arguments)]
fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
    cull_mode: Option<wgpu::Face>,
    depth: DepthConfig,
    sample_count: u32,
    fragment_entry: &'static str,
    label: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[SceneVertex::layout()],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SCENE_DEPTH_FORMAT,
            depth_write_enabled: depth.write_enabled,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            stencil: wgpu::StencilState::default(),
            bias: depth.bias,
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            // The four alpha-blended scene-pass MRT targets (invariant 11).
            targets: &scene_color_targets(Some(wgpu::BlendState::ALPHA_BLENDING)),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}
