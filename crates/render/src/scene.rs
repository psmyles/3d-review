//! The scene render. The renderer's only entry point is the egui paint
//! [`SceneCallback`] (in [`callback`]); it drives the GPU resource cache
//! [`SceneResources`] (defined here so the callback and the [`resources`] impl
//! can both reach its fields). Pipeline + buffer construction live in
//! [`pipelines`] / [`buffers`], and the WGSL-lockstep GPU types in [`gpu_types`].

use crate::config::BoundingBoxScope;
use crate::UvShadingMode;
use crate::gtao::GtaoPass;
use crate::ibl::IblResources;
use crate::material::{MaterialDrawRange, MaterialTable};
use crate::post::PostPass;
use crate::selection::Selection;
use crate::targets::SceneTargets;

mod buffers;
mod callback;
mod gpu_types;
mod pipelines;
mod resources;

pub use callback::SceneCallback;
pub(crate) use gpu_types::SceneVertex;
use pipelines::ScenePipelines;

pub const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Depth format of egui's own framebuffer. The scene uses `Depth32Float`
/// Reversed-Z offscreen, but egui's pass only needs a conventional attachment so
/// the fullscreen composite pipeline can match the painter.
pub const EGUI_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
/// MSAA sample count of egui's own framebuffer (the surface the composite pass
/// draws into). Fixed: it antialiases the egui chrome and is what the painter is
/// created with in `app`. Distinct from the scene's MSAA, which is dynamic
/// ([`AntiAliasing::msaa`]) and resolved to a single-sample texture before this
/// pass ever runs — so the two sample counts are deliberately independent.
pub const EGUI_MSAA_SAMPLE_COUNT: u32 = 4;

/// Background the offscreen scene target is cleared to each frame. Black in both
/// gamma and linear, so it matches the previous direct-to-egui clear regardless
/// of the target's color space. (The post pass overwrites the whole framebuffer,
/// so egui's own clear color no longer shows through in the scene region.)
const SCENE_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

