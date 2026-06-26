//! The Direct3D 11 scene renderer — the live replacement for the dormant
//! egui-wgpu [`SceneCallback`] path (migration). It owns the GPU-side scene
//! resources (pipelines, buffers, offscreen targets, IBL maps, material table)
//! built lazily on the first frame and drives the per-frame render: the scene draws
//! into offscreen linear-HDR MRT targets, then a composite pass tone-maps + sRGB-
//! encodes them to the swapchain backbuffer behind the egui chrome.
//!
//! Phase 2b covers the full shaded look: the skybox + the per-material PBR/IBL
//! `fs_main` (material cbuffer `b1` + the seven texture slots `t5..t11` + the IBL
//! maps `t1..t4` + the UV checker `t0`), drawn one indexed range per material.
//! Phase 3 adds GTAO + the live tone-map operator switch; Phase 4 the UV / Tex
//! viewports, the debug line views and the selection flash.

use bytemuck::{Pod, Zeroable};
use review_model::ModelData;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::geometry::{model_mesh, scene_lines};
use crate::ibl::{IblD3d, PREFILTER_MAX_LOD};
use crate::material::{
    MaterialDrawRange, MaterialState, MaterialTableD3d, build_part_key, effective_materials,
};
use crate::rhi::{
    BlendMode, ColorTarget, Cull, DepthBias, DepthCompare, DepthState, DepthTarget,
    DynamicConstantBuffer, Gpu, IndexBuffer, InputElement, Pipeline, PipelineDesc, Sampler,
    Texture, Topology, VertexBuffer, VertexFormat,
};
use crate::{
    ActiveMaterial, CameraProjection, CheckerTexture, EnvironmentSettings, MaterialMode,
    OrbitCamera, SceneDebugOptions, ShadingMode,
};

use super::gpu_types::{SceneUniforms, shading_mode_value, vertex_color_value};

/// Compiled DXBC — see `build.rs`.
const SCENE_VS: &[u8] = include_bytes!("../hlsl/scene.vs.dxbc");
const SCENE_LINE_PS: &[u8] = include_bytes!("../hlsl/scene.line.ps.dxbc");
const SCENE_MESH_PS: &[u8] = include_bytes!("../hlsl/scene.mesh.ps.dxbc");
const SCENE_SKYBOX_VS: &[u8] = include_bytes!("../hlsl/scene.skybox.vs.dxbc");
const SCENE_SKYBOX_PS: &[u8] = include_bytes!("../hlsl/scene.skybox.ps.dxbc");
const POST_VS: &[u8] = include_bytes!("../hlsl/post.vs.dxbc");
const POST_PS: &[u8] = include_bytes!("../hlsl/post.ps.dxbc");

/// The greyscale + color UV-checker PNGs (baked in; invariant: assets via
/// `include_bytes!`), uploaded once as sRGB textures and picked per frame.
const CHECKER_GREYSCALE_PNG: &[u8] =
    include_bytes!("../../../../assets/textures/T_UV_Checker_BW.png");
const CHECKER_COLOR_PNG: &[u8] = include_bytes!("../../../../assets/textures/T_UV_Checker_CLR.png");

/// The `SceneVertex` input layout, in field order (offsets auto-computed). Must
/// match `#[repr(C)] SceneVertex` (`gpu_types`) and `VsInput` in `scene.hlsl`.
const SCENE_VERTEX_LAYOUT: [InputElement; 5] = [
    InputElement::new("POSITION", 0, VertexFormat::Float3),
    InputElement::new("NORMAL", 0, VertexFormat::Float3),
    InputElement::new("TEXCOORD", 0, VertexFormat::Float2),
    InputElement::new("TANGENT", 0, VertexFormat::Float4),
    InputElement::new("COLOR", 0, VertexFormat::Float4),
];

/// Composite-pass uniform (cbuffer `b0` in `post.hlsl`). Phase 2b keeps GTAO off
/// and tone mapping on (PBR-Neutral); Phase 3 drives these from the live settings.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniforms {
    gtao_enabled: u32,
    tonemap_enabled: u32,
    tonemap_op: u32,
}

