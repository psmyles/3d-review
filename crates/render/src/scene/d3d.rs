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

use review_model::ModelData;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_mesh, model_pivot, pivot_half_extent, pivot_lines,
    scene_lines, selection_geometry, skeleton_fill_triangles, skeleton_lines, skin_weight_vertices,
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
    EnvironmentSettings, GhostStyle, GtaoSettings, MaterialMode, OptSceneFrame, OptView,
    OrbitCamera, ProcessedModelRef, SceneDebugOptions, SceneFrame, ShadingMode, TonemapSettings,
    UvCamera, UvShadingMode, ViewportBackground,
};

use super::gpu_types::{
    GtaoUniforms, PostUniforms, SceneUniforms, buffer_view_value, shading_mode_value,
    skin_weight_value, vertex_color_value,
};
use crate::rhi::gpu_profiler::{self, GpuProfiler, Zone};

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

/// Expand a [`ViewportBackground`]'s display-space top/bottom colors into the
/// `bg_top` / `bg_bottom` cbuffer fields (`xyz` color, `w` unused).
fn background_uniforms(background: ViewportBackground) -> ([f32; 4], [f32; 4]) {
    let (top, bottom) = background.gradient_srgb();
    (
        [top[0], top[1], top[2], 0.0],
        [bottom[0], bottom[1], bottom[2], 0.0],
    )
}

