//! The Direct3D 11 scene renderer. It owns the GPU-side scene resources (pipelines,
//! buffers, offscreen targets, IBL maps, material table) built lazily on the first
//! frame and drives the per-frame render: the scene draws into offscreen linear-HDR
//! MRT targets, then a composite pass tone-maps + sRGB-encodes them to the swapchain
//! backbuffer behind the egui chrome.
//!
//! The full shaded look is the skybox + the per-material PBR/IBL `fs_main` (material
//! cbuffer `b1` + the seven texture slots `t5..t11` + the IBL maps `t1..t4` + the UV
//! checker `t0`), drawn one indexed range per material, plus GTAO + the live tone-map
//! operator switch, the UV / Tex viewports, the debug line views and the selection
//! flash.

use bytemuck::{Pod, Zeroable};
use review_model::ModelData;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_mesh, scene_lines, selection_geometry,
    uv_fill_triangles, uv_grid_lines, uv_wireframe_lines, vertex_normal_lines, visible_geometry,
    wireframe_lines,
};
use crate::ibl::{IblD3d, PREFILTER_MAX_LOD};
use crate::material::{
    MaterialDrawRange, MaterialState, MaterialTableD3d, build_part_key, effective_materials,
};
use crate::rhi::{
    BlendMode, ColorTarget, Cull, DepthBias, DepthCompare, DepthState, DepthTarget,
    DynamicConstantBuffer, Gpu, IndexBuffer, InputElement, Pipeline, PipelineDesc, Sampler,
    Texture, Topology, VertexBuffer, VertexFormat,
};
use crate::selection::{Selection, SelectionView, selection_bounds};
use crate::{
    ActiveMaterial, AntiAliasing, BoundingBoxScope, CameraProjection, CheckerTexture,
    EnvironmentSettings, GtaoSettings, MaterialMode, OrbitCamera, SceneDebugOptions, ShadingMode,
    TonemapSettings, UvCamera, UvShadingMode,
};

use super::gpu_profiler::{self, GpuProfiler, Zone};
use super::gpu_types::{SceneUniforms, buffer_view_value, shading_mode_value, vertex_color_value};

/// Compiled DXBC — see `build.rs`.
const SCENE_VS: &[u8] = include_bytes!("../hlsl/scene.vs.dxbc");
const SCENE_LINE_PS: &[u8] = include_bytes!("../hlsl/scene.line.ps.dxbc");
const SCENE_MESH_PS: &[u8] = include_bytes!("../hlsl/scene.mesh.ps.dxbc");
const SCENE_SKYBOX_VS: &[u8] = include_bytes!("../hlsl/scene.skybox.vs.dxbc");
const SCENE_SKYBOX_PS: &[u8] = include_bytes!("../hlsl/scene.skybox.ps.dxbc");
const SCENE_SELECTION_PS: &[u8] = include_bytes!("../hlsl/scene.selection.ps.dxbc");
const SCENE_GTAO_GBUFFER_PS: &[u8] = include_bytes!("../hlsl/scene.gtao_gbuffer.ps.dxbc");
const GTAO_VS: &[u8] = include_bytes!("../hlsl/gtao.vs.dxbc");
const GTAO_PS: &[u8] = include_bytes!("../hlsl/gtao.ps.dxbc");
const GTAO_BLUR_PS: &[u8] = include_bytes!("../hlsl/gtao.blur.ps.dxbc");
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

/// Composite-pass uniform (cbuffer `b0` in `post.hlsl`): the GTAO enable flag, the
/// tone-map enable + operator, and a raw-passthrough flag, all driven from the
/// live settings each frame.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniforms {
    gtao_enabled: u32,
    tonemap_enabled: u32,
    tonemap_op: u32,
    /// When non-zero the composite blits the (already display-ready) scene color
    /// straight to the backbuffer — no GTAO, tone map or sRGB encode. Set for the
    /// [`ActiveMaterial::Buffers`] data-inspection view, whose scene shader emits
    /// final display pixels itself so the shown value is faithful.
    passthrough: u32,
}

/// GTAO-pass uniform (cbuffer `b0` in `gtao.hlsl`). `#[repr(C)]` + `Pod` to match
/// the HLSL `GtaoUniforms` layout (invariant 11): a `float4x4` + two `float4`s, all
/// 16-byte aligned. Uploaded each frame so the panel sliders stay live.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GtaoUniforms {
    /// View → clip projection (column-major), for reconstruction + sample projection.
    proj: [[f32; 4]; 4],
    /// x = radius (view units), y = intensity, z = thickness, w unused.
    params: [f32; 4],
    /// x = is_ortho (1.0 / 0.0), y = slice count, z = steps per slice, w unused.
    config: [f32; 4],
}

/// Baked parameters for the bounding-box view. Only the inputs the chosen scope
/// depends on are populated (the hidden set for `VisibleOnly`, the selection for
/// `OnlySelection`), so an unrelated change can't rebuild the box.
#[derive(PartialEq)]
struct BoundingBoxParams {
    color: [f32; 4],
    scope: BoundingBoxScope,
    hidden: Vec<u32>,
    selection: Selection,
}

/// Baked parameters for a normal-line view (face or vertex normals).
#[derive(PartialEq)]
struct NormalParams {
    length: f32,
    color: [f32; 4],
    hidden: Vec<u32>,
}

/// Bake key for the selection draw list — the hidden set because hiding a selected
/// mesh drops it, the mode because Unique re-groups the solo list by part.
#[derive(PartialEq)]
struct SelectionBaked {
    model_revision: u64,
    selection: Selection,
    hidden: Vec<u32>,
    mode: MaterialMode,
}