/// The mesh's GPU buffers + per-material draw ranges, rebuilt when the model (or UV
/// channel / material mode) changes. `None` for an empty model (the grid still draws).
struct MeshBuffers {
    vertices: VertexBuffer,
    indices: IndexBuffer,
    ranges: Vec<MaterialDrawRange>,
}

/// The Direct3D 11 scene GPU resources, built once (lazily) on the first `render`.
/// The offscreen targets + depth are recreated on resize; the mesh buffers when the
/// model changes; the IBL maps when the environment changes.
pub(crate) struct SceneGpu {
    /// Shared per-frame scene uniforms (cbuffer `b0`).
    uniforms: DynamicConstantBuffer,
    /// Composite-pass uniform (cbuffer `b0` in the post shader).
    post_uniforms: DynamicConstantBuffer,
    line_pipeline: Pipeline,
    mesh_pipeline: Pipeline,
    mesh_double_sided_pipeline: Pipeline,
    skybox_pipeline: Pipeline,
    composite_pipeline: Pipeline,
    /// Linear clamp sampler — the composite's input sampler (`s0` of the post pass)
    /// and the IBL sampler (`s1` of the scene pass).
    sampler: Sampler,
    /// Repeat sampler for the UV checker (`s0` of the scene pass).
    checker_sampler: Sampler,
    /// The two baked UV-checker textures (`t0`), picked per frame.
    checker_greyscale: Texture,
    checker_color: Texture,
    /// The image-based-lighting maps (`t1..t4`), reloaded on environment change.
    ibl: IblD3d,
    /// The editable per-material table (`b1` + `t5..t11` + `s2`).
    materials: MaterialTableD3d,
    /// The static reference grid (built once; model-independent).
    grid: VertexBuffer,
    /// Offscreen linear-HDR targets: location 0 scene color, location 1 ambient.
    color: ColorTarget,
    ambient: ColorTarget,
    depth: DepthTarget,
    mesh: Option<MeshBuffers>,
    mesh_revision: u64,
    mesh_uv_channel: u32,
    mesh_material_mode: MaterialMode,
    /// Cached Unique-mode per-triangle mesh-part key + count, baked by model
    /// revision while Unique is active (invariant 3: freed otherwise).
    unique_part_key: Vec<u32>,
    unique_part_count: usize,
    unique_baked: Option<u64>,
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
        let mesh_desc = |cull| PipelineDesc {
            vs: SCENE_VS,
            ps: SCENE_MESH_PS,
            input: &SCENE_VERTEX_LAYOUT,
            topology: Topology::TriangleList,
            cull,
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
        };
        let mesh_pipeline = Pipeline::new(device, &mesh_desc(Cull::Back))?;
        // Double-sided variant for Backface Rendering: identical but uncull.
        let mesh_double_sided_pipeline = Pipeline::new(device, &mesh_desc(Cull::None))?;

