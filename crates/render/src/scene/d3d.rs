//! The Direct3D 11 scene renderer — the live replacement for the dormant
//! egui-wgpu [`SceneCallback`] path (migration). It owns the GPU-side scene
//! resources (pipelines, buffers, offscreen targets) built lazily on the first
//! frame and drives the per-frame render: the scene draws into offscreen
//! linear-HDR MRT targets, then a composite pass tone-maps + sRGB-encodes them to
//! the swapchain backbuffer behind the egui chrome.
//!
//! Phase 2a covers the grid + the mesh under analytic lighting (`fs_mesh`) +
//! the offscreen-HDR composite (`post.hlsl`, PBR-Neutral tone map). Phase 2b
//! replaces `fs_mesh` with the full per-material PBR/IBL `fs_main` (material
//! cbuffer + texture/IBL SRVs + checker + skybox); Phase 3 adds GTAO + the
//! tone-map operator switch to the composite.

use bytemuck::{Pod, Zeroable};
use review_model::ModelData;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::geometry::{model_mesh, scene_lines};
use crate::rhi::{
    BlendMode, ColorTarget, Cull, DepthBias, DepthCompare, DepthState, DepthTarget,
    DynamicConstantBuffer, Gpu, IndexBuffer, InputElement, Pipeline, PipelineDesc, Sampler,
    Topology, VertexBuffer, VertexFormat,
};
use crate::{CameraProjection, OrbitCamera, SceneDebugOptions};

use super::gpu_types::{SceneUniforms, shading_mode_value, vertex_color_value};

/// Compiled DXBC — see `build.rs`.
const SCENE_VS: &[u8] = include_bytes!("../hlsl/scene.vs.dxbc");
const SCENE_LINE_PS: &[u8] = include_bytes!("../hlsl/scene.line.ps.dxbc");
const SCENE_MESH_PS: &[u8] = include_bytes!("../hlsl/scene.mesh.ps.dxbc");
const POST_VS: &[u8] = include_bytes!("../hlsl/post.vs.dxbc");
const POST_PS: &[u8] = include_bytes!("../hlsl/post.ps.dxbc");

/// The `SceneVertex` input layout, in field order (offsets auto-computed). Must
/// match `#[repr(C)] SceneVertex` (`gpu_types`) and `VsInput` in `scene.hlsl`.
const SCENE_VERTEX_LAYOUT: [InputElement; 5] = [
    InputElement::new("POSITION", 0, VertexFormat::Float3),
    InputElement::new("NORMAL", 0, VertexFormat::Float3),
    InputElement::new("TEXCOORD", 0, VertexFormat::Float2),
    InputElement::new("TANGENT", 0, VertexFormat::Float4),
    InputElement::new("COLOR", 0, VertexFormat::Float4),
];

/// Composite-pass uniform (cbuffer `b0` in `post.hlsl`). Phase 2a keeps GTAO off
/// and tone mapping on (PBR-Neutral); Phase 3 drives these from the live settings.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniforms {
    gtao_enabled: u32,
    tonemap_enabled: u32,
    tonemap_op: u32,
}

/// The mesh's GPU buffers, rebuilt when the model (or UV channel) changes. `None`
/// for an empty model (the grid still draws).
struct MeshBuffers {
    vertices: VertexBuffer,
    indices: IndexBuffer,
}

/// The Direct3D 11 scene GPU resources, built once (lazily) on the first `render`.
/// The offscreen targets + depth are recreated on resize; the mesh buffers when
/// the model changes.
pub(crate) struct SceneGpu {
    /// Shared per-frame scene uniforms (cbuffer `b0`).
    uniforms: DynamicConstantBuffer,
    /// Composite-pass uniform (cbuffer `b0` in the post shader).
    post_uniforms: DynamicConstantBuffer,
    line_pipeline: Pipeline,
    mesh_pipeline: Pipeline,
    composite_pipeline: Pipeline,
    /// Linear sampler the composite reads its inputs with.
    sampler: Sampler,
    /// The static reference grid (built once; model-independent).
    grid: VertexBuffer,
    /// Offscreen linear-HDR targets: location 0 scene color, location 1 ambient.
    color: ColorTarget,
    ambient: ColorTarget,
    depth: DepthTarget,
    mesh: Option<MeshBuffers>,
    mesh_revision: u64,
    mesh_uv_channel: u32,
}

impl std::fmt::Debug for SceneGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneGpu").finish_non_exhaustive()
    }
}

