//! Per-model GPU state: the mesh buffers, the derived views built on top of
//! them, and the bake key each view is rebuilt against.
//!
//! `SceneGpu` holds an active/idle **pair** of these so the Opt workspace can
//! keep the source and processed meshes both resident and alternate between them
//! within a frame for one `mem::swap` — a single slot would rebuild both meshes
//! on every alternation. Revisions must be unique across every mesh the renderer
//! is handed, since the cache keys on the revision alone.

use review_model::Bounds;

use crate::geometry::deform::DeformLayout;
use crate::material::MaterialDrawRange;
use crate::rhi::{IndexBuffer, VertexBuffer};
use crate::selection::Selection;
use crate::{BoundingBoxScope, MaterialMode, UvShadingMode};

use super::deform_gpu::DeformGpu;

/// Baked parameters for the bounding-box view. Only the inputs the chosen scope
/// depends on are populated (the hidden set for `VisibleOnly`, the selection for
/// `OnlySelection`), so an unrelated change can't rebuild the box.
#[derive(PartialEq)]
pub(super) struct BoundingBoxParams {
    pub(super) color: [f32; 4],
    pub(super) scope: BoundingBoxScope,
    pub(super) hidden: Vec<u32>,
    pub(super) selection: Selection,
    pub(super) selected_nodes: Vec<u32>,
    /// The frame-supplied bounds the All Meshes scope draws (a clip's envelope
    /// while one is selected), so selecting a clip rebuilds the box.
    pub(super) bounds: Option<Bounds>,
}

/// Baked parameters for a normal-line view (face or vertex normals).
#[derive(PartialEq)]
pub(super) struct NormalParams {
    pub(super) length: f32,
    pub(super) color: [f32; 4],
    pub(super) hidden: Vec<u32>,
}

/// Baked parameters for the UV viewport's wireframe and fill: which mesh, which
/// UV set, and which nodes were laid out (the Outliner's node selection and its
/// hidden set — see [`crate::geometry::UvNodeScope`]).
#[derive(PartialEq)]
pub(super) struct UvViewParams {
    pub(super) model_revision: u64,
    pub(super) channel: u32,
    pub(super) selected: Vec<u32>,
    pub(super) hidden: Vec<u32>,
}

impl UvViewParams {
    /// Whether these are the parameters the borrowed frame inputs describe —
    /// compared without allocating, so a steady UV frame rebuilds nothing.
    pub(super) fn matches(
        &self,
        model_revision: u64,
        channel: u32,
        selected: &[u32],
        hidden: &[u32],
    ) -> bool {
        self.model_revision == model_revision
            && self.channel == channel
            && self.selected == selected
            && self.hidden == hidden
    }
}

/// Baked parameters for the UV-seam view. The channel is in the key because the
/// seams themselves change with the UV set, not just their color.
#[derive(PartialEq)]
pub(super) struct UvSeamParams {
    pub(super) color: [f32; 4],
    pub(super) channel: u32,
    pub(super) hidden: Vec<u32>,
}

/// Baked parameters for the pivot marker: the pivot position + half-length, both
/// derived from the model, so a model swap (new pivot / size) rebuilds it while an
/// unrelated change does not.
#[derive(PartialEq)]
pub(super) struct PivotParams {
    pub(super) pivot: [f32; 3],
    pub(super) half: f32,
}

/// Opacity of the skeleton's solid octahedron fills, as a multiplier on the bone
/// color's own alpha. Low enough that the character reads through the rig, high
/// enough that a bone's volume and orientation are legible.
pub(super) const SKELETON_FILL_ALPHA: f32 = 0.35;

/// Baked parameters for the skeleton overlay. The selected set is part of the key
/// because the highlight color is baked per bone into the vertex buffer (unlike the
/// mesh selection highlight, whose color rides in a uniform) — a skeleton is a few
/// thousand vertices, so rebuilding it on an Outliner click is far cheaper than
/// carrying a per-bone lookup into the shader.
#[derive(PartialEq)]
pub(super) struct SkeletonParams {
    pub(super) model_revision: u64,
    pub(super) selected: Vec<u32>,
    /// The hovered bone, which tints like a selected one. In the key because the
    /// tint is baked per bone into the vertex colour — so moving the pointer from
    /// one bone to the next does rebuild these buffers, which is the same trade
    /// the selected set already makes and just as cheap at a few thousand
    /// vertices.
    pub(super) hovered: Option<u32>,
    pub(super) scale: f32,
    pub(super) color: [f32; 4],
    pub(super) selected_color: [f32; 4],
    pub(super) hover_color: [f32; 4],
}