        // The skybox: a fullscreen triangle (no vertex buffer / input layout) drawn
        // first, behind geometry — depth-test always, no write, opaque overwrite.
        let skybox_pipeline = Pipeline::new(
            device,
            &PipelineDesc {
                vs: SCENE_SKYBOX_VS,
                ps: SCENE_SKYBOX_PS,
                input: &[],
                topology: Topology::TriangleList,
                cull: Cull::None,
                depth: DepthState {
                    test: true,
                    write: false,
                    compare: DepthCompare::Always,
                },
                blend: BlendMode::Opaque,
                depth_bias: DepthBias::default(),
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
        let checker_sampler = Sampler::linear_repeat(device)?;
        let checker_greyscale = decode_checker(device, CHECKER_GREYSCALE_PNG)?;
        let checker_color = decode_checker(device, CHECKER_COLOR_PNG)?;
        let ibl = IblD3d::from_baked(gpu, EnvironmentSettings::default().map)?;
        let materials = MaterialTableD3d::new(device)?;
        let grid = VertexBuffer::new(device, &scene_lines())?;
        let color = ColorTarget::new(device, width, height)?;
        let ambient = ColorTarget::new(device, width, height)?;
        let depth = DepthTarget::new(device, width, height)?;

        Ok(Self {
            uniforms,
            post_uniforms,
            line_pipeline,
            mesh_pipeline,
            mesh_double_sided_pipeline,
            skybox_pipeline,
            composite_pipeline,
            sampler,
            checker_sampler,
            checker_greyscale,
            checker_color,
            ibl,
            materials,
            grid,
            color,
            ambient,
            depth,
            mesh: None,
            // Sentinel distinct from any real revision so the first frame builds the
            // mesh (or leaves it None for an empty model).
            mesh_revision: u64::MAX,
            mesh_uv_channel: 0,
            mesh_material_mode: MaterialMode::Source,
            unique_part_key: Vec::new(),
            unique_part_count: 0,
            unique_baked: None,
        })
    }

    /// Render the scene: skybox + mesh (per-material PBR/IBL) + grid into the
    /// offscreen HDR MRT, then composite (tone map + sRGB) to the backbuffer. The
    /// egui chrome is drawn on top afterwards by `app`.
    // Independent per-frame inputs (gpu + model + revisions + materials + camera +
    // projection + environment + debug + clear); none is redundant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        model: &ModelData,
        model_revision: u64,
        material_states: &[MaterialState],
        material_revision: u64,
        camera: OrbitCamera,
        projection: CameraProjection,
        environment: EnvironmentSettings,
        debug: SceneDebugOptions,
        clear: [f32; 4],
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let ctx = gpu.context();

        // Keep the offscreen targets + depth matched to the (already-resized)
        // backbuffer.
        if self.color.size() != gpu.size() {
            let (width, height) = gpu.size();
            self.color = ColorTarget::new(device, width, height)?;
            self.ambient = ColorTarget::new(device, width, height)?;
            self.depth = DepthTarget::new(device, width, height)?;
        }

        // Reconcile the Unique-mode part key, then the mesh + the effective material
        // table (both depend on the active material mode's grouping).
        self.sync_unique_parts(model, model_revision, debug.material_mode);
        self.sync_mesh(
            device,
            model,
            model_revision,
            debug.uv_channel,
            debug.material_mode,
        )?;
        let effective =
            effective_materials(debug.material_mode, material_states, self.unique_part_count);
        self.materials.sync(
            device,
            ctx,
            &effective,
            material_revision,
            debug.material_mode,
        )?;

        // Reload the IBL maps when the chosen environment changes (a pure upload).
        if self.ibl.environment != environment.map {
            self.ibl = IblD3d::from_baked(gpu, environment.map)?;
        }

        let uniforms = scene_uniforms(camera, projection, environment, debug);
        self.uniforms.update(ctx, &uniforms)?;

        // --- Offscreen scene pass (2 MRT + depth). ---
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, clear);
        // Shared bindings: `b0` (VS + PS), the checker (`t0`/`s0`), the IBL maps
        // (`t1..t4`) + their sampler (`s1`), and the material cbuffer + sampler
        // (`b1`/`s2`). The mesh loop rebinds `b1` + `t5..t11` per range.
        self.uniforms.bind_vs(ctx, 0);
        self.uniforms.bind_ps(ctx, 0);
        let checker = match debug.uv_checker_texture {
            CheckerTexture::Greyscale => &self.checker_greyscale,
            CheckerTexture::Color => &self.checker_color,
        };
        checker.bind_ps(ctx, 0);
        self.checker_sampler.bind_ps(ctx, 0);
        self.ibl.bind_ps(ctx);
        self.sampler.bind_ps(ctx, 1);
        self.materials.bind_shared(ctx);
        self.materials.bind_fallback(ctx)?;

        // Skybox background first, behind all geometry, when shown.
        if environment.show_background {
            self.skybox_pipeline.bind(ctx);
            gpu.draw(3);
        }

        // Mesh: one indexed draw per material range, each binding its own material
        // (uniform + slot SRVs). Wireframe shading draws no filled surface.
        if let Some(mesh) = &self.mesh
            && !matches!(debug.shading_mode, ShadingMode::Wireframe)
        {
            let pipeline = if debug.render_backfaces {
                &self.mesh_double_sided_pipeline
            } else {
                &self.mesh_pipeline
            };
            pipeline.bind(ctx);
            mesh.vertices.bind(ctx);
            mesh.indices.bind(ctx);
            for range in &mesh.ranges {
                self.materials.set_range(ctx, range.material)?;
                gpu.draw_indexed_range(range.index_count, range.first_index);
            }
        }