/// Bake key for the per-mesh visibility draw list.
#[derive(PartialEq)]
struct VisibilityBaked {
    model_revision: u64,
    hidden: Vec<u32>,
    mode: MaterialMode,
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
    /// GTAO-pass uniform (cbuffer `b0` in the gtao shader).
    gtao_uniforms: DynamicConstantBuffer,
    line_pipeline: Pipeline,
    mesh_pipeline: Pipeline,
    mesh_double_sided_pipeline: Pipeline,
    skybox_pipeline: Pipeline,
    composite_pipeline: Pipeline,
    /// Mesh-only single-sample view-normal/Z G-buffer pipeline (`fs_gtao_gbuffer`).
    gtao_gbuffer_pipeline: Pipeline,
    /// Horizon-based occlusion fullscreen pass (`fs_gtao`).
    gtao_pipeline: Pipeline,
    /// 5×5 bilateral-blur fullscreen pass (`fs_blur`).
    gtao_blur_pipeline: Pipeline,
    /// Scene MSAA sample count the MSAA-dependent pipelines + targets are built for
    /// (1 = no multisampling). Rebuilt when the live AA level changes.
    scene_sample_count: u32,
    /// Linear clamp sampler — the composite's input sampler (`s0` of the post pass)
    /// and the IBL sampler (`s1` of the scene pass).
    sampler: Sampler,
    /// Repeat sampler for the UV checker (`s0` of the scene pass).
    checker_sampler: Sampler,
    /// Point-clamp sampler for the GTAO passes (`s0`), so view normals / depths are
    /// never blended across geometry edges.
    gtao_sampler: Sampler,
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
    /// GTAO targets, all full-resolution + recreated on resize: the single-sample
    /// view-normal/Z G-buffer (HDR) + its own depth, then the raw and blurred
    /// occlusion (`R8`). Sized to the framebuffer like the scene targets.
    gtao_gbuffer: ColorTarget,
    gtao_depth: DepthTarget,
    gtao_raw: ColorTarget,
    gtao_blur: ColorTarget,
    mesh: Option<MeshBuffers>,
    mesh_revision: u64,
    mesh_uv_channel: u32,
    mesh_material_mode: MaterialMode,
    /// Cached Unique-mode per-triangle mesh-part key + count, baked by model
    /// revision while Unique is active (invariant 3: freed otherwise).
    unique_part_key: Vec<u32>,
    unique_part_count: usize,
    unique_baked: Option<u64>,
    // Derived 3D debug line views: each buffer exists only while its toggle is on
    // and is rebuilt live when its baked params drift (invariant 3, mirroring the
    // wgpu `sync_line_views`). All draw with the shared `line_pipeline`.
    /// Model wireframe (original-polygon edges); `None` while off. The bake key is
    /// `(color, hidden_nodes)`.
    wireframe_buf: Option<VertexBuffer>,
    wireframe_baked: Option<([f32; 4], Vec<u32>)>,
    /// Axis-aligned bounding box; `None` while off or when the scope wraps no
    /// geometry.
    bounding_box_buf: Option<VertexBuffer>,
    bounding_box_baked: Option<BoundingBoxParams>,
    /// One line per face along its normal; `None` while off.
    face_normal_buf: Option<VertexBuffer>,
    face_baked: Option<NormalParams>,
    /// One line per vertex along its normal; `None` while off.
    vertex_normal_buf: Option<VertexBuffer>,
    vertex_baked: Option<NormalParams>,
    /// Selection-flash fill pipeline (`fs_selection`): flat highlight color × fade,
    /// depth-tested (Reversed-Z `GreaterEqual`) but no depth write, alpha-blended.
    selection_pipeline: Pipeline,
    /// The selected triangles reordered per-material over a fresh index buffer that
    /// shares the mesh vertex buffer (the solo isolate list + the flash fill source).
    /// `None` while nothing is selected or the selection resolves to no geometry
    /// (invariant 3). Built on demand by [`Self::sync_selection`].
    selection_index: Option<IndexBuffer>,
    selection_ranges: Vec<MaterialDrawRange>,
    selection_baked: Option<SelectionBaked>,
    /// The visible (non-hidden) triangles reordered per-material over a fresh index
    /// buffer sharing the mesh vertex buffer; drawn instead of the full mesh while
    /// `visible_active`. `None` when nothing is hidden (full mesh), or when every
    /// mesh is hidden (active but empty → draw nothing).
    visible_index: Option<IndexBuffer>,
    visible_ranges: Vec<MaterialDrawRange>,
    /// Whether the per-mesh visibility filter is in effect (some mesh hidden and the
    /// model carries per-triangle node info): the mesh + GTAO passes then draw the
    /// filtered list instead of the full mesh.
    visible_active: bool,
    visibility_baked: Option<VisibilityBaked>,
    // --- 2D UV viewport (drawn instead of the 3D scene in UV workspace mode). ---
    /// The static 0..1 reference grid (built once; model-independent).
    uv_grid: VertexBuffer,
    /// Flat-color triangle fill for the UV islands (`fs_main`'s zero-normal overlay
    /// branch — same as the line views but filled). No depth write; alpha-blended.
    uv_fill_pipeline: Pipeline,
    /// The model's UV edges for the active channel; `None` until built / for an empty
    /// model. Built on demand per `(model_revision, channel)`.
    uv_wireframe_buf: Option<VertexBuffer>,
    uv_wireframe_baked: Option<(u64, u32)>,
    /// The UV island fill (Shaded / Islands modes only); `None` in Wire mode. Built
    /// per `(model_revision, channel, shading_mode)`.
    uv_fill_buf: Option<VertexBuffer>,
    uv_fill_baked: Option<(u64, u32, UvShadingMode)>,
    /// Hand-rolled D3D11 timestamp-query → Tracy GPU profiler. `None` unless `--tracy`
    /// armed it and a Tracy client is running; built lazily on the first profiled
    /// frame and never touched otherwise (every scene pass records no timestamps).
    gpu_profiler: Option<GpuProfiler>,
}

impl std::fmt::Debug for SceneGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneGpu").finish_non_exhaustive()
    }
}

