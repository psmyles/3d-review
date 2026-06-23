//! Scene render-pipeline construction: the mesh / line / UV-fill / selection-fill
//! pipelines (rebuilt per MSAA level), the skybox pipeline, and the single-sample
//! SSAO G-buffer pipeline. Pure builders over the shared pipeline layout + shader.

use super::SCENE_DEPTH_FORMAT;
use super::SceneVertex;
use crate::targets::SCENE_HDR_FORMAT;

/// Depth behavior for a pipeline: whether it writes depth, and how much it biases
/// fragments. The mesh writes depth and pushes the surface back (so coplanar line
/// overlays win the depth test); the line pipeline does neither.
struct DepthConfig {
    write_enabled: bool,
    bias: wgpu::DepthBiasState,
}

/// Build the three scene pipelines (mesh / line / UV-fill) for a given MSAA
/// sample count. Rebuilt whenever the level changes, since the sample count is
/// baked into pipeline state.
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
) {
    let mesh_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        DepthConfig {
            write_enabled: true,
            bias: wgpu::DepthBiasState {
                constant: -2,
                slope_scale: -2.0,
                clamp: 0.0,
            },
        },
        sample_count,
        "fs_main",
        "review_scene_mesh_pipeline",
    );
    let line_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::LineList,
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
        line_pipeline,
        uv_fill_pipeline,
        selection_fill_pipeline,
        skybox_pipeline,
    )
}

/// Mesh-only pipeline that writes a single-sample view-space normal/Z buffer for
/// SSAO. It deliberately has no MSAA so geometry-edge normals/depths are not
/// averaged by a resolve before the AO shader samples them.
pub(super) fn create_ssao_gbuffer_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("review_ssao_gbuffer_pipeline"),
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
            entry_point: Some("fs_ssao_gbuffer"),
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
            // MRT to match the other scene pipelines: location 0 = linear HDR
            // scene color, location 1 = linear-HDR bloom source, location 2 =
            // AO-eligible ambient radiance. The skybox draws first over the
            // cleared frame, so no target blends (opaque replace); it writes zero
            // ambient so SSAO never darkens the background.
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

// Independent pipeline knobs (device + layout + shader + topology + depth + MSAA
// level + fragment entry + label); none is redundant and a params struct would
// only rename them, so the wide signature is intentional.
#[allow(clippy::too_many_arguments)]
fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
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
            cull_mode: None,
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
            // MRT (invariant 11: matches the `FragOutput` struct in `scene.wgsl`):
            // location 0 = linear HDR scene color, location 1 = linear-HDR bloom
            // source, location 2 = AO-eligible ambient radiance. All three
            // alpha-blend so transparent material/overlay coverage is handled in
            // linear light; overlays write zero ambient color with their own alpha
            // to mask the mesh ambient below, so AO does not darken the overlay.
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}