struct SceneResources {
    output_format: wgpu::TextureFormat,
    /// MSAA sample count the scene pipelines + targets are currently built for.
    /// Rebuilt when the chosen [`AntiAliasing::msaa`] level changes.
    scene_sample_count: u32,
    /// Retained so the scene pipelines can be rebuilt when the MSAA level changes
    /// (pipeline sample count is baked at creation).
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    /// Layout of the IBL bind group (group 2), embedded in `pipeline_layout`.
    /// Stable across environment switches so the pipelines stay valid; passed to
    /// every [`IblResources::from_baked`] to rebuild the bind group.
    ibl_layout: wgpu::BindGroupLayout,
    /// Baked image-based-lighting maps + their bind group. Reloaded by
    /// `sync_environment` only when the chosen environment changes.
    ibl: IblResources,
    /// Layout of the material bind group (group 3), embedded in `pipeline_layout`.
    /// Passed to [`MaterialTable::sync`] to rebuild the table's bind group when a
    /// new model changes the material count.
    material_layout: wgpu::BindGroupLayout,
    /// Min uniform-buffer offset alignment the material table strides to.
    material_alignment: u64,
    /// Editable per-material PBR uniform table (group 3), seeded from import
    /// defaults and re-uploaded on edit by `sync_materials`.
    material_table: MaterialTable,
    /// Per-material draw ranges over the reordered mesh index buffer; one draw per
    /// entry. Rebuilt with the mesh in `update_model` / `update_mesh_channel`.
    material_ranges: Vec<MaterialDrawRange>,
    /// Last material revision uploaded into `material_table`; compared against the
    /// callback's to drive re-uploads on edit (separate from `model_revision`).
    material_revision: u64,
    /// Selected-triangle index buffer: the selection reordered grouped by material,
    /// sharing `mesh_vertex_buffer`. Drawn via `selection_ranges` when solo is on
    /// (the isolate view), and redrawn whole by the highlight flash (a flat color
    /// fill over `selection_index_count` indices). Holds a placeholder while
    /// nothing is selected.
    selection_index_buffer: wgpu::Buffer,
    /// Per-material draw ranges over `selection_index_buffer` (the solo draw list).
    selection_ranges: Vec<MaterialDrawRange>,
    /// Total selected indices in `selection_index_buffer`; the flash draws them all
    /// in one call (material is irrelevant to the flat fill). 0 while nothing is
    /// selected (the buffer then holds only a placeholder index).
    selection_index_count: u32,
    /// `(model_revision, selection, sorted hidden meshes)` baked into the selection
    /// buffer, or `None` while nothing is selected — compared each frame to drive
    /// build / free (invariant 3). The hidden set is part of the key because hiding
    /// a selected mesh must drop it from the solo list + highlight flash. The
    /// highlight color + flash fade ride in the uniform, not the geometry, so they
    /// are not part of the key.
    selection_baked: Option<(u64, Selection, Vec<u32>)>,
    /// Per-mesh visibility draw list: the visible triangles reordered grouped by
    /// material, sharing `mesh_vertex_buffer`. Drawn instead of the full mesh
    /// whenever `visible_active`. Holds a placeholder while nothing is hidden.
    visible_index_buffer: wgpu::Buffer,
    /// Per-material draw ranges over `visible_index_buffer`.
    visible_ranges: Vec<MaterialDrawRange>,
    /// Total indices in `visible_index_buffer` (0 when every mesh is hidden).
    visible_index_count: u32,
    /// Whether the visibility filter is in effect (some mesh hidden and the model
    /// carries per-triangle node info): when set, the mesh / GTAO passes draw
    /// `visible_index_buffer` instead of the full mesh.
    visible_active: bool,
    /// `(model_revision, sorted hidden mesh nodes)` baked into the visibility
    /// buffer, or `None` while nothing is hidden — compared each frame to drive
    /// build / free (invariant 3).
    visibility_baked: Option<(u64, Vec<u32>)>,
    /// Offscreen HDR color + depth the scene renders into, recreated on resize or
    /// MSAA change.
    targets: SceneTargets,
    /// The fullscreen composite pass (offscreen scene → egui's framebuffer).
    post: PostPass,
    /// Bind group feeding the resolved scene color + blurred AO + ambient radiance
    /// to `post`; rebuilt with `targets`.
    post_bind_group: wgpu::BindGroup,
    /// The GTAO occlusion + blur pass (Phase 5). Size-independent; the full-res AO
    /// textures + bind groups below are rebuilt with `targets`. `None` until the
    /// deferred `Gtao` build stage runs (Phase B); until then the composite treats
    /// AO as inactive (an empty startup scene has nothing to occlude anyway).
    gtao: Option<GtaoPass>,
    /// Single-sample GTAO G-buffer (view-space normal + view-space Z). Kept out
    /// of the MSAA scene MRTs so normals/depths are not averaged across geometry
    /// edges before the AO pass samples them.
    gtao_gbuffer_view: wgpu::TextureView,
    /// Reversed-Z depth used only by the single-sample GTAO G-buffer pass.
    gtao_depth_view: wgpu::TextureView,
    /// Full-resolution AO textures: `raw` holds the occlusion pass output, `blur`
    /// the denoised result the composite samples. Recreated on resize.
    gtao_raw_view: wgpu::TextureView,
    gtao_blur_view: wgpu::TextureView,
    /// GTAO bind groups: the occlusion pass reads the single-sample G-buffer, the
    /// blur reads that same G-buffer plus `raw`. `None` until the deferred GTAO
    /// pass is built; rebuilt with it / when the AO textures / G-buffer are.
    gtao_bind_group: Option<wgpu::BindGroup>,
    gtao_blur_bind_group: Option<wgpu::BindGroup>,
    model_revision: u64,
    mesh_uv_channel: u32,
    /// The deferred scene pipelines — mesh (culled + double-sided), UV-fill,
    /// selection-fill, skybox, and the single-sample GTAO G-buffer — grouped so
    /// they build together off the first frame (Phase B). `None` until the
    /// `ScenePipelines` build stage runs; the grid-only first frame draws without
    /// them, and a mesh / skybox / UV-fill / selection draw is skipped while they
    /// are absent. Rebuilt (the MSAA-dependent subset) on antialiasing change.
    scene_pipelines: Option<ScenePipelines>,
    /// Line-list pipeline for the grid + every line overlay (wireframe, bounding
    /// box, face/vertex normals). Part of the core build, since the grid-only
    /// first frame draws with it; rebuilt on MSAA change.
    line_pipeline: wgpu::RenderPipeline,
    /// Cursor over the deferred GPU-resource build (Phase B), advanced one stage
    /// per frame by [`SceneResources::advance_build`].
    build_stage: BuildStage,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    checker_bind_group_greyscale: wgpu::BindGroup,
    checker_bind_group_color: wgpu::BindGroup,
    mesh_vertex_buffer: wgpu::Buffer,
    mesh_index_buffer: wgpu::Buffer,
    mesh_index_count: u32,
    line_vertex_buffer: wgpu::Buffer,
    line_vertex_count: u32,
    // Derived line views. Each `*_baked` is `Some(params)` while that view is
    // built and `None` while it is off (its buffer holds only a placeholder).
    // Comparing against the current options drives build / rebuild / free in
    // `sync_line_views` (invariant 3).
    wireframe_line_vertex_buffer: wgpu::Buffer,
    wireframe_line_vertex_count: u32,
    /// Color baked into the wireframe buffer, or `None` when the view is off.
    wireframe_baked: Option<([f32; 4], Vec<u32>)>,
    bounding_box_vertex_buffer: wgpu::Buffer,
    bounding_box_vertex_count: u32,
    /// Color baked into the bounding-box buffer, or `None` when the view is off.
    bounding_box_baked: Option<BoundingBoxParams>,
    face_normal_vertex_buffer: wgpu::Buffer,
    face_normal_vertex_count: u32,
    /// `(length_scale, color)` baked into the face-normal buffer, or `None`.
    face_baked: Option<NormalParams>,
    vertex_normal_vertex_buffer: wgpu::Buffer,
    vertex_normal_vertex_count: u32,
    /// `(length_scale, color)` baked into the vertex-normal buffer, or `None`.
    vertex_baked: Option<NormalParams>,
    // UV viewport buffers. The 0..1 grid is static (built once); the UV
    // wireframe is built on demand for the active `(model_revision, channel)`
    // and freed when the 3D scene is shown again (invariant 3).
    uv_grid_vertex_buffer: wgpu::Buffer,
    uv_grid_vertex_count: u32,
    uv_wireframe_vertex_buffer: wgpu::Buffer,
    uv_wireframe_vertex_count: u32,
    /// `(model_revision, channel)` baked into the UV wireframe, or `None` when
    /// the view is off (its buffer holds only a placeholder).
    uv_baked: Option<(u64, u32)>,
    // UV island fill. Built on demand for the active `(model_revision, channel,
    // shading_mode)` when the mode draws a fill (Shaded / Islands), and freed
    // back to a placeholder in Wire mode or when the 3D scene is shown.
    uv_fill_vertex_buffer: wgpu::Buffer,
    uv_fill_vertex_count: u32,
    /// `(model_revision, channel, shading_mode)` baked into the UV fill, or
    /// `None` when no fill is drawn (its buffer holds only a placeholder).
    uv_fill_baked: Option<(u64, u32, UvShadingMode)>,
}