impl SceneGpu {
    /// Build the scene GPU resources. Called once on the first frame; `sample_count`
    /// is the initial scene MSAA level (the live AA setting), so the scene pipelines +
    /// targets start at the right level and the first frame needs no rebuild.
    pub(crate) fn new(gpu: &Gpu, sample_count: u32) -> windows::core::Result<Self> {
        let device = gpu.device();
        let (width, height) = gpu.size();
        let sample_count = sample_count.max(1);

        let uniforms = DynamicConstantBuffer::new::<SceneUniforms>(device)?;
        let post_uniforms = DynamicConstantBuffer::new::<PostUniforms>(device)?;
        let gtao_uniforms = DynamicConstantBuffer::new::<GtaoUniforms>(device)?;

        // The MSAA-dependent scene pipelines (line / mesh / double-sided mesh /
        // skybox / UV fill / selection fill) — all draw into the MSAA scene MRT, so
        // their sample count is baked at the live AA level and rebuilt when it changes
        // ([`Self::rebuild_scene_pipelines`]).
        let scene = build_scene_pipelines(device, sample_count)?;

        // The composite: a fullscreen triangle (no vertex buffer / input layout),
        // depth disabled, opaque overwrite of the backbuffer. Always single-sample —
        // it draws to the backbuffer, not the MSAA MRT.
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

        // GTAO G-buffer: a mesh-only pass writing the single-sample view normal/Z.
        // No culling (matches the wgpu G-buffer), writes + tests depth (Reversed-Z
        // `GreaterEqual`), no bias, opaque single target.
        let gtao_gbuffer_pipeline = Pipeline::new(
            device,
            &PipelineDesc {
                vs: SCENE_VS,
                ps: SCENE_GTAO_GBUFFER_PS,
                input: &SCENE_VERTEX_LAYOUT,
                topology: Topology::TriangleList,
                cull: Cull::None,
                depth: DepthState {
                    test: true,
                    write: true,
                    compare: DepthCompare::GreaterEqual,
                },
                blend: BlendMode::Opaque,
                depth_bias: DepthBias::default(),
                sample_count: 1,
            },
        )?;

        // The two GTAO fullscreen passes (occlusion + bilateral blur): a fullscreen
        // triangle (no vertex buffer / input layout), depth disabled, opaque write
        // into the `R8` AO targets. Shared `PipelineDesc` differing only in the PS.
        let gtao_fullscreen_desc = |ps| PipelineDesc {
            vs: GTAO_VS,
            ps,
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
        };
        let gtao_pipeline = Pipeline::new(device, &gtao_fullscreen_desc(GTAO_PS))?;
        let gtao_blur_pipeline = Pipeline::new(device, &gtao_fullscreen_desc(GTAO_BLUR_PS))?;

        let sampler = Sampler::linear_clamp(device)?;
        let checker_sampler = Sampler::linear_repeat(device)?;
        let gtao_sampler = Sampler::point_clamp(device)?;
        let checker_greyscale = decode_checker(device, CHECKER_GREYSCALE_PNG)?;
        let checker_color = decode_checker(device, CHECKER_COLOR_PNG)?;
        let ibl = IblD3d::from_baked(gpu, EnvironmentSettings::default().map)?;
        let materials = MaterialTableD3d::new(device)?;
        let grid = VertexBuffer::new(device, &scene_lines())?;
        let uv_grid = VertexBuffer::new(device, &uv_grid_lines())?;
        // The scene MRT + depth carry the MSAA level; the GTAO targets stay single-
        // sample (its own mesh-only pass, never resolved).
        let color = ColorTarget::hdr_msaa(device, width, height, sample_count)?;
        let ambient = ColorTarget::hdr_msaa(device, width, height, sample_count)?;
        let depth = DepthTarget::with_samples(device, width, height, sample_count)?;
        let gtao_gbuffer = ColorTarget::new(device, width, height)?;
        let gtao_depth = DepthTarget::new(device, width, height)?;
        let gtao_raw = ColorTarget::r8(device, width, height)?;
        let gtao_blur = ColorTarget::r8(device, width, height)?;

        Ok(Self {
            uniforms,
            post_uniforms,
            gtao_uniforms,
            line_pipeline: scene.line,
            mesh_pipeline: scene.mesh,
            mesh_double_sided_pipeline: scene.mesh_double_sided,
            skybox_pipeline: scene.skybox,
            composite_pipeline,
            gtao_gbuffer_pipeline,
            gtao_pipeline,
            gtao_blur_pipeline,
            scene_sample_count: sample_count,
            sampler,
            checker_sampler,
            gtao_sampler,
            checker_greyscale,
            checker_color,
            ibl,
            materials,
            grid,
            color,
            ambient,
            depth,
            gtao_gbuffer,
            gtao_depth,
            gtao_raw,
            gtao_blur,
            mesh: None,
            // Sentinel distinct from any real revision so the first frame builds the
            // mesh (or leaves it None for an empty model).
            mesh_revision: u64::MAX,
            mesh_uv_channel: 0,
            mesh_material_mode: MaterialMode::Source,
            unique_part_key: Vec::new(),
            unique_part_count: 0,
            unique_baked: None,
            wireframe_buf: None,
            wireframe_baked: None,
            bounding_box_buf: None,
            bounding_box_baked: None,
            face_normal_buf: None,
            face_baked: None,
            vertex_normal_buf: None,
            vertex_baked: None,
            selection_pipeline: scene.selection,
            selection_index: None,
            selection_ranges: Vec::new(),
            selection_baked: None,
            visible_index: None,
            visible_ranges: Vec::new(),
            visible_active: false,
            visibility_baked: None,
            uv_grid,
            uv_fill_pipeline: scene.uv_fill,
            uv_wireframe_buf: None,
            uv_wireframe_baked: None,
            uv_fill_buf: None,
            uv_fill_baked: None,
            gpu_profiler: None,
        })
    }

    /// Reconcile the offscreen targets + scene pipelines with the backbuffer size +
    /// the scene MSAA level. The scene MRT + depth carry the MSAA level (recreated on
    /// a size *or* sample-count change); the GTAO targets stay single-sample (size
    /// only); the MSAA-dependent scene pipelines rebuild on a sample-count change
    /// (their sample count is baked at creation). Steady-state frames allocate
    /// nothing. Shared by the 3D scene + UV viewport paths.
    fn sync_targets(&mut self, gpu: &Gpu, sample_count: u32) -> windows::core::Result<()> {
        let device = gpu.device();
        let (width, height) = gpu.size();
        let sample_count = sample_count.max(1);
        let size_changed = self.color.size() != (width, height);
        let samples_changed = self.scene_sample_count != sample_count;

        if size_changed || samples_changed {
            self.color = ColorTarget::hdr_msaa(device, width, height, sample_count)?;
            self.ambient = ColorTarget::hdr_msaa(device, width, height, sample_count)?;
            self.depth = DepthTarget::with_samples(device, width, height, sample_count)?;
        }
        if size_changed {
            self.gtao_gbuffer = ColorTarget::new(device, width, height)?;
            self.gtao_depth = DepthTarget::new(device, width, height)?;
            self.gtao_raw = ColorTarget::r8(device, width, height)?;
            self.gtao_blur = ColorTarget::r8(device, width, height)?;
        }
        if samples_changed {
            self.rebuild_scene_pipelines(device, sample_count)?;
            self.scene_sample_count = sample_count;
        }
        Ok(())
    }