/// Build the composite pass's [`PostUniforms`] from the live settings.
fn post_uniforms(
    background: ViewportBackground,
    gtao_active: bool,
    tonemap: TonemapSettings,
    passthrough: bool,
) -> PostUniforms {
    let (bg_top, bg_bottom) = background_uniforms(background);
    PostUniforms {
        gtao_enabled: u32::from(gtao_active),
        tonemap_enabled: u32::from(tonemap.enabled),
        tonemap_op: tonemap.operator.shader_index(),
        passthrough: u32::from(passthrough),
        bg_top,
        bg_bottom,
    }
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

/// Baked parameters for the pivot marker: the pivot position + half-length, both
/// derived from the model, so a model swap (new pivot / size) rebuilds it while an
/// unrelated change does not.
#[derive(PartialEq)]
struct PivotParams {
    pivot: [f32; 3],
    half: f32,
}

/// Opacity of the skeleton's solid octahedron fills, as a multiplier on the bone
/// color's own alpha. Low enough that the character reads through the rig, high
/// enough that a bone's volume and orientation are legible.
const SKELETON_FILL_ALPHA: f32 = 0.35;

/// Baked parameters for the skeleton overlay. The selected set is part of the key
/// because the highlight color is baked per bone into the vertex buffer (unlike
/// the mesh selection flash, whose color rides in a uniform) — a skeleton is a few
/// thousand vertices, so rebuilding it on an Outliner click is far cheaper than
/// carrying a per-bone lookup into the shader.
#[derive(PartialEq)]
struct SkeletonParams {
    model_revision: u64,
    selected: Vec<u32>,
    scale: f32,
    color: [f32; 4],
    selected_color: [f32; 4],
}

/// Bake key for the skin-weight heat map. Only the model and the selected bone
/// set change its bytes — the ramp is fixed, and the neutral base it blends from
/// is a shader constant.
#[derive(PartialEq)]
struct SkinWeightParams {
    model_revision: u64,
    selected: Vec<u32>,
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

/// The build-on-demand derived views (invariant 3): each buffer exists only while
/// its toggle is on, paired with the bake key it was last built for. The 3D line
/// views draw with the shared `line_pipeline` (the pivot marker with the
/// always-on-top `line_overlay_pipeline`); the UV pair belongs to the 2D UV
/// viewport.
#[derive(Default)]
struct DerivedViews {
    /// Model wireframe (original-polygon edges); the key is `(color, hidden)`.
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
    /// 3-axis pivot marker at the model's origin; `None` while off. The bake key
    /// is the pivot position + half-length (both model-derived).
    pivot_buf: Option<VertexBuffer>,
    pivot_baked: Option<PivotParams>,
    /// Vertex buffer parallel to the mesh's own, colored by the selected bones'
    /// influence, drawn *in place of* the mesh while the Skin Weights material is
    /// active. `None` in every other mode (invariant 3).
    weights_buf: Option<VertexBuffer>,
    weights_baked: Option<SkinWeightParams>,
    /// The skeleton overlay's solid octahedron fills and their outlines; both
    /// `None` while the toggle is off. Drawn always-on-top (X-ray).
    skeleton_fill_buf: Option<VertexBuffer>,
    skeleton_line_buf: Option<VertexBuffer>,
    skeleton_baked: Option<SkeletonParams>,
    /// The model's UV edges for the active channel; built per
    /// `(model_revision, channel)`.
    uv_wireframe_buf: Option<VertexBuffer>,
    uv_wireframe_baked: Option<(u64, u32)>,
    /// The UV island fill (Shaded / Islands modes only); `None` in Wire mode.
    /// Built per `(model_revision, channel, shading_mode)`.
    uv_fill_buf: Option<VertexBuffer>,
    uv_fill_baked: Option<(u64, u32, UvShadingMode)>,
}

/// A rectangle of the backbuffer for the composite to write into. The whole
/// backbuffer for a single view; one half of it for each side of the Opt
/// workspace's split.
#[derive(Debug, Clone, Copy)]
struct BackbufferRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl BackbufferRect {
    fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// Which model a [`ModelSlot`] holds. The Opt workspace draws a source mesh and a
/// processed one in the same frame; every other workspace only ever uses
/// [`SlotId::Source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotId {
    Source,
    Processed,
}

/// Everything cached *per model*: the uploaded mesh, the derived views, and the
/// selection / visibility draw lists, each with the bake key it was built for.
///
/// This exists so two models can be resident at once. Rendering the Opt
/// workspace's split view alternates between them within a single frame, and a
/// single-slot cache would rebuild both meshes on every one of those alternations
/// — tens of megabytes of vertex data per frame for a real asset. Because every
/// bake key travels inside the slot, swapping slots is `mem::swap` and nothing
/// more: no GPU work, no reallocation, and each model's caches stay valid.
#[derive(Default)]
struct ModelSlot {
    /// `None` for an empty model (the grid still draws).
    mesh: Option<MeshBuffers>,
    mesh_revision: u64,
    mesh_uv_channel: u32,
    mesh_material_mode: MaterialMode,
    /// Cached Unique-mode per-triangle mesh-part key + count, baked by model
    /// revision while Unique is active (invariant 3: freed otherwise).
    unique_part_key: Vec<u32>,
    unique_part_count: usize,
    unique_baked: Option<u64>,
    /// The build-on-demand derived line/fill views (invariant 3) — each buffer
    /// exists only while its toggle is on, paired with the bake key it was last
    /// built for.
    views: DerivedViews,
    /// The model revision `views` were derived from.
    ///
    /// Their own bake keys describe only the *parameters* they were built with —
    /// a color, a hidden set, a normal length — and say nothing about which mesh
    /// the geometry came from. So the revision is tracked here and the whole set
    /// dropped when it moves: otherwise a mesh replaced underneath them (a newly
    /// loaded file, or the Opt workspace reprocessing) keeps the previous mesh's
    /// wireframe and normal lines drawn over the new one.
    views_revision: u64,
    /// The selected triangles reordered per-material over a fresh index buffer that
    /// shares the mesh vertex buffer (the solo isolate list + the flash fill source).
    /// `None` while nothing is selected or the selection resolves to no geometry
    /// (invariant 3).
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
    /// This model's wireframe as drawn when it is the *ghost* in the Opt
    /// workspace's overlay view. Deliberately separate from `views.wireframe_buf`
    /// rather than reusing it: that one is owned by the user's wireframe toggle
    /// and coloured by it, so sharing would have the two rebuild the same buffer
    /// in opposite directions every frame. `None` whenever this model is not
    /// currently the ghost (invariant 3).
    ghost_wireframe_buf: Option<VertexBuffer>,
    ghost_wireframe_baked: Option<(u64, Vec<u32>, [f32; 4])>,
}

impl ModelSlot {
    fn new() -> Self {
        Self {
            // A sentinel distinct from any real revision, so the first sync builds
            // the mesh (or leaves it `None` for an empty model).
            mesh_revision: u64::MAX,
            views_revision: u64::MAX,
            ..Self::default()
        }
    }

    /// Whether this slot has a mesh uploaded — `mesh_revision` alone can't say,
    /// since an empty model leaves the buffers `None` at a real revision.
    fn has_mesh(&self) -> bool {
        self.mesh.is_some()
    }

    /// Drop every cached buffer and bake key, so the next sync rebuilds from
    /// scratch. Used when a slot's model goes away — the GPU resources are
    /// released by the drop, no explicit teardown needed (invariant 3).
    fn release(&mut self) {
        *self = ModelSlot::new();
    }
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
    /// Always-on-top line variant (depth compare `Always`, no write): used by the
    /// pivot marker so it shows through the mesh rather than being occluded.
    line_overlay_pipeline: Pipeline,
    /// Always-on-top triangle fill, for the skeleton overlay's octahedra.
    fill_overlay_pipeline: Pipeline,
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
    /// The last *requested* AA level, before capability clamping — cached so the
    /// (cheap but per-frame) `CheckMultisampleQualityLevels` re-clamp only runs
    /// when the request actually changes.
    requested_sample_count: u32,
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
    /// Everything cached for the model currently being drawn.
    active: ModelSlot,
    /// The *other* model's cache, held so the Opt workspace can show two meshes
    /// without rebuilding either one every frame. See [`SceneGpu::activate`].
    idle: ModelSlot,
    /// Which model `active` currently holds.
    active_slot: SlotId,
    /// Selection-flash fill pipeline (`fs_selection`): flat highlight color × fade,
    /// depth-tested (Reversed-Z `GreaterEqual`) but no depth write, alpha-blended.
    selection_pipeline: Pipeline,
    // --- 2D UV viewport (drawn instead of the 3D scene in UV workspace mode). ---
    /// The static 0..1 reference grid (built once; model-independent).
    uv_grid: VertexBuffer,
    /// Flat-color triangle fill for the UV islands (`fs_main`'s zero-normal overlay
    /// branch — same as the line views but filled). No depth write; alpha-blended.
    uv_fill_pipeline: Pipeline,
    /// Hand-rolled D3D11 timestamp-query → Tracy GPU profiler. `None` unless `--tracy`
    /// armed it and a Tracy client is running; built lazily on the first profiled
    /// frame and never touched otherwise (every scene pass records no timestamps).
    gpu_profiler: Option<GpuProfiler>,
    /// Building the profiler failed once — don't re-attempt its `RING × (SLOTS+1)`
    /// `CreateQuery` calls every frame on a device that keeps refusing them.
    gpu_profiler_failed: bool,
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
        // Capability-gate the initial level too (invariant 4): a persisted AA
        // setting restored on a weaker adapter degrades instead of failing.
        let requested_sample_count = sample_count.max(1);
        let sample_count = gpu.clamp_msaa(requested_sample_count);

        let uniforms = DynamicConstantBuffer::new::<SceneUniforms>(device)?;
        let post_uniforms = DynamicConstantBuffer::new::<PostUniforms>(device)?;
        let gtao_uniforms = DynamicConstantBuffer::new::<GtaoUniforms>(device)?;

        // The MSAA-dependent scene pipelines (line / mesh / double-sided mesh /
        // skybox / UV fill / selection fill) — all draw into the MSAA scene MRT, so
        // their sample count is baked at the live AA level and rebuilt when it changes
        // ([`Self::rebuild_scene_pipelines`]).
        let scene = build_scene_pipelines(device, sample_count)?;

        // The composite: a fullscreen triangle, opaque overwrite of the backbuffer.
        // Always single-sample — it draws to the backbuffer, not the MSAA MRT.
        let composite_pipeline = Pipeline::new(
            device,
            &PipelineDesc::fullscreen(POST_VS, POST_PS, BlendMode::Opaque),
        )?;

        // GTAO G-buffer: a mesh-only pass writing the single-sample view normal/Z.
        // No culling (backfaces still occlude), writes + tests depth (Reversed-Z
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

        // The two GTAO fullscreen passes (occlusion + bilateral blur): opaque
        // writes into the `R8` AO targets, differing only in the PS.
        let gtao_pipeline = Pipeline::new(
            device,
            &PipelineDesc::fullscreen(GTAO_VS, GTAO_PS, BlendMode::Opaque),
        )?;
        let gtao_blur_pipeline = Pipeline::new(
            device,
            &PipelineDesc::fullscreen(GTAO_VS, GTAO_BLUR_PS, BlendMode::Opaque),
        )?;

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
            line_overlay_pipeline: scene.line_overlay,
            fill_overlay_pipeline: scene.fill_overlay,
            mesh_pipeline: scene.mesh,
            mesh_double_sided_pipeline: scene.mesh_double_sided,
            skybox_pipeline: scene.skybox,
            composite_pipeline,
            gtao_gbuffer_pipeline,
            gtao_pipeline,
            gtao_blur_pipeline,
            scene_sample_count: sample_count,
            requested_sample_count,
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
            active: ModelSlot::new(),
            idle: ModelSlot::new(),
            active_slot: SlotId::Source,
            selection_pipeline: scene.selection,
            uv_grid,
            uv_fill_pipeline: scene.uv_fill,
            gpu_profiler: None,
            gpu_profiler_failed: false,
        })
    }

    /// Record a profiled zone's begin timestamp (a no-op without `--tracy`).
    fn zone_begin(&self, ctx: &ID3D11DeviceContext, zone: Zone) {
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_begin(ctx, zone);
        }
    }

    /// Record a profiled zone's end timestamp (a no-op without `--tracy`).
    fn zone_end(&self, ctx: &ID3D11DeviceContext, zone: Zone) {
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_end(ctx, zone);
        }
    }

    /// Reconcile the offscreen targets + scene pipelines with the backbuffer size +
    /// the scene MSAA level. The scene MRT + depth carry the MSAA level (recreated on
    /// a size *or* sample-count change); the GTAO targets stay single-sample (size
    /// only); the MSAA-dependent scene pipelines rebuild on a sample-count change
    /// (their sample count is baked at creation). Steady-state frames allocate
    /// nothing. Shared by the 3D scene + UV viewport paths.
    /// `size` is the offscreen resolution to render at, which is *not* always the
    /// backbuffer's: the Opt workspace's split view renders each half at half
    /// width so the composite maps its target onto its half of the backbuffer
    /// one-to-one instead of squashing a full-width image into it.
    fn sync_targets(
        &mut self,
        gpu: &Gpu,
        sample_count: u32,
        size: (u32, u32),
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let (width, height) = (size.0.max(1), size.1.max(1));
        // Capability-clamp a *changed* request (invariant 4: an unsupported level
        // — e.g. restored settings on a weaker adapter — degrades to the nearest
        // supported one rather than failing target creation every frame).
        let requested = sample_count.max(1);
        let sample_count = if requested == self.requested_sample_count {
            self.scene_sample_count
        } else {
            self.requested_sample_count = requested;
            gpu.clamp_msaa(requested)
        };
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
        self.line_overlay_pipeline = scene.line_overlay;
        self.fill_overlay_pipeline = scene.fill_overlay;
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
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        camera: OrbitCamera,
    ) -> windows::core::Result<()> {
        // Every workspace but Opt draws the source model, and Opt may have left
        // the processed slot active. Without this the source mesh would be synced
        // *into* the processed slot — rebuilding both, and discarding the
        // processed cache on every switch between workspaces.
        self.activate(SlotId::Source);

        let size = gpu.size();
        self.sync_frame(gpu, frame, material_states, material_revision, size)?;

        let gtao_active = self.gtao_active(frame);
        self.begin_gpu_frame(gpu, gtao_active);
        let result = self.record_view(gpu, frame, camera, gtao_active, BackbufferRect::full(size));
        self.end_gpu_frame(gpu);
        result
    }

    /// Render the Opt workspace: the source and processed meshes side by side, or
    /// one over the other with the second drawn as a ghost.
    ///
    /// The split renders each half through the *same* offscreen targets, sized to
    /// half the backbuffer, compositing each into its own half — so the two views
    /// cost no extra target memory. What they do need is for both meshes to stay
    /// uploaded across the swap between them, which is what [`ModelSlot`] is for.
    pub(crate) fn render_opt(
        &mut self,
        gpu: &Gpu,
        frame: &OptSceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
    ) -> windows::core::Result<()> {
        let size = gpu.size();

        match frame.view {
            // The overlay needs two meshes to have anything to overlay; without a
            // processed one it is simply the 3D scene.
            OptView::Overlay { ghost, swap, tint } => match frame.processed {
                Some(processed) => self.render_overlay(
                    gpu,
                    frame,
                    processed,
                    material_states,
                    material_revision,
                    ghost,
                    swap,
                    tint,
                    size,
                ),
                None => {
                    self.activate(SlotId::Source);
                    self.release_ghost_wireframes();
                    self.render(
                        gpu,
                        &frame.base,
                        material_states,
                        material_revision,
                        frame.source_camera,
                    )
                }
            },
            // The split always draws two views. With nothing processed yet both
            // show the source: the layout the user picked is the layout they get,
            // and the right-hand view fills in as soon as a run lands.
            OptView::Split => self.render_split(gpu, frame, material_states, material_revision),
        }
    }

    /// The side-by-side comparison: two views of the same scene, laid out inside
    /// the chrome-free viewport so the divider falls where the user sees it fall.
    fn render_split(
        &mut self,
        gpu: &Gpu,
        frame: &OptSceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
    ) -> windows::core::Result<()> {
        // No ghost is drawn in the split view, so its buffers go (invariant 3).
        self.release_ghost_wireframes();

        // The two composites below cover only the chrome-free rect, and a
        // flip-model swapchain hands back a backbuffer whose contents are
        // undefined — so define them. The chrome is drawn over this, and painting
        // it black costs one clear rather than a third composite.
        gpu.clear_backbuffer([0.0, 0.0, 0.0, 1.0]);

        let view = frame.viewport;
        let height = view.height.max(1);
        // Each half renders at half the viewport's width. An odd width gives the
        // right half the leftover column so no strip is left unpainted; the
        // sub-pixel stretch that implies is invisible, whereas an unpainted
        // column showing the previous frame is not.
        let half = (view.width / 2).max(1);
        let target = (half, height);
        // Each view is half as wide as the area it is drawn into, so its camera's
        // aspect has to say so — `app` set it from the whole window, which would
        // stretch both halves horizontally. The renderer owns the split geometry,
        // so it owns this correction.
        let aspect = half as f32 / height as f32;
        // Both views share one camera unless the user has unlinked them, and the
        // aspect correction is the same for both: the two halves are the same
        // size, so anything that differs between them is a difference in the
        // mesh, which is the entire point of the comparison.
        let mut source_camera = frame.source_camera;
        source_camera.aspect_ratio = aspect;
        let mut processed_camera = frame.processed_camera;
        processed_camera.aspect_ratio = aspect;

        self.activate(SlotId::Source);
        self.sync_frame(gpu, &frame.base, material_states, material_revision, target)?;
        let source_gtao = self.gtao_active(&frame.base);
        // One profiler frame spans both halves. Its per-zone timestamp slots are
        // fixed, so the second half's overwrite the first's and the reported
        // scene/composite times describe the right-hand view — accurate for what
        // it measures, just not the whole frame.
        self.begin_gpu_frame(gpu, source_gtao);
        // Deliberately not `?`: an error here must still close the profiler frame
        // below, or its ring slot stays pending forever.
        let left = self.record_view(
            gpu,
            &frame.base,
            source_camera,
            source_gtao,
            BackbufferRect {
                x: view.x,
                y: view.y,
                width: half,
                height,
            },
        );

        let right_rect = BackbufferRect {
            x: view.x + half,
            y: view.y,
            width: view.width.saturating_sub(half).max(1),
            height,
        };
        let right = match frame.processed {
            Some(processed) => {
                let processed_frame = frame.base.with_model(processed.model, processed.revision);
                self.activate(SlotId::Processed);
                let synced = self.sync_frame(
                    gpu,
                    &processed_frame,
                    material_states,
                    material_revision,
                    target,
                );
                let processed_gtao = self.gtao_active(&processed_frame);
                synced.and_then(|()| {
                    self.record_view(
                        gpu,
                        &processed_frame,
                        processed_camera,
                        processed_gtao,
                        right_rect,
                    )
                })
            }
            // Nothing processed: the right half is the same scene again, drawn
            // from the slot already active — no swap, no second upload.
            None => self.record_view(gpu, &frame.base, processed_camera, source_gtao, right_rect),
        };
        self.end_gpu_frame(gpu);
        left.and(right)
    }

    /// The single-view comparison: one mesh shaded, the other over it as a ghost.
    ///
    /// Both meshes are drawn in the same pass so they occlude each other properly,
    /// which is the whole point — a silhouette that has moved shows up as ghost
    /// spilling past the solid surface.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; grouping these into a struct would only move the list"
    )]
    fn render_overlay(
        &mut self,
        gpu: &Gpu,
        frame: &OptSceneFrame<'_>,
        processed: ProcessedModelRef<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        ghost: GhostStyle,
        swap: bool,
        tint: [f32; 3],
        size: (u32, u32),
    ) -> windows::core::Result<()> {
        let processed_frame = frame.base.with_model(processed.model, processed.revision);
        // `swap` decides which mesh reads as solid; the other becomes the ghost.
        let (solid_slot, solid_frame, ghost_slot, ghost_frame) = if swap {
            (
                SlotId::Source,
                &frame.base,
                SlotId::Processed,
                &processed_frame,
            )
        } else {
            (
                SlotId::Processed,
                &processed_frame,
                SlotId::Source,
                &frame.base,
            )
        };

        // Build the ghost's buffers first, then leave the solid slot active so the
        // scene pass draws it normally. The ghost is drawn from the idle slot,
        // which is only safe because a slot owns its own buffers outright.
        self.activate(ghost_slot);
        self.sync_frame(gpu, ghost_frame, material_states, material_revision, size)?;
        if ghost == GhostStyle::Wireframe {
            self.sync_ghost_wireframe(gpu.device(), ghost_frame, tint)?;
        } else {
            self.release_ghost_wireframes();
        }

        self.activate(solid_slot);
        self.sync_frame(gpu, solid_frame, material_states, material_revision, size)?;

        let gtao_active = self.gtao_active(solid_frame);
        self.begin_gpu_frame(gpu, gtao_active);
        let result = self.record_view_with_ghost(
            gpu,
            solid_frame,
            frame.source_camera,
            gtao_active,
            BackbufferRect::full(size),
            Some((ghost, tint)),
        );
        self.end_gpu_frame(gpu);
        result
    }

    /// Whether GTAO should run for this frame: enabled, a mesh is present, and the
    /// view isn't one of the flat data-inspection ones.
    fn gtao_active(&self, frame: &SceneFrame<'_>) -> bool {
        frame.gtao.enabled && self.active.has_mesh() && !flat_display(frame)
    }

    /// Arm the GPU profiler (lazily, only under `--tracy` with a running client)
    /// and open this frame's timing window. Absent on a normal launch, so the
    /// passes record no timestamps. A failed build is reported once and never
    /// retried (44 `CreateQuery` calls per frame otherwise).
    fn begin_gpu_frame(&mut self, gpu: &Gpu, gtao_active: bool) {
        if self.gpu_profiler.is_none() && !self.gpu_profiler_failed && gpu_profiler::should_enable()
        {
            match GpuProfiler::new(gpu.device()) {
                Ok(profiler) => self.gpu_profiler = Some(profiler),
                Err(error) => {
                    self.gpu_profiler_failed = true;
                    gpu_profiler::note(&format!("GPU profiler unavailable: {error}"));
                }
            }
        }
        if let Some(profiler) = self.gpu_profiler.as_mut() {
            profiler.begin_frame(gpu.context(), gpu_profiler::frame_mask(gtao_active));
        }
    }

    /// Close the GPU profiler's timing window (resolves + reads back a few frames
    /// later in `begin_frame`).
    fn end_gpu_frame(&mut self, gpu: &Gpu) {
        if let Some(profiler) = self.gpu_profiler.as_mut() {
            profiler.end_frame(gpu.context());
        }
    }

    /// Draw the active slot's model into `dest`: the scene pass, GTAO when active,
    /// then the composite. Assumes [`Self::sync_frame`] has already run for this
    /// model.
    fn record_view(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        camera: OrbitCamera,
        gtao_active: bool,
        dest: BackbufferRect,
    ) -> windows::core::Result<()> {
        self.record_view_with_ghost(gpu, frame, camera, gtao_active, dest, None)
    }

    /// [`Self::record_view`], optionally drawing the *idle* slot's mesh as a ghost
    /// inside the same scene pass.
    fn record_view_with_ghost(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        camera: OrbitCamera,
        gtao_active: bool,
        dest: BackbufferRect,
        ghost: Option<(GhostStyle, [f32; 3])>,
    ) -> windows::core::Result<()> {
        let ctx = gpu.context();

        let uniforms = scene_uniforms(
            camera,
            frame.projection,
            frame.environment,
            frame.selection,
            frame.debug,
        );
        self.uniforms.update(ctx, &uniforms)?;

        self.record_scene_pass(gpu, frame)?;
        if let Some((style, tint)) = ghost {
            self.record_ghost(gpu, camera, frame, style, tint)?;
        }

        if gtao_active {
            self.record_gtao(gpu, camera, frame.projection, frame.gtao)?;
        }

        // Composite to the backbuffer (ambient-only AO + tone map + sRGB, over the
        // viewport background). `t1` is the blurred GTAO when active; otherwise a
        // harmless placeholder (the shader ignores it when `gtao_enabled` is 0).
        let post = post_uniforms(
            frame.background,
            gtao_active,
            frame.tonemap,
            flat_display(frame),
        );
        self.zone_begin(ctx, Zone::Composite);
        self.record_composite(gpu, &post, gtao_active.then_some(&self.gtao_blur), dest)?;
        self.zone_end(ctx, Zone::Composite);
        Ok(())
    }

    /// Make `slot` the active one. A no-op when it already is; otherwise a single
    /// `mem::swap`, which is why alternating between two models within a frame
    /// costs nothing.
    fn activate(&mut self, slot: SlotId) {
        if self.active_slot == slot {
            return;
        }
        std::mem::swap(&mut self.active, &mut self.idle);
        self.active_slot = slot;
    }

    /// Drop the processed model's cached buffers (invariant 3), whichever slot
    /// currently holds them. Called when the Opt workspace has no processed mesh
    /// to show, so its GPU memory isn't held while another workspace is up.
    pub(crate) fn release_processed(&mut self) {
        if self.active_slot == SlotId::Processed {
            self.active.release();
        } else {
            self.idle.release();
        }
    }

    /// Draw the idle slot's mesh as a see-through ghost over the solid one,
    /// inside the scene pass the caller has already opened.
    ///
    /// Both styles reuse pipelines that already exist. The x-ray is the
    /// selection-flash fill — a flat tinted colour, alpha-blended, depth-tested
    /// but not depth-writing, which is exactly ghost behaviour — with the tint fed
    /// through the same `selection_color` uniform it always reads. The wireframe
    /// ghost is the derived wireframe view drawn with the line pipeline. Neither
    /// needs a shader change, so the committed DXBC stays valid.
    fn record_ghost(
        &mut self,
        gpu: &Gpu,
        camera: OrbitCamera,
        frame: &SceneFrame<'_>,
        style: GhostStyle,
        tint: [f32; 3],
    ) -> windows::core::Result<()> {
        let ctx = gpu.context();

        // The ghost's own uniform: same camera and projection, but the flat fill
        // colour swapped in. Restored to the frame's own uniform afterwards so the
        // GTAO pass (which reads `view` from `b0`) still sees the right one.
        let mut uniforms = scene_uniforms(
            camera,
            frame.projection,
            frame.environment,
            frame.selection,
            frame.debug,
        );
        uniforms.selection_color = ghost_tint(style, tint);
        self.uniforms.update(ctx, &uniforms)?;

        match style {
            GhostStyle::Xray => {
                if let Some(mesh) = &self.idle.mesh {
                    self.selection_pipeline.bind(ctx);
                    mesh.vertices.bind(ctx);
                    mesh.indices.bind(ctx);
                    gpu.draw_indexed_range(mesh.indices.count(), 0);
                }
            }
            GhostStyle::Wireframe => {
                if let Some(lines) = &self.idle.ghost_wireframe_buf {
                    self.line_pipeline.bind(ctx);
                    lines.bind(ctx);
                    gpu.draw(lines.count());
                }
            }
        }

        // Put the frame's own uniform back for the passes that follow.
        let restored = scene_uniforms(
            camera,
            frame.projection,
            frame.environment,
            frame.selection,
            frame.debug,
        );
        self.uniforms.update(ctx, &restored)?;
        Ok(())
    }

    /// Build the active slot's ghost wireframe, which exists regardless of the
    /// user's wireframe toggle — in the wireframe ghost style it *is* the ghost,
    /// not an overlay on it.
    fn sync_ghost_wireframe(
        &mut self,
        device: &ID3D11Device,
        frame: &SceneFrame<'_>,
        tint: [f32; 3],
    ) -> windows::core::Result<()> {
        let colour = ghost_tint(GhostStyle::Wireframe, tint);
        let want = Some((frame.model_revision, frame.hidden_meshes.to_vec(), colour));
        if self.active.ghost_wireframe_baked == want {
            return Ok(());
        }
        let lines = wireframe_lines(frame.model, colour, frame.hidden_meshes);
        self.active.ghost_wireframe_buf = optional_vertex_buffer(device, &lines)?;
        self.active.ghost_wireframe_baked = want;
        Ok(())
    }

    /// Free both slots' ghost wireframes (invariant 3) — the overlay view is no
    /// longer showing one.
    fn release_ghost_wireframes(&mut self) {
        for slot in [&mut self.active, &mut self.idle] {
            slot.ghost_wireframe_buf = None;
            slot.ghost_wireframe_baked = None;
        }
    }

    /// Reconcile every GPU resource with this frame's inputs: the offscreen
    /// targets + scene pipelines (size / AA level), the Unique-mode part key, the
    /// mesh buffers + effective material table, the derived line views, the
    /// selection / visibility draw lists, and the IBL maps.
    fn sync_frame(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        target_size: (u32, u32),
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let ctx = gpu.context();

        self.sync_targets(
            gpu,
            frame.anti_aliasing.effective_sample_count(),
            target_size,
        )?;

        // The Unique-mode part key, then the mesh + the effective material table
        // (both depend on the active material mode's grouping).
        self.sync_unique_parts(frame.model, frame.model_revision, frame.debug.material_mode);
        self.sync_mesh(
            device,
            frame.model,
            frame.model_revision,
            frame.debug.uv_channel,
            frame.debug.material_mode,
        )?;
        let effective = effective_materials(
            frame.debug.material_mode,
            material_states,
            self.active.unique_part_count,
        );
        self.materials.sync(
            device,
            ctx,
            &effective,
            material_revision,
            frame.debug.material_mode,
        )?;

        // Build-on-demand / free-on-off for the derived 3D line overlays (invariant
        // 3): each view's buffer exists only while its toggle is on, rebuilt live
        // when its baked params (color / length / hidden set / scope) drift.
        self.sync_line_views(
            device,
            frame.model,
            frame.model_revision,
            frame.debug,
            frame.hidden_meshes,
        )?;
        self.sync_skeleton(
            device,
            frame.model,
            frame.model_revision,
            frame.debug,
            frame.selected_bones,
        )?;
        self.sync_skin_weights(
            device,
            frame.model,
            frame.model_revision,
            frame.debug,
            frame.selected_bones,
        )?;

        // Build (or free) the selected-triangle draw list (the solo isolate list +
        // the highlight-flash fill source) and the per-mesh visibility filter, when
        // the selection / hidden set / model / mode drifts (invariant 3).
        self.sync_selection(
            device,
            frame.model,
            frame.model_revision,
            frame.selection,
            frame.hidden_meshes,
            frame.debug.material_mode,
        )?;
        self.sync_visibility(
            device,
            frame.model,
            frame.model_revision,
            frame.hidden_meshes,
            frame.debug.material_mode,
        )?;

        // Reload the IBL maps when the chosen environment changes (a pure upload).
        if self.ibl.environment != frame.environment.map {
            self.ibl = IblD3d::from_baked(gpu, frame.environment.map)?;
        }
        Ok(())
    }

    /// Shared per-pass bindings for the scene shader: `b0` (VS + PS), the checker
    /// (`t0`/`s0`), the IBL maps (`t1..t4`) + their sampler (`s1`), and the
    /// material cbuffer + sampler (`b1`/`s2`). The mesh loop rebinds `b1` +
    /// `t5..t11` per range.
    fn bind_scene_shared(
        &self,
        ctx: &ID3D11DeviceContext,
        checker: &Texture,
    ) -> windows::core::Result<()> {
        self.uniforms.bind_vs(ctx, 0);
        self.uniforms.bind_ps(ctx, 0);
        checker.bind_ps(ctx, 0);
        self.checker_sampler.bind_ps(ctx, 0);
        self.ibl.bind_ps(ctx);
        self.sampler.bind_ps(ctx, 1);
        self.materials.bind_shared(ctx);
        self.materials.bind_fallback(ctx)
    }

    /// The offscreen 2-MRT scene pass: skybox, the mesh draw list, the grid, the
    /// derived line overlays, the pivot marker and the selection flash.
    fn record_scene_pass(&self, gpu: &Gpu, frame: &SceneFrame<'_>) -> windows::core::Result<()> {
        let ctx = gpu.context();
        let debug = frame.debug;
        let selection = frame.selection;

        // Clear the scene color + ambient to zero radiance *and* zero alpha: the
        // alpha is the composite's coverage mask, so the cleared background reads as
        // "no geometry" and the post pass paints the chosen viewport background
        // there (in display space, after tone mapping).
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, [0.0; 4]);
        self.zone_begin(ctx, Zone::Scene);
        let checker = match debug.uv_checker_texture {
            CheckerTexture::Greyscale => &self.checker_greyscale,
            CheckerTexture::Color => &self.checker_color,
        };
        self.bind_scene_shared(ctx, checker)?;

        // Skybox background first, behind all geometry, when shown.
        if frame.environment.show_background {
            self.skybox_pipeline.bind(ctx);
            gpu.draw(3);
        }

        // Mesh draw list, in precedence order: solo (isolate the selection) wins;
        // otherwise per-mesh visibility (the filtered list, present only while some
        // mesh is hidden); otherwise the whole mesh. All three share the mesh vertex
        // buffer, so only the index source + ranges differ. Wireframe shading draws
        // no filled surface.
        let solo = selection.solo && selection.selection.is_active();
        if let Some(mesh) = &self.active.mesh
            && !matches!(debug.shading_mode, ShadingMode::Wireframe)
        {
            let pipeline = if debug.render_backfaces {
                &self.mesh_double_sided_pipeline
            } else {
                &self.mesh_pipeline
            };
            pipeline.bind(ctx);
            // The skin-weight heat map is a drop-in replacement for the mesh's
            // vertex buffer: same length, same order, so the index buffer, the
            // per-material ranges and the solo / visibility lists below all stay
            // valid. It keeps the real normals (the shader Lambert-shades it) and
            // is selected by `projection_params.w`, so the material bound per range
            // is simply ignored.
            let vertex_source = self
                .active
                .views
                .weights_buf
                .as_ref()
                .unwrap_or(&mesh.vertices);
            vertex_source.bind(ctx);
            // Solo draws only the selection (empty → nothing); visible draws the
            // filtered list (None while active means every mesh is hidden → nothing);
            // otherwise the whole mesh.
            let draw_list: Option<(&IndexBuffer, &[MaterialDrawRange])> = if solo {
                self.active
                    .selection_index
                    .as_ref()
                    .map(|index| (index, self.active.selection_ranges.as_slice()))
            } else if self.active.visible_active {
                self.active
                    .visible_index
                    .as_ref()
                    .map(|index| (index, self.active.visible_ranges.as_slice()))
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
            &self.active.views.wireframe_buf,
            &self.active.views.bounding_box_buf,
            &self.active.views.face_normal_buf,
            &self.active.views.vertex_normal_buf,
        ];
        if line_views.iter().any(|view| view.is_some()) {
            self.line_pipeline.bind(ctx);
            for buffer in line_views.into_iter().flatten() {
                buffer.bind(ctx);
                gpu.draw(buffer.count());
            }
        }

        // Pivot marker: drawn last among the line overlays with the always-on-top
        // line pipeline (depth compare `Always`), so the 3-axis cross reads through
        // the mesh instead of being occluded inside it.
        if let Some(pivot) = &self.active.views.pivot_buf {
            self.line_overlay_pipeline.bind(ctx);
            pivot.bind(ctx);
            gpu.draw(pivot.count());
        }

        // Skeleton overlay: translucent octahedron fills first, then their opaque
        // outlines on top, both always-on-top so the rig reads through the
        // character it deforms. The per-bone selection tint is already baked into
        // these buffers (see `sync_skeleton`).
        if let Some(fill) = &self.active.views.skeleton_fill_buf {
            self.fill_overlay_pipeline.bind(ctx);
            fill.bind(ctx);
            gpu.draw(fill.count());
        }
        if let Some(lines) = &self.active.views.skeleton_line_buf {
            self.line_overlay_pipeline.bind(ctx);
            lines.bind(ctx);
            gpu.draw(lines.count());
        }

        // Selection highlight flash: a flat bright-color fill redrawing the selected
        // triangles over the mesh, fading out after a selection change. Drawn last in
        // the scene pass so it sits on top. Reuses the selection index buffer over the
        // shared mesh vertex buffer; `fs_selection` tints it with the uniform
        // highlight color × the flash fade (`selection_color`, already in `b0`).
        // Skipped once the flash has faded, so the steady state pays nothing.
        let flash = selection.selection.is_active() && selection.fade > 0.0;
        if flash
            && let (Some(mesh), Some(index)) = (&self.active.mesh, &self.active.selection_index)
        {
            self.selection_pipeline.bind(ctx);
            mesh.vertices.bind(ctx);
            index.bind(ctx);
            gpu.draw_indexed_range(index.count(), 0);
        }
        self.zone_end(ctx, Zone::Scene);
        Ok(())
    }

    /// The GTAO passes: a single-sample mesh-only G-buffer (view normal + Z), then
    /// the horizon occlusion pass (→ raw `R8`) and the bilateral blur (→ blurred
    /// `R8`) the composite darkens the ambient radiance by. The G-buffer has its
    /// own depth (nearest-surface) and shares `b0` (the scene uniforms carry
    /// `view`); the fullscreen passes read `b0` as the GTAO uniform + `s0` as the
    /// point sampler.
    fn record_gtao(
        &self,
        gpu: &Gpu,
        camera: OrbitCamera,
        projection: CameraProjection,
        gtao: GtaoSettings,
    ) -> windows::core::Result<()> {
        let ctx = gpu.context();
        let gtao_uniforms = build_gtao_uniforms(camera, projection, gtao);
        self.gtao_uniforms.update(ctx, &gtao_uniforms)?;

        // G-buffer: redraw the whole mesh (material irrelevant) into the
        // single-sample normal/Z target, clearing the target + its depth.
        self.zone_begin(ctx, Zone::GtaoGbuffer);
        gpu.begin_scene_pass(&[&self.gtao_gbuffer], &self.gtao_depth, [0.0; 4]);
        self.uniforms.bind_vs(ctx, 0);
        self.uniforms.bind_ps(ctx, 0);
        self.gtao_gbuffer_pipeline.bind(ctx);
        if let Some(mesh) = &self.active.mesh {
            mesh.vertices.bind(ctx);
            // Match the shaded mesh's visibility so a hidden mesh casts no AO; solo
            // is deliberately left out, so only the per-mesh hide filters the AO.
            // `None` while active means every mesh is hidden → nothing to occlude.
            let index = if self.active.visible_active {
                self.active.visible_index.as_ref()
            } else {
                Some(&mesh.indices)
            };
            if let Some(index) = index {
                index.bind(ctx);
                gpu.draw_indexed_range(index.count(), 0);
            }
        }
        self.zone_end(ctx, Zone::GtaoGbuffer);

        // Occlusion: a fullscreen pass reading the G-buffer (`t0`) → raw AO.
        self.zone_begin(ctx, Zone::Gtao);
        gpu.begin_color_pass(&self.gtao_raw);
        self.gtao_pipeline.bind(ctx);
        self.gtao_uniforms.bind_ps(ctx, 0);
        self.gtao_sampler.bind_ps(ctx, 0);
        self.gtao_gbuffer.bind_ps_srv(ctx, 0);
        gpu.draw(3);
        self.zone_end(ctx, Zone::Gtao);

        // Bilateral blur: reads the G-buffer (`t0`) + raw AO (`t1`) → blurred AO.
        // `begin_color_pass` rebinds the RTV to `gtao_blur`, releasing `gtao_raw`
        // as a render target before it's bound below as an SRV.
        self.zone_begin(ctx, Zone::GtaoBlur);
        gpu.begin_color_pass(&self.gtao_blur);
        self.gtao_blur_pipeline.bind(ctx);
        self.gtao_gbuffer.bind_ps_srv(ctx, 0);
        self.gtao_raw.bind_ps_srv(ctx, 1);
        gpu.draw(3);
        self.zone_end(ctx, Zone::GtaoBlur);
        // Drop the G-buffer / raw SRVs before the composite binds the scene
        // targets (and before next frame rebinds them as render targets).
        gpu.unbind_ps_srvs(2);
        Ok(())
    }

    /// Composite the offscreen HDR scene into the backbuffer: resolve the MSAA MRT
    /// into the single-sample textures the post pass samples (a no-op at 1×), then
    /// the fullscreen post pass (AO-darkened ambient + tone map + sRGB) over the
    /// viewport background. `ao` is the blurred GTAO target, or `None` to bind the
    /// ambient as a harmless placeholder (the shader ignores `t1` when
    /// `gtao_enabled` is 0). `self.sampler` (linear clamp) is bound at `s0`.
    fn record_composite(
        &self,
        gpu: &Gpu,
        post: &PostUniforms,
        ao: Option<&ColorTarget>,
        dest: BackbufferRect,
    ) -> windows::core::Result<()> {
        let ctx = gpu.context();
        self.post_uniforms.update(ctx, post)?;
        gpu.begin_backbuffer_blit_rect(dest.x, dest.y, dest.width, dest.height);
        // The scene RTVs are unbound now (the backbuffer is the only bound
        // target), so the multisample resolve source is free.
        self.color.resolve(ctx);
        self.ambient.resolve(ctx);
        self.composite_pipeline.bind(ctx);
        self.post_uniforms.bind_ps(ctx, 0);
        self.color.bind_ps_srv(ctx, 0);
        ao.unwrap_or(&self.ambient).bind_ps_srv(ctx, 1);
        self.ambient.bind_ps_srv(ctx, 2);
        self.sampler.bind_ps(ctx, 0);
        gpu.draw(3);
        // Release the offscreen SRVs so next frame can bind them as render targets.
        gpu.unbind_ps_srvs(3);
        Ok(())
    }

    /// Render the 2D UV viewport (instead of the 3D scene): the 0..1 grid, the
    /// optional island fill (Shaded / Islands modes), then the model's UV edges on
    /// top — all framed by the 2D `uv_camera` and composited like the 3D scene
    /// (tone-mapped, no GTAO).
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
        background: ViewportBackground,
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let ctx = gpu.context();

        // The UV viewport shows the source model, so its derived buffers belong in
        // the source slot (see the note in `render`).
        self.activate(SlotId::Source);
        self.sync_targets(gpu, anti_aliasing.effective_sample_count(), gpu.size())?;
        self.sync_uv_view(device, model, model_revision, channel, shading_mode)?;

        // The UV camera's orthographic view-projection; the rest of the uniform is
        // unused by the UV path (lines/fill return their own vertex color).
        let uniforms = uv_scene_uniforms(uv_camera);
        self.uniforms.update(ctx, &uniforms)?;

        // Clear to zero (radiance + coverage); the background is painted in the
        // composite, matching the 3D path.
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, [0.0; 4]);
        // `fs_main` (the UV fill) samples the checker + every material slot at the top
        // (uniform control flow) before its zero-normal early-out, so all of group
        // 1/2/3 must be bound even though the UV draws never use the sampled values.
        self.bind_scene_shared(ctx, &self.checker_greyscale)?;

        // Reference grid first.
        self.line_pipeline.bind(ctx);
        self.uv_grid.bind(ctx);
        gpu.draw(self.uv_grid.count());

        // Island fill (Shaded / Islands), under the wireframe.
        if let Some(fill) = &self.active.views.uv_fill_buf {
            self.uv_fill_pipeline.bind(ctx);
            fill.bind(ctx);
            gpu.draw(fill.count());
        }

        // The model's UV edges on top.
        if let Some(wireframe) = &self.active.views.uv_wireframe_buf {
            self.line_pipeline.bind(ctx);
            wireframe.bind(ctx);
            gpu.draw(wireframe.count());
        }

        // Composite to the backbuffer: tone-mapped (the default operator, so shaded
        // fills read like the 3D scene), no GTAO (the flat UV viewport has no depth
        // to occlude), over the chosen viewport background.
        let post = post_uniforms(background, false, TonemapSettings::default(), false);
        self.record_composite(gpu, &post, None, BackbufferRect::full(gpu.size()))?;

        Ok(())
    }

    /// Build-on-demand for the UV viewport's derived buffers (invariant 3): the
    /// wireframe is rebuilt only when the model / channel changes; the island fill
    /// when the model / channel / shading mode changes, and freed in Wire mode — so
    /// panning/zooming rebuilds nothing.
    fn sync_uv_view(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        channel: u32,
        shading_mode: UvShadingMode,
    ) -> windows::core::Result<()> {
        let want_wireframe = Some((model_revision, channel));
        if self.active.views.uv_wireframe_baked != want_wireframe {
            self.active.views.uv_wireframe_buf =
                optional_vertex_buffer(device, &uv_wireframe_lines(model, channel))?;
            self.active.views.uv_wireframe_baked = want_wireframe;
        }

        let want_fill = match shading_mode {
            UvShadingMode::Wire => None,
            UvShadingMode::Shaded | UvShadingMode::Islands => {
                Some((model_revision, channel, shading_mode))
            }
        };
        if self.active.views.uv_fill_baked != want_fill {
            let fill = match shading_mode {
                UvShadingMode::Wire => Vec::new(),
                UvShadingMode::Shaded => uv_fill_triangles(model, channel, false),
                UvShadingMode::Islands => uv_fill_triangles(model, channel, true),
            };
            self.active.views.uv_fill_buf = optional_vertex_buffer(device, &fill)?;
            self.active.views.uv_fill_baked = want_fill;
        }

        Ok(())
    }

    /// Reconcile the Unique-mode per-triangle mesh-part key: baked by `model_revision`
    /// while Unique is active, freed back to empty otherwise (invariant 3).
    fn sync_unique_parts(&mut self, model: &ModelData, model_revision: u64, mode: MaterialMode) {
        let want = matches!(mode, MaterialMode::Unique).then_some(model_revision);
        if self.active.unique_baked == want {
            return;
        }
        match want {
            Some(_) => {
                let (key, count) = build_part_key(model);
                self.active.unique_part_key = key;
                self.active.unique_part_count = count;
            }
            None => {
                self.active.unique_part_key = Vec::new();
                self.active.unique_part_count = 0;
            }
        }
        self.active.unique_baked = want;
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
        if self.active.mesh_revision == model_revision
            && self.active.mesh_uv_channel == uv_channel
            && self.active.mesh_material_mode == mode
        {
            return Ok(());
        }
        let key = match mode {
            MaterialMode::Unique if !self.active.unique_part_key.is_empty() => {
                Some(self.active.unique_part_key.as_slice())
            }
            _ => None,
        };
        let (vertices, indices, ranges) = model_mesh(model, uv_channel, key);
        self.active.mesh = if indices.is_empty() {
            None
        } else {
            Some(MeshBuffers {
                vertices: VertexBuffer::new(device, &vertices)?,
                indices: IndexBuffer::new(device, &indices)?,
                ranges,
            })
        };
        self.active.mesh_revision = model_revision;
        self.active.mesh_uv_channel = uv_channel;
        self.active.mesh_material_mode = mode;
        Ok(())
    }

    /// Build-on-demand / free-on-off for the derived line views (invariant 3).
    /// A view's buffer is (re)built when its toggle is on and its baked params
    /// drift from the current options, and dropped to `None` when off. Unchanged
    /// views are left untouched, and the drift checks compare against the
    /// *borrowed* frame inputs, so a steady-state frame allocates nothing.
    fn sync_line_views(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        hidden_meshes: &[u32],
    ) -> windows::core::Result<()> {
        // A different mesh invalidates every view derived from it, whatever the
        // view's own parameters are doing (see `ModelSlot::views_revision`).
        // Dropping them here is what lets each bake key below stay purely about
        // its own settings; the ones still switched on rebuild in this same call.
        if self.active.views_revision != model_revision {
            self.active.views = DerivedViews::default();
            self.active.views_revision = model_revision;
        }

        // Wireframe rebuilds when its color *or* the Outliner's hidden set drifts
        // (edges of a hidden mesh disappear with the mesh). On in both the wireframe
        // overlay and the wireframe-only shading mode.
        let wireframe_on =
            debug.wireframe_overlay || matches!(debug.shading_mode, ShadingMode::Wireframe);
        let wireframe_unchanged = match (&self.active.views.wireframe_baked, wireframe_on) {
            (None, false) => true,
            (Some((color, hidden)), true) => {
                *color == debug.wireframe_color && hidden == hidden_meshes
            }
            _ => false,
        };
        if !wireframe_unchanged {
            self.active.views.wireframe_buf = if wireframe_on {
                optional_vertex_buffer(
                    device,
                    &wireframe_lines(model, debug.wireframe_color, hidden_meshes),
                )?
            } else {
                None
            };
            self.active.views.wireframe_baked =
                wireframe_on.then(|| (debug.wireframe_color, hidden_meshes.to_vec()));
        }

        // Bounding box: only the inputs the chosen scope depends on go in the bake
        // key, so an unrelated change can't rebuild the box.
        let scope = debug.bounding_box_scope;
        let scope_hidden: &[u32] = match scope {
            BoundingBoxScope::VisibleOnly => hidden_meshes,
            _ => &[],
        };
        let scope_selection = match scope {
            BoundingBoxScope::OnlySelection => debug.bounding_box_selection,
            _ => Selection::None,
        };
        let bounding_box_unchanged = match (
            &self.active.views.bounding_box_baked,
            debug.show_bounding_box,
        ) {
            (None, false) => true,
            (Some(params), true) => {
                params.color == debug.bounding_box_color
                    && params.scope == scope
                    && params.hidden == scope_hidden
                    && params.selection == scope_selection
            }
            _ => false,
        };
        if !bounding_box_unchanged {
            self.active.views.bounding_box_buf = if debug.show_bounding_box {
                let bounds = match scope {
                    BoundingBoxScope::AllMeshes => model.bounds,
                    BoundingBoxScope::OnlySelection => selection_bounds(model, scope_selection),
                    BoundingBoxScope::VisibleOnly => model.visible_bounds(scope_hidden),
                };
                match bounds {
                    Some(bounds) => optional_vertex_buffer(
                        device,
                        &bounding_box_lines(bounds, debug.bounding_box_color),
                    )?,
                    None => None,
                }
            } else {
                None
            };
            self.active.views.bounding_box_baked =
                debug.show_bounding_box.then(|| BoundingBoxParams {
                    color: debug.bounding_box_color,
                    scope,
                    hidden: scope_hidden.to_vec(),
                    selection: scope_selection,
                });
        }

        // The two normal-line views share one shape (length + color + hidden set),
        // differing only in the toggle and line builder.
        sync_normal_view(
            device,
            model,
            &mut self.active.views.face_normal_buf,
            &mut self.active.views.face_baked,
            debug.face_normals,
            debug.face_normal_length,
            debug.face_normal_color,
            hidden_meshes,
            face_normal_lines,
        )?;
        sync_normal_view(
            device,
            model,
            &mut self.active.views.vertex_normal_buf,
            &mut self.active.views.vertex_baked,
            debug.vertex_normals,
            debug.vertex_normal_length,
            debug.vertex_normal_color,
            hidden_meshes,
            vertex_normal_lines,
        )?;

        // Pivot marker: a 3-axis cross at the model's origin. Its bytes depend only
        // on the model (pivot position + size), so the bake key rebuilds it on a
        // model swap and the buffer is freed while the toggle is off.
        let want_pivot = debug.show_pivot.then(|| PivotParams {
            pivot: model_pivot(model),
            half: pivot_half_extent(model),
        });
        if self.active.views.pivot_baked != want_pivot {
            self.active.views.pivot_buf = match &want_pivot {
                Some(PivotParams { pivot, half }) => {
                    optional_vertex_buffer(device, &pivot_lines(*pivot, *half))?
                }
                None => None,
            };
            self.active.views.pivot_baked = want_pivot;
        }

        Ok(())
    }

    /// Build (or free) the skin-weight heat map's vertex buffer.
    ///
    /// Active only while the Skin Weights material is chosen *and* the model
    /// actually carries skin — a model without it falls back to the ordinary mesh
    /// rather than showing a blank one. Freed the moment the mode changes
    /// (invariant 3), so the steady-state shaded view holds nothing.
    ///
    /// Rebuilt on every change of the selected bone set, which is what makes the
    /// heat map follow the Outliner. That is a full vertex re-upload (~48 bytes
    /// per render vertex); the alternative — a per-vertex weight lookup in the
    /// shader — would need a structured buffer and a capability gate (invariant 4)
    /// to save a cost only paid on an explicit click.
    fn sync_skin_weights(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        selected_bones: &[u32],
    ) -> windows::core::Result<()> {
        let active = debug.active_material == ActiveMaterial::SkinWeights && model.skin.is_some();
        let want = active.then(|| SkinWeightParams {
            model_revision,
            selected: selected_bones.to_vec(),
        });
        if self.active.views.weights_baked == want {
            return Ok(());
        }
        self.active.views.weights_buf = match &want {
            Some(params) => {
                optional_vertex_buffer(device, &skin_weight_vertices(model, &params.selected))?
            }
            None => None,
        };
        self.active.views.weights_baked = want;
        Ok(())
    }

    /// Build (or free) the skeleton overlay's two buffers — the octahedron fills
    /// and their outlines — following the same build-on-demand / free-on-off
    /// discipline as the line views (invariant 3): while the toggle is off both are
    /// `None` and the overlay costs nothing.
    ///
    /// The bake key carries the *selected* bone set alongside the colors, because
    /// the highlight is baked per bone into the vertex color rather than applied
    /// from a uniform. That means an Outliner click rebuilds these buffers — the
    /// same trade `sync_selection` makes, and cheap here: a 72-bone rig is a few
    /// thousand vertices.
    fn sync_skeleton(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        selected_bones: &[u32],
    ) -> windows::core::Result<()> {
        let want = debug.show_skeleton.then(|| SkeletonParams {
            model_revision,
            selected: selected_bones.to_vec(),
            scale: debug.skeleton_joint_scale,
            color: debug.skeleton_color,
            selected_color: debug.skeleton_selected_color,
        });
        if self.active.views.skeleton_baked == want {
            return Ok(());
        }
        match &want {
            Some(params) => {
                self.active.views.skeleton_fill_buf = optional_vertex_buffer(
                    device,
                    &skeleton_fill_triangles(
                        model,
                        &params.selected,
                        params.scale,
                        params.color,
                        params.selected_color,
                        SKELETON_FILL_ALPHA,
                    ),
                )?;
                self.active.views.skeleton_line_buf = optional_vertex_buffer(
                    device,
                    &skeleton_lines(
                        model,
                        &params.selected,
                        params.scale,
                        params.color,
                        params.selected_color,
                    ),
                )?;
            }
            None => {
                self.active.views.skeleton_fill_buf = None;
                self.active.views.skeleton_line_buf = None;
            }
        }
        self.active.views.skeleton_baked = want;
        Ok(())
    }

    /// The per-triangle grouping key for `mode`: the cached mesh-part key in Unique
    /// mode (when the model carries per-triangle node info), else `None` to group by
    /// material slot. Call after `sync_unique_parts`.
    fn grouping_key(&self, mode: MaterialMode) -> Option<&[u32]> {
        match mode {
            MaterialMode::Unique if !self.active.unique_part_key.is_empty() => {
                Some(&self.active.unique_part_key)
            }
            _ => None,
        }
    }

    /// Build (or free) the selected-triangle draw list (the solo isolate list + the
    /// flash fill source) when the selection / model / hidden set / mode drifts
    /// (invariant 3). The highlight color + flash fade ride in the uniform, so they
    /// never trigger a rebuild — only a change of *what* is selected does.
    fn sync_selection(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        view: SelectionView,
        hidden: &[u32],
        mode: MaterialMode,
    ) -> windows::core::Result<()> {
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let active = view.selection.is_active();
        let unchanged = match (&self.active.selection_baked, active) {
            (None, false) => true,
            (Some(baked), true) => {
                baked.model_revision == model_revision
                    && baked.selection == view.selection
                    && baked.hidden == hidden
                    && baked.mode == mode
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        // Resolve the visible selected triangles, grouped by the same key as the main
        // mesh so each range binds the right effective material. `None` (no usable
        // geometry) or an empty list both clear to a no-draw selection.
        let geometry = if active {
            let key = self.grouping_key(mode);
            selection_geometry(model, view.selection, hidden, key)
        } else {
            None
        };
        match geometry {
            Some((indices, ranges)) if !indices.is_empty() => {
                self.active.selection_index = Some(IndexBuffer::new(device, &indices)?);
                self.active.selection_ranges = ranges;
            }
            _ => {
                self.active.selection_index = None;
                self.active.selection_ranges = Vec::new();
            }
        }
        self.active.selection_baked = active.then(|| SelectionBaked {
            model_revision,
            selection: view.selection,
            hidden: hidden.to_vec(),
            mode,
        });
        Ok(())
    }

    /// Build (or free) the per-mesh visibility draw list when the hidden set / model /
    /// mode drifts (invariant 3). `hidden` empty means nothing is hidden (full mesh,
    /// no filter). When every mesh is hidden the filter is active but the list empty
    /// (draw nothing); when the model carries no per-triangle node info the filter is
    /// off (full mesh).
    fn sync_visibility(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        hidden: &[u32],
        mode: MaterialMode,
    ) -> windows::core::Result<()> {
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let active = !hidden.is_empty();
        let unchanged = match (&self.active.visibility_baked, active) {
            (None, false) => true,
            (Some(baked), true) => {
                baked.model_revision == model_revision
                    && baked.hidden == hidden
                    && baked.mode == mode
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        let geometry = if active {
            let key = self.grouping_key(mode);
            visible_geometry(model, hidden, key)
        } else {
            None
        };
        match geometry {
            // Some unhidden geometry: draw the filtered list.
            Some((indices, ranges)) if !indices.is_empty() => {
                self.active.visible_index = Some(IndexBuffer::new(device, &indices)?);
                self.active.visible_ranges = ranges;
                self.active.visible_active = true;
            }
            // Every mesh hidden: the filter is active but draws nothing.
            Some(_) => {
                self.active.visible_index = None;
                self.active.visible_ranges = Vec::new();
                self.active.visible_active = true;
            }
            // Nothing hidden, or the model carries no per-triangle node info: draw the
            // full mesh (no filter).
            None => {
                self.active.visible_index = None;
                self.active.visible_ranges = Vec::new();
                self.active.visible_active = false;
            }
        }
        self.active.visibility_baked = active.then(|| VisibilityBaked {
            model_revision,
            hidden: hidden.to_vec(),
            mode,
        });
        Ok(())
    }
}

/// The six MSAA-dependent scene pipelines (line / mesh / double-sided mesh / skybox /
/// UV fill / selection fill). They all draw into the MSAA scene MRT, so their sample
/// count is baked at the live AA level; `SceneGpu::new` + `rebuild_scene_pipelines`
/// build them together via this helper so they stay in lockstep with the targets.
struct ScenePipelineSet {
    line: Pipeline,
    line_overlay: Pipeline,
    fill_overlay: Pipeline,
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

    // Always-on-top line variant (depth compare `Always`, no write): the pivot
    // marker uses this so the model never occludes it — it reads through solid
    // geometry, unlike the depth-tested overlays above.
    let line_overlay = Pipeline::new(
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
                compare: DepthCompare::Always,
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

    // Skeleton octahedron fills: the same flat overlay path the lines use (zero
    // normals -> `fs_main`'s overlay branch), but as triangles and with
    // `DepthCompare::Always` so the bones read *through* the character. A skeleton
    // lives inside its mesh, so depth-testing it would hide the entire thing —
    // the same reasoning as the pivot marker's `line_overlay` above. Double-sided,
    // since an octahedron is viewed from every angle as the camera orbits.
    let fill_overlay = Pipeline::new(
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
                compare: DepthCompare::Always,
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
        line_overlay,
        fill_overlay,
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

/// Reconcile one normal-line view (face or vertex normals — the same shape,
/// differing only in the toggle and `lines` builder): rebuild when on + drifted
/// (compared against the borrowed inputs, so a steady-state frame allocates
/// nothing), free when off.
#[allow(clippy::too_many_arguments)] // Disjoint &mut field pairs + the view's plain inputs.
fn sync_normal_view(
    device: &ID3D11Device,
    model: &ModelData,
    buf: &mut Option<VertexBuffer>,
    baked: &mut Option<NormalParams>,
    on: bool,
    length: f32,
    color: [f32; 4],
    hidden: &[u32],
    lines: fn(&ModelData, f32, [f32; 4], &[u32]) -> Vec<crate::scene::SceneVertex>,
) -> windows::core::Result<()> {
    let unchanged = match (&baked, on) {
        (None, false) => true,
        (Some(params), true) => {
            params.length == length && params.color == color && params.hidden == hidden
        }
        _ => false,
    };
    if unchanged {
        return Ok(());
    }
    *buf = if on {
        optional_vertex_buffer(device, &lines(model, length, color, hidden))?
    } else {
        None
    };
    *baked = on.then(|| NormalParams {
        length,
        color,
        hidden: hidden.to_vec(),
    });
    Ok(())
}

/// Decode a baked UV-checker PNG into an sRGB GPU texture. A decode failure is a
/// packaging bug — fall back to a 1×1 white texel rather than failing the build of
/// the scene resources.
fn decode_checker(device: &ID3D11Device, png_bytes: &[u8]) -> windows::core::Result<Texture> {
    match image::load_from_memory(png_bytes) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Texture::rgba8_single(device, width, height, &rgba, true)
        }
        Err(error) => {
            // Degrading to flat white is deliberate, but not silently: a corrupt
            // baked checker is a packaging bug worth seeing under `--tracy`.
            gpu_profiler::note(&format!("baked UV-checker PNG failed to decode: {error}"));
            Texture::rgba8_single(device, 1, 1, &[255, 255, 255, 255], true)
        }
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera, projection, environment,
/// selection and debug options. The selection
/// flash rides in `selection_color` (gamma-space rgb + the flash fade in alpha,
/// zero while nothing is selected/flashing), read only by `fs_selection`.
/// Whether this frame is one of the flat data-inspection views, which emit final
/// display pixels from the scene shader.
///
/// They bypass lighting, the composite's tone map and GTAO entirely — a value
/// shown through a tone curve is no longer the value — so the composite blits
/// straight through and the occlusion passes are skipped.
fn flat_display(frame: &SceneFrame<'_>) -> bool {
    matches!(
        frame.debug.active_material,
        ActiveMaterial::Buffers | ActiveMaterial::SkinWeights
    )
}

/// The ghost's colour, as the `selection_color` uniform's gamma-space RGB plus
/// alpha.
///
/// The hue comes from the caller — the chrome shows the same colour in the
/// overlay's legend, and a swatch that disagreed with the mesh would be worse
/// than no legend. The alpha is decided here because it is a rendering matter:
/// the x-ray's is low enough that the solid mesh stays legible through it but
/// high enough that a silhouette spilling past that surface is obvious — the
/// spill is the whole signal the overlay exists to show. The wireframe ghost is
/// opaque: an alpha-faded line a pixel wide would simply disappear.
fn ghost_tint(style: GhostStyle, tint: [f32; 3]) -> [f32; 4] {
    let [r, g, b] = tint;
    match style {
        GhostStyle::Xray => [r, g, b, 0.28],
        GhostStyle::Wireframe => [r, g, b, 1.0],
    }
}

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
            // `w` flags the skin-weight heat map.
            skin_weight_value(debug),
        ],
        view: camera.view_matrix().to_cols_array_2d(),
        selection_color,
    }
}

/// Build the [`SceneUniforms`] for the 2D UV viewport: only the UV camera's
/// orthographic view-projection matters (the grid / wireframe / fill return their
/// own vertex color, never reaching the IBL / shading / selection code).
/// `projection_params.x = 1.0` marks orthographic.
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
/// settings. The settings' `radius` is a
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant 11: the hand-maintained input-element list must cover exactly
    /// the `#[repr(C)] SceneVertex` — a drifted field order/size would misfeed
    /// the vertex shader with no runtime error.
    #[test]
    fn scene_vertex_layout_matches_struct_stride() {
        let layout_stride: usize = SCENE_VERTEX_LAYOUT
            .iter()
            .map(|element| element.format.byte_size())
            .sum();
        assert_eq!(
            layout_stride,
            std::mem::size_of::<crate::scene::SceneVertex>(),
            "SCENE_VERTEX_LAYOUT must match SceneVertex field-for-field"
        );
    }
}