        if debug.show_grid {
            self.line_pipeline.bind(ctx);
            self.grid.bind(ctx);
            gpu.draw(self.grid.count());
        }

        // --- Composite to the backbuffer. ---
        // Phase 2b: GTAO off, tone mapping on (PBR-Neutral). Phase 3 drives these
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

    /// Reconcile the Unique-mode per-triangle mesh-part key: baked by `model_revision`
    /// while Unique is active, freed back to empty otherwise (invariant 3). Mirrors
    /// the wgpu `sync_unique_parts`.
    fn sync_unique_parts(&mut self, model: &ModelData, model_revision: u64, mode: MaterialMode) {
        let want = matches!(mode, MaterialMode::Unique).then_some(model_revision);
        if self.unique_baked == want {
            return;
        }
        match want {
            Some(_) => {
                let (key, count) = build_part_key(model);
                self.unique_part_key = key;
                self.unique_part_count = count;
            }
            None => {
                self.unique_part_key = Vec::new();
                self.unique_part_count = 0;
            }
        }
        self.unique_baked = want;
    }

    /// Rebuild the mesh buffers + per-material draw ranges when the model, UV channel
    /// or material mode changes (the index reorder groups by the active mode's key).
    /// An empty model leaves `mesh` as `None`. Call after [`Self::sync_unique_parts`].
    fn sync_mesh(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        uv_channel: u32,
        mode: MaterialMode,
    ) -> windows::core::Result<()> {
        if self.mesh_revision == model_revision
            && self.mesh_uv_channel == uv_channel
            && self.mesh_material_mode == mode
        {
            return Ok(());
        }
        let key = match mode {
            MaterialMode::Unique if !self.unique_part_key.is_empty() => {
                Some(self.unique_part_key.as_slice())
            }
            _ => None,
        };
        let (vertices, indices, ranges) = model_mesh(model, uv_channel, key);
        self.mesh = if indices.is_empty() {
            None
        } else {
            Some(MeshBuffers {
                vertices: VertexBuffer::new(device, &vertices)?,
                indices: IndexBuffer::new(device, &indices)?,
                ranges,
            })
        };
        self.mesh_revision = model_revision;
        self.mesh_uv_channel = uv_channel;
        self.mesh_material_mode = mode;
        Ok(())
    }
}

/// Decode a baked UV-checker PNG into an sRGB GPU texture. A decode failure is a
/// packaging bug — fall back to a 1×1 white texel rather than failing the build of
/// the scene resources (mirrors the wgpu `create_checker_bind_group`).
fn decode_checker(device: &ID3D11Device, png_bytes: &[u8]) -> windows::core::Result<Texture> {
    match image::load_from_memory(png_bytes) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Texture::rgba8_single(device, width, height, &rgba, true)
        }
        Err(_) => Texture::rgba8_single(device, 1, 1, &[255, 255, 255, 255], true),
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera, projection, environment
/// and debug options. Mirrors the wgpu `update_camera` (selection flash is Phase 4,
/// so `selection_color` stays zero).
fn scene_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    environment: EnvironmentSettings,
    debug: SceneDebugOptions,
) -> SceneUniforms {
    let view_projection = camera.view_projection(projection);
    SceneUniforms {
        view_projection: view_projection.to_cols_array_2d(),
        inv_view_projection: view_projection.inverse().to_cols_array_2d(),
        render_options: [
            shading_mode_value(debug.shading_mode),
            if debug.active_material == ActiveMaterial::UvChecker {
                1.0
            } else {
                0.0
            },
            debug.uv_checker_tiling.max(1) as f32,
            vertex_color_value(debug),
        ],
        camera_position: camera.eye_position().extend(0.0).to_array(),
        env_params: [
            if environment.ibl_enabled { 1.0 } else { 0.0 },
            environment.intensity.max(0.0),
            if environment.show_background {
                1.0
            } else {
                0.0
            },
            PREFILTER_MAX_LOD,
        ],
        projection_params: [
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            environment.rotation_degrees.to_radians(),
            0.0,
            0.0,
        ],
        view: camera.view_matrix().to_cols_array_2d(),
        selection_color: [0.0; 4],
    }
}