    /// Rebuild the MSAA-dependent scene pipelines at `sample_count` (their sample
    /// count is baked into the rasterizer + must match the MSAA targets). The
    /// composite + GTAO pipelines are single-sample and untouched.
    fn rebuild_scene_pipelines(
        &mut self,
        device: &ID3D11Device,
        sample_count: u32,
    ) -> windows::core::Result<()> {
        let scene = build_scene_pipelines(device, sample_count)?;
        self.line_pipeline = scene.line;
        self.mesh_pipeline = scene.mesh;
        self.mesh_double_sided_pipeline = scene.mesh_double_sided;
        self.skybox_pipeline = scene.skybox;
        self.uv_fill_pipeline = scene.uv_fill;
        self.selection_pipeline = scene.selection;
        Ok(())
    }

    /// Render the scene: skybox + mesh (per-material PBR/IBL) + grid into the
    /// offscreen HDR MRT, then — when GTAO is on — a single-sample G-buffer +
    /// horizon occlusion + bilateral blur, and finally a composite (ambient-only AO
    /// darkening + tone map + sRGB) to the backbuffer. The egui chrome is drawn on
    /// top afterwards by `app`.
    // Independent per-frame inputs (gpu + model + revisions + materials + camera +
    // projection + environment + gtao + tonemap + debug + clear); none is redundant.
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
        gtao: GtaoSettings,
        tonemap: TonemapSettings,
        anti_aliasing: AntiAliasing,
        selection: SelectionView,
        debug: SceneDebugOptions,
        hidden_meshes: &[u32],
        clear: [f32; 4],
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let ctx = gpu.context();

        // Reconcile the offscreen targets + scene pipelines with the backbuffer size +
        // the scene MSAA level.
        self.sync_targets(gpu, anti_aliasing.effective_sample_count())?;

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

        // Build-on-demand / free-on-off for the derived 3D line overlays (invariant
        // 3): each view's buffer exists only while its toggle is on, rebuilt live
        // when its baked params (color / length / hidden set / scope) drift.
        self.sync_line_views(device, model, debug, hidden_meshes)?;

        // Build (or free) the selected-triangle draw list (the solo isolate list +
        // the highlight-flash fill source) and the per-mesh visibility filter, when
        // the selection / hidden set / model / mode drifts (invariant 3).
        self.sync_selection(
            device,
            model,
            model_revision,
            selection,
            hidden_meshes,
            debug.material_mode,
        )?;
        self.sync_visibility(
            device,
            model,
            model_revision,
            hidden_meshes,
            debug.material_mode,
        )?;

        // Reload the IBL maps when the chosen environment changes (a pure upload).
        if self.ibl.environment != environment.map {
            self.ibl = IblD3d::from_baked(gpu, environment.map)?;
        }

        let uniforms = scene_uniforms(camera, projection, environment, selection, debug);
        self.uniforms.update(ctx, &uniforms)?;

        // The buffer-inspection view bypasses lighting + the composite's tone
        // map/GTAO entirely (the scene shader emits final display pixels), so the
        // composite blits straight through and the GTAO passes are skipped.
        let buffer_view_active = debug.active_material == ActiveMaterial::Buffers;

        // GTAO runs only when enabled, a mesh is present, and we're not in the
        // flat buffer-inspection view; computed up front so it also drives the GPU
        // profiler's per-frame zone mask.
        let gtao_active = gtao.enabled && self.mesh.is_some() && !buffer_view_active;

        // Arm the GPU profiler (lazily, only under `--tracy` with a running client),
        // then open this frame's timing window. Absent on a normal launch, so the
        // passes below record no timestamps.
        if self.gpu_profiler.is_none() && gpu_profiler::should_enable() {
            self.gpu_profiler = GpuProfiler::new(device).ok();
        }
        if let Some(profiler) = self.gpu_profiler.as_mut() {
            profiler.begin_frame(ctx, gpu_profiler::frame_mask(gtao_active));
        }