/// Bake key for the skin-weight heat map. Only the model and the selected bone
/// set change its bytes — the ramp is fixed, and the neutral base it blends from
/// is a shader constant.
#[derive(PartialEq)]
pub(super) struct SkinWeightParams {
    pub(super) model_revision: u64,
    pub(super) selected: Vec<u32>,
}

/// Bake key for the hover draw list. The same inputs as the selection's, over one
/// node — the tint itself rides in the uniform, so only a change of *which* node is
/// hovered rebuilds anything.
#[derive(PartialEq)]
pub(super) struct HoverBaked {
    pub(super) model_revision: u64,
    pub(super) node: u32,
    pub(super) hidden: Vec<u32>,
    pub(super) mode: MaterialMode,
}

/// Bake key for the selection draw list — the hidden set because hiding a
/// selected mesh drops it, the mode because Unique re-groups the solo list by
/// part.
#[derive(PartialEq)]
pub(super) struct SelectionBaked {
    pub(super) model_revision: u64,
    pub(super) selection: Selection,
    pub(super) selected_nodes: Vec<u32>,
    pub(super) hidden: Vec<u32>,
    pub(super) mode: MaterialMode,
}

/// Bake key for the per-mesh visibility draw list.
#[derive(PartialEq)]
pub(super) struct VisibilityBaked {
    pub(super) model_revision: u64,
    pub(super) hidden: Vec<u32>,
    pub(super) mode: MaterialMode,
}

/// The mesh's GPU buffers + per-material draw ranges, rebuilt when the model (or UV
/// channel / material mode) changes. `None` for an empty model (the grid still draws).
pub(super) struct MeshBuffers {
    pub(super) vertices: VertexBuffer,
    pub(super) indices: IndexBuffer,
    pub(super) ranges: Vec<MaterialDrawRange>,
    /// The GPU deform tables, `None` for a model that never deforms.
    pub(super) deform: Option<DeformGpu>,
}

