//! The Direct3D 11 scene renderer — the live replacement for the dormant
//! egui-wgpu [`SceneCallback`] path (migration). It owns the GPU-side scene
//! resources (pipelines, buffers, depth) built lazily on the first frame and
//! drives the per-frame draw straight onto the swapchain backbuffer behind the
//! egui chrome.
//!
//! Phase 1 draws only the reference grid (the line pipeline + the `vs_main` /
//! `fs_line` HLSL): enough to verify the matrix convention (`mul(M, v)` with
//! column-major upload) and Reversed-Z depth. The mesh / overlays / UV / Tex
//! paths and the offscreen HDR MRT + post composite land in later phases, reusing
//! the same rhi wrappers and the CPU geometry in `crate::geometry`.

use crate::geometry::scene_lines;
use crate::rhi::{
    BlendMode, Cull, DepthBias, DepthCompare, DepthState, DepthTarget, DynamicConstantBuffer, Gpu,
    InputElement, Pipeline, PipelineDesc, Topology, VertexBuffer, VertexFormat,
};
use crate::{CameraProjection, OrbitCamera, SceneDebugOptions};

use super::gpu_types::{SceneUniforms, shading_mode_value, vertex_color_value};

/// Compiled DXBC for the scene vertex shader (`vs_main`) — see `build.rs`.
const SCENE_VS: &[u8] = include_bytes!("../hlsl/scene.vs.dxbc");
/// Compiled DXBC for the line/grid fragment shader (`fs_line`).
const SCENE_LINE_PS: &[u8] = include_bytes!("../hlsl/scene.line.ps.dxbc");

/// The `SceneVertex` input layout, in field order. Offsets are auto-computed
/// (`APPEND_ALIGNED`), so this only names the HLSL semantics + their formats; it
/// must match both `#[repr(C)] SceneVertex` (in `gpu_types`) and `VsInput` in
/// `scene.hlsl`.
const SCENE_VERTEX_LAYOUT: [InputElement; 5] = [
    InputElement::new("POSITION", 0, VertexFormat::Float3),
    InputElement::new("NORMAL", 0, VertexFormat::Float3),
    InputElement::new("TEXCOORD", 0, VertexFormat::Float2),
    InputElement::new("TANGENT", 0, VertexFormat::Float4),
    InputElement::new("COLOR", 0, VertexFormat::Float4),
];

/// The Direct3D 11 scene GPU resources. Built once (lazily) on the first
/// `render`, then reused; the depth target is recreated when the framebuffer
/// resizes.
pub(crate) struct SceneGpu {
    /// Shared per-frame uniforms (group 0 / cbuffer `b0`).
    uniforms: DynamicConstantBuffer,
    /// The line-list pipeline for the grid + (later) every line overlay.
    line_pipeline: Pipeline,
    /// The static reference grid vertex buffer (built once; model-independent).
    grid: VertexBuffer,
    /// Reversed-Z depth buffer, recreated on resize.
    depth: DepthTarget,
}

impl std::fmt::Debug for SceneGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneGpu").finish_non_exhaustive()
    }
}

impl SceneGpu {
    /// Build the scene GPU resources. Called once on the first frame, with the
    /// live device + initial backbuffer size.
    pub(crate) fn new(gpu: &Gpu) -> windows::core::Result<Self> {
        let device = gpu.device();
        let (width, height) = gpu.size();

        let uniforms = DynamicConstantBuffer::new::<SceneUniforms>(device)?;

        // The line pipeline: depth-tested (Reversed-Z `GreaterEqual`) but neither
        // writing nor biasing depth, alpha-blended, single-sample (MSAA in Phase 5).
        let line_pipeline = Pipeline::new(
            device,
            &PipelineDesc {
                vs: SCENE_VS,
                ps: SCENE_LINE_PS,
                input: &SCENE_VERTEX_LAYOUT,
                topology: Topology::LineList,
                cull: Cull::None,
                depth: DepthState {
                    test: true,
                    write: false,
                    compare: DepthCompare::GreaterEqual,
                },
                blend: BlendMode::AlphaBlend,
                depth_bias: DepthBias::default(),
                sample_count: 1,
            },
        )?;

        let grid = VertexBuffer::new(device, &scene_lines())?;
        let depth = DepthTarget::new(device, width, height)?;

        Ok(Self {
            uniforms,
            line_pipeline,
            grid,
            depth,
        })
    }

    /// Render the scene to the backbuffer: clear, then draw the grid (Phase 1).
    /// The egui chrome is drawn on top afterwards by `app` (the central viewport
    /// area of the chrome is transparent, so the scene shows through).
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        camera: OrbitCamera,
        projection: CameraProjection,
        debug: SceneDebugOptions,
        clear: [f32; 4],
    ) -> windows::core::Result<()> {
        // Keep the depth buffer matched to the (already-resized) backbuffer.
        if self.depth.size() != gpu.size() {
            let (width, height) = gpu.size();
            self.depth = DepthTarget::new(gpu.device(), width, height)?;
        }

        let ctx = gpu.context();
        let uniforms = scene_uniforms(camera, projection, debug);
        self.uniforms.update(ctx, &uniforms)?;

        gpu.begin_backbuffer_pass(&self.depth, clear);
        // `vs_main` reads the uniforms from `b0`; `fs_line` reads none. Bind to the
        // vertex stage only for now (the fragment-stage binding lands with the mesh).
        self.uniforms.bind_vs(ctx, 0);

        if debug.show_grid {
            self.line_pipeline.bind(ctx);
            self.grid.bind(ctx);
            gpu.draw(self.grid.count());
        }

        Ok(())
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera + projection + debug
/// options. Phase 1 only needs `view_projection` (the grid's `vs_main`), but the
/// whole block is filled so the cbuffer layout is exercised exactly as the mesh
/// path will use it (Phase 2 adds the environment + selection inputs).
fn scene_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    debug: SceneDebugOptions,
) -> SceneUniforms {
    let view_projection = camera.view_projection(projection);
    SceneUniforms {
        view_projection: view_projection.to_cols_array_2d(),
        inv_view_projection: view_projection.inverse().to_cols_array_2d(),
        render_options: [
            shading_mode_value(debug.shading_mode),
            0.0,
            debug.uv_checker_tiling.max(1) as f32,
            vertex_color_value(debug),
        ],
        camera_position: camera.eye_position().extend(0.0).to_array(),
        // The grid/line path ignores the environment; Phase 2 fills these from the
        // real `EnvironmentSettings`.
        env_params: [0.0, 1.0, 0.0, 0.0],
        projection_params: [
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            0.0,
            0.0,
            0.0,
        ],
        view: camera.view_matrix().to_cols_array_2d(),
        selection_color: [0.0; 4],
    }
}