        // --- Offscreen scene pass (2 MRT + depth). ---
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, clear);
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_begin(ctx, Zone::Scene);
        }
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

        // Mesh draw list, in precedence order: solo (isolate the selection) wins;
        // otherwise per-mesh visibility (the filtered list, present only while some
        // mesh is hidden); otherwise the whole mesh. All three share the mesh vertex
        // buffer, so only the index source + ranges differ. Wireframe shading draws
        // no filled surface.
        let solo = selection.solo && selection.selection.is_active();
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
            // Solo draws only the selection (empty → nothing); visible draws the
            // filtered list (None while active means every mesh is hidden → nothing);
            // otherwise the whole mesh.
            let draw_list: Option<(&IndexBuffer, &[MaterialDrawRange])> = if solo {
                self.selection_index
                    .as_ref()
                    .map(|index| (index, self.selection_ranges.as_slice()))
            } else if self.visible_active {
                self.visible_index
                    .as_ref()
                    .map(|index| (index, self.visible_ranges.as_slice()))
            } else {
                Some((&mesh.indices, mesh.ranges.as_slice()))
            };
            if let Some((index_buffer, ranges)) = draw_list {
                index_buffer.bind(ctx);
                for range in ranges {
                    self.materials.set_range(ctx, range.material)?;
                    gpu.draw_indexed_range(range.index_count, range.first_index);
                }
            }
        }

        if debug.show_grid {
            self.line_pipeline.bind(ctx);
            self.grid.bind(ctx);
            gpu.draw(self.grid.count());
        }

        // Derived line overlays (wireframe / bounding box / face+vertex normals),
        // drawn after the mesh + grid so they sit on top. All share the line
        // pipeline (depth-tested Reversed-Z `GreaterEqual`, no depth write — the
        // mesh pushed its surface back so coplanar edges win) + the scene uniforms
        // (`b0`, still bound). Each buffer is `None` while its view is off.
        let line_views = [
            &self.wireframe_buf,
            &self.bounding_box_buf,
            &self.face_normal_buf,
            &self.vertex_normal_buf,
        ];
        if line_views.iter().any(|view| view.is_some()) {
            self.line_pipeline.bind(ctx);
            for buffer in line_views.into_iter().flatten() {
                buffer.bind(ctx);
                gpu.draw(buffer.count());
            }
        }

        // Selection highlight flash: a flat bright-color fill redrawing the selected
        // triangles over the mesh, fading out after a selection change. Drawn last in
        // the scene pass so it sits on top. Reuses the selection index buffer over the
        // shared mesh vertex buffer; `fs_selection` tints it with the uniform
        // highlight color × the flash fade (`selection_color`, already in `b0`).
        // Skipped once the flash has faded, so the steady state pays nothing.
        let flash = selection.selection.is_active() && selection.fade > 0.0;
        if flash && let (Some(mesh), Some(index)) = (&self.mesh, &self.selection_index) {
            self.selection_pipeline.bind(ctx);
            mesh.vertices.bind(ctx);
            index.bind(ctx);
            gpu.draw_indexed_range(index.count(), 0);
        }
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_end(ctx, Zone::Scene);
        }

        // --- GTAO (ambient occlusion), only when enabled and a mesh is present. ---
        // A single-sample mesh-only G-buffer (view normal + Z), then the horizon
        // occlusion pass (→ raw `R8`) and the bilateral blur (→ blurred `R8`) the
        // composite darkens the ambient radiance by. The G-buffer has its own depth
        // (nearest-surface) and shares `b0` (the scene uniforms carry `view`); the
        // fullscreen passes read `b0` as the GTAO uniform + `s0` as the point sampler.
        if gtao_active {
            let gtao_uniforms = build_gtao_uniforms(camera, projection, gtao);
            self.gtao_uniforms.update(ctx, &gtao_uniforms)?;

            // G-buffer: redraw the whole mesh (material irrelevant) into the
            // single-sample normal/Z target, clearing the target + its depth.
            if let Some(profiler) = self.gpu_profiler.as_ref() {
                profiler.zone_begin(ctx, Zone::GtaoGbuffer);
            }
            gpu.begin_scene_pass(&[&self.gtao_gbuffer], &self.gtao_depth, [0.0; 4]);
            self.uniforms.bind_vs(ctx, 0);
            self.uniforms.bind_ps(ctx, 0);
            self.gtao_gbuffer_pipeline.bind(ctx);
            if let Some(mesh) = &self.mesh {
                mesh.vertices.bind(ctx);
                // Match the shaded mesh's visibility so a hidden mesh casts no AO;
                // solo is deliberately left out (as in the wgpu G-buffer), so only
                // the per-mesh hide filters the AO. `None` while active means every
                // mesh is hidden → nothing to occlude.
                let index = if self.visible_active {
                    self.visible_index.as_ref()
                } else {
                    Some(&mesh.indices)
                };
                if let Some(index) = index {
                    index.bind(ctx);
                    gpu.draw_indexed_range(index.count(), 0);
                }
            }
            if let Some(profiler) = self.gpu_profiler.as_ref() {
                profiler.zone_end(ctx, Zone::GtaoGbuffer);
            }

            // Occlusion: a fullscreen pass reading the G-buffer (`t0`) → raw AO.
            if let Some(profiler) = self.gpu_profiler.as_ref() {
                profiler.zone_begin(ctx, Zone::Gtao);
            }
            gpu.begin_color_pass(&self.gtao_raw);
            self.gtao_pipeline.bind(ctx);
            self.gtao_uniforms.bind_ps(ctx, 0);
            self.gtao_sampler.bind_ps(ctx, 0);
            self.gtao_gbuffer.bind_ps_srv(ctx, 0);
            gpu.draw(3);
            if let Some(profiler) = self.gpu_profiler.as_ref() {
                profiler.zone_end(ctx, Zone::Gtao);
            }

            // Bilateral blur: reads the G-buffer (`t0`) + raw AO (`t1`) → blurred AO.
            // `begin_color_pass` rebinds the RTV to `gtao_blur`, releasing `gtao_raw`
            // as a render target before it's bound below as an SRV.
            if let Some(profiler) = self.gpu_profiler.as_ref() {
                profiler.zone_begin(ctx, Zone::GtaoBlur);
            }
            gpu.begin_color_pass(&self.gtao_blur);
            self.gtao_blur_pipeline.bind(ctx);
            self.gtao_gbuffer.bind_ps_srv(ctx, 0);
            self.gtao_raw.bind_ps_srv(ctx, 1);
            gpu.draw(3);
            if let Some(profiler) = self.gpu_profiler.as_ref() {
                profiler.zone_end(ctx, Zone::GtaoBlur);
            }
            // Drop the G-buffer / raw SRVs before the composite binds the scene
            // targets (and before next frame rebinds them as render targets).
            gpu.unbind_ps_srvs(2);
        }

        // --- Composite to the backbuffer (ambient-only AO + tone map + sRGB). ---
        let post = PostUniforms {
            gtao_enabled: u32::from(gtao_active),
            tonemap_enabled: u32::from(tonemap.enabled),
            tonemap_op: tonemap.operator.shader_index(),
            passthrough: u32::from(buffer_view_active),
        };
        self.post_uniforms.update(ctx, &post)?;
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_begin(ctx, Zone::Composite);
        }
        gpu.begin_backbuffer_blit();
        // Resolve the MSAA scene MRT into the single-sample textures the composite
        // samples (a no-op at 1×). The scene RTVs are unbound now (the backbuffer is
        // the only bound target), so the multisample resolve source is free.
        self.color.resolve(ctx);
        self.ambient.resolve(ctx);
        self.composite_pipeline.bind(ctx);
        self.post_uniforms.bind_ps(ctx, 0);
        self.color.bind_ps_srv(ctx, 0);
        // t1 = the blurred GTAO when active; otherwise a harmless placeholder (the
        // shader ignores it when `gtao_enabled` is 0). t2 is the ambient radiance.
        // `self.sampler` (linear clamp) replaces the GTAO point sampler at `s0`.
        if gtao_active {
            self.gtao_blur.bind_ps_srv(ctx, 1);
        } else {
            self.ambient.bind_ps_srv(ctx, 1);
        }
        self.ambient.bind_ps_srv(ctx, 2);
        self.sampler.bind_ps(ctx, 0);
        gpu.draw(3);
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_end(ctx, Zone::Composite);
        }
        // Release the offscreen SRVs so next frame can bind them as render targets.
        gpu.unbind_ps_srvs(3);

        // Close the GPU profiler's timing window for this frame (resolves + reads back
        // a few frames later in `begin_frame`).
        if let Some(profiler) = self.gpu_profiler.as_mut() {
            profiler.end_frame(ctx);
        }

        Ok(())
    }

    /// Render the 2D UV viewport (instead of the 3D scene): the 0..1 grid, the
    /// optional island fill (Shaded / Islands modes), then the model's UV edges on
    /// top — all framed by the 2D `uv_camera` and composited like the 3D scene
    /// (tone-mapped, no GTAO). Faithful port of the wgpu `record_scene` UV path.
    // Independent per-frame inputs (gpu + model + revision + camera + channel +
    // shading mode + clear); none is redundant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_uv(
        &mut self,
        gpu: &Gpu,
        model: &ModelData,
        model_revision: u64,
        uv_camera: UvCamera,
        channel: u32,
        shading_mode: UvShadingMode,
        anti_aliasing: AntiAliasing,
        clear: [f32; 4],
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let ctx = gpu.context();

        self.sync_targets(gpu, anti_aliasing.effective_sample_count())?;
        self.sync_uv_view(device, model, model_revision, channel, shading_mode)?;

        // The UV camera's orthographic view-projection; the rest of the uniform is
        // unused by the UV path (lines/fill return their own vertex color).
        let uniforms = uv_scene_uniforms(uv_camera);
        self.uniforms.update(ctx, &uniforms)?;

        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, clear);
        self.uniforms.bind_vs(ctx, 0);
        self.uniforms.bind_ps(ctx, 0);
        // `fs_main` (the UV fill) samples the checker + every material slot at the top
        // (uniform control flow) before its zero-normal early-out, so all of group
        // 1/2/3 must be bound even though the UV draws never use the sampled values.
        self.checker_greyscale.bind_ps(ctx, 0);
        self.checker_sampler.bind_ps(ctx, 0);
        self.ibl.bind_ps(ctx);
        self.sampler.bind_ps(ctx, 1);
        self.materials.bind_shared(ctx);
        self.materials.bind_fallback(ctx)?;

        // Reference grid first.
        self.line_pipeline.bind(ctx);
        self.uv_grid.bind(ctx);
        gpu.draw(self.uv_grid.count());

        // Island fill (Shaded / Islands), under the wireframe.
        if let Some(fill) = &self.uv_fill_buf {
            self.uv_fill_pipeline.bind(ctx);
            fill.bind(ctx);
            gpu.draw(fill.count());
        }

        // The model's UV edges on top.
        if let Some(wireframe) = &self.uv_wireframe_buf {
            self.line_pipeline.bind(ctx);
            wireframe.bind(ctx);
            gpu.draw(wireframe.count());
        }

        // Composite to the backbuffer: tone-mapped (the default operator, so shaded
        // fills read like the 3D scene), no GTAO (the flat UV viewport has no depth
        // to occlude). `t1` binds the ambient as a harmless placeholder (the shader
        // ignores it when GTAO is off).
        let post = PostUniforms {
            gtao_enabled: 0,
            tonemap_enabled: 1,
            tonemap_op: 0,
            passthrough: 0,
        };
        self.post_uniforms.update(ctx, &post)?;
        gpu.begin_backbuffer_blit();
        // Resolve the MSAA scene MRT (no-op at 1×) before the composite samples it.
        self.color.resolve(ctx);
        self.ambient.resolve(ctx);
        self.composite_pipeline.bind(ctx);
        self.post_uniforms.bind_ps(ctx, 0);
        self.color.bind_ps_srv(ctx, 0);
        self.ambient.bind_ps_srv(ctx, 1);
        self.ambient.bind_ps_srv(ctx, 2);
        self.sampler.bind_ps(ctx, 0);
        gpu.draw(3);
        gpu.unbind_ps_srvs(3);

        Ok(())
    }

    /// Build-on-demand for the UV viewport's derived buffers (invariant 3): the
    /// wireframe is rebuilt only when the model / channel changes; the island fill
    /// when the model / channel / shading mode changes, and freed in Wire mode — so
    /// panning/zooming rebuilds nothing. Port of the wgpu `sync_uv_view`.
    fn sync_uv_view(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        channel: u32,
        shading_mode: UvShadingMode,
    ) -> windows::core::Result<()> {
        let want_wireframe = Some((model_revision, channel));
        if self.uv_wireframe_baked != want_wireframe {
            self.uv_wireframe_buf =
                optional_vertex_buffer(device, &uv_wireframe_lines(model, channel))?;
            self.uv_wireframe_baked = want_wireframe;
        }

        let want_fill = match shading_mode {
            UvShadingMode::Wire => None,
            UvShadingMode::Shaded | UvShadingMode::Islands => {
                Some((model_revision, channel, shading_mode))
            }
        };
        if self.uv_fill_baked != want_fill {
            let fill = match shading_mode {
                UvShadingMode::Wire => Vec::new(),
                UvShadingMode::Shaded => uv_fill_triangles(model, channel, false),
                UvShadingMode::Islands => uv_fill_triangles(model, channel, true),
            };
            self.uv_fill_buf = optional_vertex_buffer(device, &fill)?;
            self.uv_fill_baked = want_fill;
        }

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

    /// Build-on-demand / free-on-off for the three derived line views (invariant 3).
    /// A view's buffer is (re)built when its toggle is on and its baked params drift
    /// from the current options, and dropped to `None` when off. Unchanged views are
    /// left untouched. Faithful port of the wgpu `SceneResources::sync_line_views`.
    fn sync_line_views(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        debug: SceneDebugOptions,
        hidden_meshes: &[u32],
    ) -> windows::core::Result<()> {
        // Wireframe rebuilds when its color *or* the Outliner's hidden set drifts
        // (edges of a hidden mesh disappear with the mesh). On in both the wireframe
        // overlay and the wireframe-only shading mode.
        let wireframe_on =
            debug.wireframe_overlay || matches!(debug.shading_mode, ShadingMode::Wireframe);
        let want_wireframe = wireframe_on.then(|| (debug.wireframe_color, hidden_meshes.to_vec()));
        if self.wireframe_baked != want_wireframe {
            self.wireframe_buf = match &want_wireframe {
                Some((color, hidden)) => {
                    optional_vertex_buffer(device, &wireframe_lines(model, *color, hidden))?
                }
                None => None,
            };
            self.wireframe_baked = want_wireframe;
        }

        // Bounding box: only the inputs the chosen scope depends on go in the bake
        // key, so an unrelated change can't rebuild the box.
        let want_bounding_box = debug.show_bounding_box.then(|| {
            let scope = debug.bounding_box_scope;
            let hidden = match scope {
                BoundingBoxScope::VisibleOnly => hidden_meshes.to_vec(),
                _ => Vec::new(),
            };
            let selection = match scope {
                BoundingBoxScope::OnlySelection => debug.bounding_box_selection,
                _ => Selection::None,
            };
            BoundingBoxParams {
                color: debug.bounding_box_color,
                scope,
                hidden,
                selection,
            }
        });
        if self.bounding_box_baked != want_bounding_box {
            self.bounding_box_buf = match &want_bounding_box {
                Some(BoundingBoxParams {
                    color,
                    scope,
                    hidden,
                    selection,
                }) => {
                    let bounds = match scope {
                        BoundingBoxScope::AllMeshes => model.bounds,
                        BoundingBoxScope::OnlySelection => selection_bounds(model, *selection),
                        BoundingBoxScope::VisibleOnly => model.visible_bounds(hidden),
                    };
                    match bounds {
                        Some(bounds) => {
                            optional_vertex_buffer(device, &bounding_box_lines(bounds, *color))?
                        }
                        None => None,
                    }
                }
                None => None,
            };
            self.bounding_box_baked = want_bounding_box;
        }

        let want_face = debug.face_normals.then(|| NormalParams {
            length: debug.face_normal_length,
            color: debug.face_normal_color,
            hidden: hidden_meshes.to_vec(),
        });
        if self.face_baked != want_face {
            self.face_normal_buf = match &want_face {
                Some(NormalParams {
                    length,
                    color,
                    hidden,
                }) => optional_vertex_buffer(
                    device,
                    &face_normal_lines(model, *length, *color, hidden),
                )?,
                None => None,
            };
            self.face_baked = want_face;
        }

        let want_vertex = debug.vertex_normals.then(|| NormalParams {
            length: debug.vertex_normal_length,
            color: debug.vertex_normal_color,
            hidden: hidden_meshes.to_vec(),
        });
        if self.vertex_baked != want_vertex {
            self.vertex_normal_buf = match &want_vertex {
                Some(NormalParams {
                    length,
                    color,
                    hidden,
                }) => optional_vertex_buffer(
                    device,
                    &vertex_normal_lines(model, *length, *color, hidden),
                )?,
                None => None,
            };
            self.vertex_baked = want_vertex;
        }

        Ok(())
    }

    /// The per-triangle grouping key for `mode`: the cached mesh-part key in Unique
    /// mode (when the model carries per-triangle node info), else `None` to group by
    /// material slot. Mirrors the wgpu `grouping_key`; call after `sync_unique_parts`.
    fn grouping_key(&self, mode: MaterialMode) -> Option<&[u32]> {
        match mode {
            MaterialMode::Unique if !self.unique_part_key.is_empty() => Some(&self.unique_part_key),
            _ => None,
        }
    }

    /// Build (or free) the selected-triangle draw list (the solo isolate list + the
    /// flash fill source) when the selection / model / hidden set / mode drifts
    /// (invariant 3). The highlight color + flash fade ride in the uniform, so they
    /// never trigger a rebuild — only a change of *what* is selected does. Faithful
    /// port of the wgpu `sync_selection`.
    fn sync_selection(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        view: SelectionView,
        hidden: &[u32],
        mode: MaterialMode,
    ) -> windows::core::Result<()> {
        let want = view.selection.is_active().then(|| SelectionBaked {
            model_revision,
            selection: view.selection,
            hidden: hidden.to_vec(),
            mode,
        });
        if self.selection_baked == want {
            return Ok(());
        }
        // Resolve the visible selected triangles, grouped by the same key as the main
        // mesh so each range binds the right effective material. `None` (no usable
        // geometry) or an empty list both clear to a no-draw selection.
        let geometry = match &want {
            Some(SelectionBaked {
                selection, hidden, ..
            }) => {
                let key = self.grouping_key(mode);
                selection_geometry(model, *selection, hidden, key)
            }
            None => None,
        };
        match geometry {
            Some((indices, ranges)) if !indices.is_empty() => {
                self.selection_index = Some(IndexBuffer::new(device, &indices)?);
                self.selection_ranges = ranges;
            }
            _ => {
                self.selection_index = None;
                self.selection_ranges = Vec::new();
            }
        }
        self.selection_baked = want;
        Ok(())
    }

    /// Build (or free) the per-mesh visibility draw list when the hidden set / model /
    /// mode drifts (invariant 3). `hidden` empty means nothing is hidden (full mesh,
    /// no filter). When every mesh is hidden the filter is active but the list empty
    /// (draw nothing); when the model carries no per-triangle node info the filter is
    /// off (full mesh). Faithful port of the wgpu `sync_visibility`.
    fn sync_visibility(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        hidden: &[u32],
        mode: MaterialMode,
    ) -> windows::core::Result<()> {
        let want = (!hidden.is_empty()).then(|| VisibilityBaked {
            model_revision,
            hidden: hidden.to_vec(),
            mode,
        });
        if self.visibility_baked == want {
            return Ok(());
        }
        let geometry = match &want {
            Some(VisibilityBaked { hidden, .. }) => {
                let key = self.grouping_key(mode);
                visible_geometry(model, hidden, key)
            }
            None => None,
        };
        match geometry {
            // Some unhidden geometry: draw the filtered list.
            Some((indices, ranges)) if !indices.is_empty() => {
                self.visible_index = Some(IndexBuffer::new(device, &indices)?);
                self.visible_ranges = ranges;
                self.visible_active = true;
            }
            // Every mesh hidden: the filter is active but draws nothing.
            Some(_) => {
                self.visible_index = None;
                self.visible_ranges = Vec::new();
                self.visible_active = true;
            }
            // Nothing hidden, or the model carries no per-triangle node info: draw the
            // full mesh (no filter).
            None => {
                self.visible_index = None;
                self.visible_ranges = Vec::new();
                self.visible_active = false;
            }
        }
        self.visibility_baked = want;
        Ok(())
    }
}