impl SceneGpu {
    /// Build the scene GPU resources. Called once on the first frame.
    pub(crate) fn new(gpu: &Gpu) -> windows::core::Result<Self> {
        let device = gpu.device();
        let (width, height) = gpu.size();

        let uniforms = DynamicConstantBuffer::new::<SceneUniforms>(device)?;
        let post_uniforms = DynamicConstantBuffer::new::<PostUniforms>(device)?;

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

        // The mesh pipeline: back-face culled, writes depth + pushes the surface
        // back (slope-scaled bias) so coplanar line overlays win the test.
        let mesh_pipeline = Pipeline::new(
            device,
            &PipelineDesc {
                vs: SCENE_VS,
                ps: SCENE_MESH_PS,
                input: &SCENE_VERTEX_LAYOUT,
                topology: Topology::TriangleList,
                cull: Cull::Back,
                depth: DepthState {
                    test: true,
                    write: true,
                    compare: DepthCompare::GreaterEqual,
                },
                blend: BlendMode::AlphaBlend,
                depth_bias: DepthBias {
                    constant: -2,
                    slope_scaled: -2.0,
                },
                sample_count: 1,
            },
        )?;

        // The composite: a fullscreen triangle (no vertex buffer / input layout),
        // depth disabled, opaque overwrite of the backbuffer.
        let composite_pipeline = Pipeline::new(
            device,
            &PipelineDesc {
                vs: POST_VS,
                ps: POST_PS,
                input: &[],
                topology: Topology::TriangleList,
                cull: Cull::None,
                depth: DepthState {
                    test: false,
                    write: false,
                    compare: DepthCompare::Always,
                },
                blend: BlendMode::Opaque,
                depth_bias: DepthBias::default(),
                sample_count: 1,
            },
        )?;

        let sampler = Sampler::linear_clamp(device)?;
        let grid = VertexBuffer::new(device, &scene_lines())?;
        let color = ColorTarget::new(device, width, height)?;
        let ambient = ColorTarget::new(device, width, height)?;
        let depth = DepthTarget::new(device, width, height)?;

        Ok(Self {
            uniforms,
            post_uniforms,
            line_pipeline,
            mesh_pipeline,
            composite_pipeline,
            sampler,
            grid,
            color,
            ambient,
            depth,
            mesh: None,
            // Sentinel distinct from any real revision so the first frame builds the
            // mesh (or leaves it None for an empty model).
            mesh_revision: u64::MAX,
            mesh_uv_channel: 0,
        })
    }

    /// Render the scene: draw the mesh + grid into the offscreen HDR MRT, then
    /// composite (tone map + sRGB) to the backbuffer. The egui chrome is drawn on
    /// top afterwards by `app`.
    // Independent per-frame inputs (gpu + model + revision + camera + projection +
    // debug options + clear); none is redundant, so the wide signature is intended.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        model: &ModelData,
        model_revision: u64,
        camera: OrbitCamera,
        projection: CameraProjection,
        debug: SceneDebugOptions,
        clear: [f32; 4],
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        // Keep the offscreen targets + depth matched to the (already-resized)
        // backbuffer.
        if self.color.size() != gpu.size() {
            let (width, height) = gpu.size();
            self.color = ColorTarget::new(device, width, height)?;
            self.ambient = ColorTarget::new(device, width, height)?;
            self.depth = DepthTarget::new(device, width, height)?;
        }
        self.sync_mesh(device, model, model_revision, debug.uv_channel)?;

        let ctx = gpu.context();
        let uniforms = scene_uniforms(camera, projection, debug);
        self.uniforms.update(ctx, &uniforms)?;

        // --- Offscreen scene pass (2 MRT + depth). ---
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, clear);
        // `vs_main` reads the uniforms from `b0` (VS); `fs_mesh` reads
        // `camera_position` from `b0` (PS). `fs_line` reads neither.
        self.uniforms.bind_vs(ctx, 0);
        self.uniforms.bind_ps(ctx, 0);

        if let Some(mesh) = &self.mesh {
            // Wireframe mode draws no filled surface (Phase 2a: the mesh is the only
            // filled draw, so just skip it).
            if !matches!(debug.shading_mode, crate::ShadingMode::Wireframe) {
                self.mesh_pipeline.bind(ctx);
                mesh.vertices.bind(ctx);
                mesh.indices.bind(ctx);
                gpu.draw_indexed(mesh.indices.count());
            }
        }

        if debug.show_grid {
            self.line_pipeline.bind(ctx);
            self.grid.bind(ctx);
            gpu.draw(self.grid.count());
        }

        // --- Composite to the backbuffer. ---
        // Phase 2a: GTAO off, tone mapping on (PBR-Neutral). Phase 3 drives these
        // from the live tonemap/GTAO settings.
        let post = PostUniforms {
            gtao_enabled: 0,
            tonemap_enabled: 1,
            tonemap_op: 0,
        };
        self.post_uniforms.update(ctx, &post)?;
        gpu.begin_backbuffer_blit();
        self.composite_pipeline.bind(ctx);
        self.post_uniforms.bind_ps(ctx, 0);
        self.color.bind_ps_srv(ctx, 0);
        // t1 (GTAO) is unused while GTAO is off; bind the ambient view as a harmless
        // placeholder so the slot isn't left dangling. t2 is the real ambient.
        self.ambient.bind_ps_srv(ctx, 1);
        self.ambient.bind_ps_srv(ctx, 2);
        self.sampler.bind_ps(ctx, 0);
        gpu.draw(3);
        // Release the offscreen SRVs so next frame can bind them as render targets.
        gpu.unbind_ps_srvs(3);

        Ok(())
    }

    /// Rebuild the mesh buffers when the model or UV channel changes (Phase 2a
    /// draws the whole mesh as one indexed draw; per-material ranges arrive in
    /// Phase 2b). An empty model leaves `mesh` as `None`.
    fn sync_mesh(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        uv_channel: u32,
    ) -> windows::core::Result<()> {
        if self.mesh_revision == model_revision && self.mesh_uv_channel == uv_channel {
            return Ok(());
        }
        let (vertices, indices, _ranges) = model_mesh(model, uv_channel, None);
        self.mesh = if indices.is_empty() {
            None
        } else {
            Some(MeshBuffers {
                vertices: VertexBuffer::new(device, &vertices)?,
                indices: IndexBuffer::new(device, &indices)?,
            })
        };
        self.mesh_revision = model_revision;
        self.mesh_uv_channel = uv_channel;
        Ok(())
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera + projection + debug
/// options. Phase 2a uses `view_projection` (mesh + grid vs_main) and
/// `camera_position` (the analytic specular view vector); the environment +
/// selection inputs are filled in Phase 2b.
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