/// Baked parameters for the bounding-box view: `(color, scope, hidden_nodes,
/// selection)`. Only the inputs the chosen scope depends on are populated (the
/// hidden set for `VisibleOnly`, the selection for `OnlySelection`); the rest
/// are left at their neutral value so an unrelated change can't rebuild the box.
type BoundingBoxParams = ([f32; 4], BoundingBoxScope, Vec<u32>, Selection);

/// Baked parameters for a normal-line view: `(length_scale, color, hidden_nodes)`.
/// Compared by value each frame to decide whether the view's buffer is up to
/// date — including the Outliner's hidden set, so a hidden mesh's normal lines
/// drop with the mesh itself.
type NormalParams = (f32, [f32; 4], Vec<u32>);

/// Cursor over the deferred GPU-resource build (Phase B). After the cheap core
/// build ([`SceneResources::new_core`]) presents the grid-only first frame,
/// [`SceneResources::advance_build`] walks these stages one per frame so the heavy
/// pipeline compilation is spread across the first few frames behind the startup
/// warmup, instead of all landing on frame 1. IBL stays in the core build (it is a
/// cheap baked-map upload, not a precompute), so it is not a stage here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildStage {
    /// Build the deferred [`ScenePipelines`] (the five MSAA-dependent scene
    /// pipelines + the GTAO G-buffer) — needed before a mesh / skybox / UV-fill /
    /// selection can draw, so this runs first.
    ScenePipelines,
    /// Build the GTAO effect pass + its bind groups. A no-op on an empty startup
    /// scene (nothing to occlude), so it builds last.
    Gtao,
    /// Everything built; [`SceneResources::advance_build`] is a no-op.
    Done,
}