/// The six MSAA-dependent scene pipelines (line / mesh / double-sided mesh / skybox /
/// UV fill / selection fill). They all draw into the MSAA scene MRT, so their sample
/// count is baked at the live AA level; `SceneGpu::new` + `rebuild_scene_pipelines`
/// build them together via this helper so they stay in lockstep with the targets.
struct ScenePipelineSet {
    line: Pipeline,
    mesh: Pipeline,
    mesh_double_sided: Pipeline,
    skybox: Pipeline,
    uv_fill: Pipeline,
    selection: Pipeline,
}

fn build_scene_pipelines(
    device: &ID3D11Device,
    sample_count: u32,
) -> windows::core::Result<ScenePipelineSet> {
    // Line overlays (grid / wireframe / bounding box / normals): depth-tested
    // (Reversed-Z `GreaterEqual`) but neither writing nor biasing depth, alpha-blended.
    let line = Pipeline::new(
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
            sample_count,
        },
    )?;

    // Mesh: back-face culled, writes depth + pushes the surface back (slope-scaled
    // bias) so coplanar line overlays win the test. The double-sided variant (Backface
    // Rendering) is identical but unculled.
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
        sample_count,
    };
    let mesh = Pipeline::new(device, &mesh_desc(Cull::Back))?;
    let mesh_double_sided = Pipeline::new(device, &mesh_desc(Cull::None))?;

    // Skybox: a fullscreen triangle drawn first behind geometry — depth-test always,
    // no write, opaque overwrite.
    let skybox = Pipeline::new(
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
            sample_count,
        },
    )?;

    // UV island fill: `fs_main` (zero-normal fill verts take its overlay branch);
    // never writes depth (everything at z=0) so grid + wireframe layer by draw order.
    let uv_fill = Pipeline::new(
        device,
        &PipelineDesc {
            vs: SCENE_VS,
            ps: SCENE_MESH_PS,
            input: &SCENE_VERTEX_LAYOUT,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: DepthState {
                test: true,
                write: false,
                compare: DepthCompare::GreaterEqual,
            },
            blend: BlendMode::AlphaBlend,
            depth_bias: DepthBias::default(),
            sample_count,
        },
    )?;

    // Selection flash: `fs_selection` flat fill, depth-tested (Reversed-Z) but no
    // depth write, so it's occluded by geometry in front yet wins over the coplanar
    // surface it tints.
    let selection = Pipeline::new(
        device,
        &PipelineDesc {
            vs: SCENE_VS,
            ps: SCENE_SELECTION_PS,
            input: &SCENE_VERTEX_LAYOUT,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: DepthState {
                test: true,
                write: false,
                compare: DepthCompare::GreaterEqual,
            },
            blend: BlendMode::AlphaBlend,
            depth_bias: DepthBias::default(),
            sample_count,
        },
    )?;

    Ok(ScenePipelineSet {
        line,
        mesh,
        mesh_double_sided,
        skybox,
        uv_fill,
        selection,
    })
}

