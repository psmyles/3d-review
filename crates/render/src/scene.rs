//! The scene render. The renderer's only entry point is the egui paint
//! [`SceneCallback`] (in [`callback`]); it drives the GPU resource cache
//! [`SceneResources`] (defined here so the callback and the [`resources`] impl
//! can both reach its fields). Pipeline + buffer construction live in
//! [`pipelines`] / [`buffers`], and the WGSL-lockstep GPU types in [`gpu_types`].

use crate::UvShadingMode;
use crate::bloom::BloomPass;
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
    /// every [`IblResources::new`] to rebuild the bind group.
    ibl_layout: wgpu::BindGroupLayout,
    /// Precomputed image-based-lighting maps + their bind group. Rebuilt by
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
    /// Bind group feeding the resolved scene color + blurred bloom to `post`;
    /// rebuilt with `targets`.
    post_bind_group: wgpu::BindGroup,
    /// The bloom bright-pass + blur pass (Phase 4). Size-independent; the half-res
    /// ping-pong textures + bind groups below are rebuilt with `targets`.
    bloom: BloomPass,
    /// Half-resolution HDR ping-pong textures the bloom blur bounces between.
    /// `a` holds the bright-pass output and the final (post-blur) result `post`
    /// samples; `b` is the intermediate. Recreated on resize.
    bloom_tex_a: wgpu::TextureView,
    bloom_tex_b: wgpu::TextureView,
    /// Bloom bind groups: bright-pass reads the scene bloom source, the blur reads
    /// `a` (→ `b`) then `b` (→ `a`). Rebuilt when the bloom textures are.
    bloom_brightpass_bind_group: wgpu::BindGroup,
    bloom_blur_h_bind_group: wgpu::BindGroup,
    bloom_blur_v_bind_group: wgpu::BindGroup,
    /// The GTAO occlusion + blur pass (Phase 5). Size-independent; the full-res AO
    /// textures + bind groups below are rebuilt with `targets`.
    gtao: GtaoPass,
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
    /// blur reads that same G-buffer plus `raw`. Rebuilt when the AO textures /
    /// G-buffer are.
    gtao_bind_group: wgpu::BindGroup,
    gtao_blur_bind_group: wgpu::BindGroup,
    model_revision: u64,
    mesh_uv_channel: u32,
    /// Default mesh pipeline: back faces culled (only camera-facing surfaces
    /// drawn). Used when [`SceneDebugOptions::render_backfaces`] is off.
    mesh_pipeline: wgpu::RenderPipeline,
    /// Double-sided mesh pipeline: no face culling, so back faces are drawn too.
    /// Used when [`SceneDebugOptions::render_backfaces`] is on. Built alongside
    /// `mesh_pipeline` and rebuilt with it on MSAA change.
    mesh_pipeline_double_sided: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    /// Flat-color triangle pipeline for the selection-highlight flash: depth-tested
    /// (Reversed-Z `GreaterEqual`) but no depth write, MRT like the mesh, drawing
    /// `fs_selection` (uniform highlight color × flash fade).
    selection_fill_pipeline: wgpu::RenderPipeline,
    /// Mesh-only pipeline that writes the single-sample GTAO normal/depth buffer.
    gtao_gbuffer_pipeline: wgpu::RenderPipeline,
    /// Fullscreen pipeline that draws the environment cubemap as the background.
    skybox_pipeline: wgpu::RenderPipeline,
    /// Flat-color triangle pipeline for the UV island fill: no lighting (the fill
    /// vertices carry a zero normal), no depth write/bias — it sits under the UV
    /// wireframe and is composited by draw order in the 2D viewport.
    uv_fill_pipeline: wgpu::RenderPipeline,
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

/// Baked parameters for the bounding-box view: `(color, visible_only,
/// hidden_nodes)`. The hidden set only changes the box in `visible_only` mode,
/// but baking it unconditionally keeps the comparison a plain value equality.
type BoundingBoxParams = ([f32; 4], bool, Vec<u32>);

/// Baked parameters for a normal-line view: `(length_scale, color, hidden_nodes)`.
/// Compared by value each frame to decide whether the view's buffer is up to
/// date — including the Outliner's hidden set, so a hidden mesh's normal lines
/// drop with the mesh itself.
type NormalParams = (f32, [f32; 4], Vec<u32>);