/// The build-on-demand derived views (invariant 3): each buffer exists only while
/// its toggle is on, paired with the bake key it was last built for. The 3D line
/// views draw with the shared `line_pipeline` (the pivot marker with the
/// always-on-top `line_overlay_pipeline`); the UV pair belongs to the 2D UV
/// viewport.
#[derive(Default)]
pub(super) struct DerivedViews {
    /// Model wireframe (original-polygon edges) as a `LineList` **index buffer
    /// over the mesh's own vertex buffer** — 8 bytes per edge, not 160
    /// (`wireframe_edge_indices`). Its colour is a uniform, so the key is the
    /// Outliner's hidden set alone.
    ///
    /// **The one documented exception to invariant 3**: this buffer is *not*
    /// freed when the wireframe is switched off. Toggling the overlay (or the
    /// Shaded+Wireframe mode) is a per-second UI gesture on the very models
    /// where rebuilding it is most expensive, and re-deriving it on each flip is
    /// a visible stall. Indexing the shared vertex buffer is what makes keeping
    /// it affordable: a 3M-corner asset retains ~24 MB rather than ~480 MB. It
    /// is still dropped with the rest of `DerivedViews` when the model changes
    /// (see `ModelSlot::views_revision`), so nothing outlives its mesh.
    pub(super) wireframe_index: Option<IndexBuffer>,
    pub(super) wireframe_baked: Option<Vec<u32>>,
    /// Axis-aligned bounding box; `None` while off or when the scope wraps no
    /// geometry.
    pub(super) bounding_box_buf: Option<VertexBuffer>,
    pub(super) bounding_box_baked: Option<BoundingBoxParams>,
    /// One line per face along its normal; `None` while off.
    pub(super) face_normal_buf: Option<VertexBuffer>,
    pub(super) face_baked: Option<NormalParams>,
    /// One line per vertex along its normal; `None` while off.
    pub(super) vertex_normal_buf: Option<VertexBuffer>,
    pub(super) vertex_baked: Option<NormalParams>,
    /// One line per UV-seam edge; `None` while off.
    pub(super) uv_seam_buf: Option<VertexBuffer>,
    pub(super) uv_seam_baked: Option<UvSeamParams>,
    /// 3-axis pivot marker at the model's origin; `None` while off. The bake key
    /// is the pivot position + half-length (both model-derived).
    pub(super) pivot_buf: Option<VertexBuffer>,
    pub(super) pivot_baked: Option<PivotParams>,
    /// Vertex buffer parallel to the mesh's own, colored by the selected bones'
    /// influence, drawn *in place of* the mesh while the Skin Weights material is
    /// active. `None` in every other mode (invariant 3).
    pub(super) weights_buf: Option<VertexBuffer>,
    pub(super) weights_baked: Option<SkinWeightParams>,
    /// The skeleton overlay's solid octahedron fills and their outlines; both
    /// `None` while the toggle is off. Drawn always-on-top (X-ray).
    pub(super) skeleton_fill_buf: Option<VertexBuffer>,
    pub(super) skeleton_line_buf: Option<VertexBuffer>,
    pub(super) skeleton_baked: Option<SkeletonParams>,
    /// The model's UV edges for the active channel and the laid-out nodes.
    pub(super) uv_wireframe_buf: Option<VertexBuffer>,
    pub(super) uv_wireframe_baked: Option<UvViewParams>,
    /// The UV island fill (Shaded / Islands modes only); `None` in Wire mode.
    /// Built per the same parameters as the wireframe, plus the shading mode and
    /// whether it is dimmed over a texture.
    pub(super) uv_fill_buf: Option<VertexBuffer>,
    pub(super) uv_fill_baked: Option<(UvViewParams, UvShadingMode, bool)>,
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
pub(super) struct ModelSlot {
    /// `None` for an empty model (the grid still draws).
    pub(super) mesh: Option<MeshBuffers>,
    /// The model's deform layout (built with the mesh, keyed by the same
    /// revision) — the per-corner lanes every mesh-derived overlay copies.
    /// `None` for a model that never deforms.
    pub(super) deform_layout: Option<DeformLayout>,
    pub(super) mesh_revision: u64,
    pub(super) mesh_uv_channel: u32,
    pub(super) mesh_material_mode: MaterialMode,
    /// Cached Unique-mode per-triangle mesh-part key + count, baked by model
    /// revision while Unique is active (invariant 3: freed otherwise).
    pub(super) unique_part_key: Vec<u32>,
    pub(super) unique_part_count: usize,
    pub(super) unique_baked: Option<u64>,
    /// The build-on-demand derived line/fill views (invariant 3) — each buffer
    /// exists only while its toggle is on, paired with the bake key it was last
    /// built for.
    pub(super) views: DerivedViews,
    /// The model revision `views` were derived from.
    ///
    /// Their own bake keys describe only the *parameters* they were built with —
    /// a color, a hidden set, a normal length — and say nothing about which mesh
    /// the geometry came from. So the revision is tracked here and the whole set
    /// dropped when it moves: otherwise a mesh replaced underneath them (a newly
    /// loaded file, or the Opt workspace reprocessing) keeps the previous mesh's
    /// wireframe and normal lines drawn over the new one.
    pub(super) views_revision: u64,
    /// The selected triangles reordered per-material over a fresh index buffer that
    /// shares the mesh vertex buffer (the solo isolate list + the highlight fill
    /// source). `None` while nothing is selected or the selection resolves to no
    /// geometry (invariant 3).
    pub(super) selection_index: Option<IndexBuffer>,
    pub(super) selection_ranges: Vec<MaterialDrawRange>,
    pub(super) selection_baked: Option<SelectionBaked>,
    /// The hovered node's triangles, over the shared mesh vertex buffer. Built
    /// when the pointer enters a node and freed when it leaves (invariant 3), so
    /// a viewer not in Select mode holds nothing.
    pub(super) hover_index: Option<IndexBuffer>,
    pub(super) hover_baked: Option<HoverBaked>,
    /// The visible (non-hidden) triangles reordered per-material over a fresh index
    /// buffer sharing the mesh vertex buffer; drawn instead of the full mesh while
    /// `visible_active`. `None` when nothing is hidden (full mesh), or when every
    /// mesh is hidden (active but empty → draw nothing).
    pub(super) visible_index: Option<IndexBuffer>,
    pub(super) visible_ranges: Vec<MaterialDrawRange>,
    /// Whether the per-mesh visibility filter is in effect (some mesh hidden and the
    /// model carries per-triangle node info): the mesh + GTAO passes then draw the
    /// filtered list instead of the full mesh.
    pub(super) visible_active: bool,
    pub(super) visibility_baked: Option<VisibilityBaked>,
    /// Bumped whenever the visibility draw list is rebuilt, so the AO accumulation
    /// can tell "the hidden set moved" from "it didn't" by comparing one integer.
    /// The set itself is a `Vec` and the accumulation key is compared every frame,
    /// so keying on the generation is what keeps that comparison allocation-free
    /// and O(1).
    pub(super) visibility_generation: u64,
    /// This model's wireframe edge indices as drawn when it is the *ghost* in the
    /// Opt workspace's overlay view — the same `LineList`-over-the-mesh form as
    /// `views.wireframe_index`, over *this* slot's vertex buffer. Kept separate
    /// from that one because the ghost is a different slot's mesh and exists
    /// regardless of the user's wireframe toggle; the colour no longer divides
    /// them (both read it from the uniform). `None` whenever this model is not
    /// currently the ghost (invariant 3).
    pub(super) ghost_wireframe_index: Option<IndexBuffer>,
    pub(super) ghost_wireframe_baked: Option<(u64, Vec<u32>)>,
}

impl ModelSlot {
    pub(super) fn new() -> Self {
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
    pub(super) fn has_mesh(&self) -> bool {
        self.mesh.is_some()
    }

    /// Drop every cached buffer and bake key, so the next sync rebuilds from
    /// scratch. Used when a slot's model goes away — the GPU resources are
    /// released by the drop, no explicit teardown needed (invariant 3).
    ///
    /// The visibility generation is carried across rather than reset: it is a
    /// change *detector*, so it has to keep moving forward. Resetting it to 0 could
    /// hand the AO accumulation the number it already held and let a converged
    /// result survive the model it was computed for.
    pub(super) fn release(&mut self) {
        let generation = self.visibility_generation;
        *self = ModelSlot::new();
        self.visibility_generation = generation.wrapping_add(1);
    }

    /// Drop everything derived from the mesh - the line views, the selection,
    /// hover and visibility lists, the ghost wireframe - but keep the mesh
    /// itself (invariant 3). For a slot that is uploaded but not being drawn as
    /// a scene: the Opt workspace's processed mesh while another workspace is up,
    /// or whichever mesh is the overlay's ghost, which is drawn from its mesh
    /// buffers alone.
    ///
    /// Every bake key goes with its buffer, so the next frame that draws this
    /// slot as a scene rebuilds exactly what it needs; the visibility list's
    /// rebuild bumps its own generation, which is what resets the AO then.
    pub(super) fn release_derived(&mut self) {
        self.views = DerivedViews::default();
        self.selection_index = None;
        self.selection_ranges = Vec::new();
        self.selection_baked = None;
        self.hover_index = None;
        self.hover_baked = None;
        self.visible_index = None;
        self.visible_ranges = Vec::new();
        self.visible_active = false;
        self.visibility_baked = None;
        self.ghost_wireframe_index = None;
        self.ghost_wireframe_baked = None;
    }

    /// The per-corner deform lanes the mesh-derived builders copy (empty for a
    /// model that never deforms).
    pub(super) fn lanes(&self) -> &[[u32; 4]] {
        self.deform_layout
            .as_ref()
            .map_or(&[], |layout| layout.corner.as_slice())
    }

    /// Whether this frame's mesh draws through the deform path: the model has
    /// deform tables and a matching pose has been uploaded.
    pub(super) fn deform_enabled(&self) -> bool {
        self.mesh
            .as_ref()
            .and_then(|mesh| mesh.deform.as_ref())
            .is_some_and(|deform| deform.palette_revision.is_some())
    }
}