/// Build an optional vertex buffer from `vertices`: `None` for an empty set (D3D11
/// rejects a zero-byte buffer, and the draw is skipped anyway), else an immutable
/// [`VertexBuffer`]. The build-on-demand line views + the UV wireframe/fill use this
/// so an off / empty view holds no allocation.
fn optional_vertex_buffer(
    device: &ID3D11Device,
    vertices: &[crate::scene::SceneVertex],
) -> windows::core::Result<Option<VertexBuffer>> {
    if vertices.is_empty() {
        Ok(None)
    } else {
        Ok(Some(VertexBuffer::new(device, vertices)?))
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

/// Build the per-frame [`SceneUniforms`] from the camera, projection, environment,
/// selection and debug options. Mirrors the wgpu `update_camera`: the selection
/// flash rides in `selection_color` (gamma-space rgb + the flash fade in alpha,
/// zero while nothing is selected/flashing), read only by `fs_selection`.
fn scene_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    environment: EnvironmentSettings,
    selection: SelectionView,
    debug: SceneDebugOptions,
) -> SceneUniforms {
    let view_projection = camera.view_projection(projection);
    let selection_color = if selection.selection.is_active() {
        let [r, g, b, _] = selection.highlight_color;
        [r, g, b, selection.fade.clamp(0.0, 1.0)]
    } else {
        [0.0; 4]
    };
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
            // `z` carries the active buffer-inspection view index (-1 when off).
            buffer_view_value(debug),
            0.0,
        ],
        view: camera.view_matrix().to_cols_array_2d(),
        selection_color,
    }
}

/// Build the [`SceneUniforms`] for the 2D UV viewport: only the UV camera's
/// orthographic view-projection matters (the grid / wireframe / fill return their
/// own vertex color, never reaching the IBL / shading / selection code). Mirrors the
/// wgpu `update_camera_uv`. `projection_params.x = 1.0` marks orthographic.
fn uv_scene_uniforms(uv_camera: UvCamera) -> SceneUniforms {
    SceneUniforms {
        view_projection: uv_camera.view_projection().to_cols_array_2d(),
        inv_view_projection: [[0.0; 4]; 4],
        render_options: [0.0; 4],
        camera_position: [0.0; 4],
        env_params: [0.0; 4],
        projection_params: [1.0, 0.0, 0.0, 0.0],
        view: [[0.0; 4]; 4],
        selection_color: [0.0; 4],
    }
}

/// Build the per-frame [`GtaoUniforms`] from the camera, projection and GTAO
/// settings. Mirrors the wgpu `GtaoPass::update`: the settings' `radius` is a
/// fraction of the framed model's bounding-sphere radius, so it's scaled into view
/// units by the live `scene_radius` here, keeping the AO look scale-invariant.
fn build_gtao_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    gtao: GtaoSettings,
) -> GtaoUniforms {
    let scene_radius = camera.scene_radius.max(1e-3);
    let (slices, steps) = gtao.quality.slices_steps();
    GtaoUniforms {
        proj: camera.projection_matrix(projection).to_cols_array_2d(),
        params: [
            (gtao.radius * scene_radius).max(1e-4),
            gtao.intensity.max(0.0),
            gtao.thickness.clamp(0.0, 1.0),
            0.0,
        ],
        config: [
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            slices.max(1) as f32,
            steps.max(1) as f32,
            0.0,
        ],
    }
}
