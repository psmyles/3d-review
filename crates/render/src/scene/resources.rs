//! Everything the scene renderer caches *per model*: [`ModelSlot`] — the uploaded
//! mesh buffers, the build-on-demand derived views and the selection / visibility
//! draw lists, each with the bake key it was built for — and the `sync_*` builders
//! that reconcile them with the live frame.
//!
//! Invariant 3 lives here: a derived view's buffer exists only while its toggle is
//! on, is rebuilt when its baked parameters drift, and is dropped the moment the
//! toggle goes off, so the steady-state shaded view holds no derived buffers. The
//! passes that *draw* these buffers are in [`super::d3d`].

use review_model::{Bounds, DeformPose, ModelData};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::geometry::deform::DeformLayout;
use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_mesh, model_pivot, pivot_half_extent, pivot_lines,
    selection_geometry, skeleton_fill_triangles, skeleton_lines, skin_weight_vertices,
    uv_fill_triangles, uv_wireframe_lines, vertex_normal_lines, visible_geometry, wireframe_lines,
};
use crate::material::{MaterialDrawRange, build_part_key};
use crate::rhi::{IndexBuffer, StructuredBuffer, VertexBuffer};
use crate::selection::{Selection, SelectionView, selection_bounds};
use crate::{
    ActiveMaterial, BoundingBoxScope, MaterialMode, SceneDebugOptions, ShadingMode, UvShadingMode,
};

use super::d3d::SceneGpu;
use super::gpu_types::{InfluenceEntry, MorphEntry, PaletteEntry};

/// Baked parameters for the bounding-box view. Only the inputs the chosen scope
/// depends on are populated (the hidden set for `VisibleOnly`, the selection for
/// `OnlySelection`), so an unrelated change can't rebuild the box.
#[derive(PartialEq)]
struct BoundingBoxParams {
    color: [f32; 4],
    scope: BoundingBoxScope,
    hidden: Vec<u32>,
    selection: Selection,
    /// The frame-supplied bounds the All Meshes scope draws (a clip's envelope
    /// while one is selected), so selecting a clip rebuilds the box.
    bounds: Option<Bounds>,
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
pub(super) struct MeshBuffers {
    pub(super) vertices: VertexBuffer,
    pub(super) indices: IndexBuffer,
    pub(super) ranges: Vec<MaterialDrawRange>,
    /// The GPU deform tables, `None` for a model that never deforms.
    pub(super) deform: Option<DeformGpu>,
}

/// The vertex shader's deform inputs for one model (`t12..t15`): the immutable
/// influence + blend-shape tables built with the mesh, and the dynamic palette +
/// shape weights re-uploaded when the pose revision moves.
pub(super) struct DeformGpu {
    pub(super) influences: StructuredBuffer<InfluenceEntry>,
    /// `None` when the model has no blend shapes; the shader never reads it then
    /// (every morph lane is empty).
    pub(super) morph: Option<StructuredBuffer<MorphEntry>>,
    pub(super) palette: StructuredBuffer<PaletteEntry>,
    pub(super) shape_weights: Option<StructuredBuffer<f32>>,
    /// The `pose_revision` the palette currently holds; `None` until a pose has
    /// been uploaded (the deform flag stays off until then).
    pub(super) palette_revision: Option<u64>,
    /// Scratch for the palette conversion, kept so a pose change allocates nothing.
    palette_scratch: Vec<PaletteEntry>,
}

impl DeformGpu {
    fn new(device: &ID3D11Device, layout: &DeformLayout) -> windows::core::Result<Option<Self>> {
        if layout.influences.is_empty() || layout.palette_len == 0 {
            return Ok(None);
        }
        let morph = if layout.morph.is_empty() {
            None
        } else {
            Some(StructuredBuffer::immutable(device, &layout.morph)?)
        };
        let shape_weights = if layout.shape_count == 0 {
            None
        } else {
            Some(StructuredBuffer::dynamic(device, layout.shape_count)?)
        };
        Ok(Some(Self {
            influences: StructuredBuffer::immutable(device, &layout.influences)?,
            morph,
            palette: StructuredBuffer::dynamic(device, layout.palette_len)?,
            shape_weights,
            palette_revision: None,
            palette_scratch: Vec::with_capacity(layout.palette_len),
        }))
    }

    /// Whether `pose` fits these tables — a pose built for another model would
    /// index past the palette, so it is refused rather than clamped.
    fn accepts(&self, pose: &DeformPose) -> bool {
        pose.palette.len() == self.palette.capacity()
            && pose.shape_weights.len() == self.shape_weights.as_ref().map_or(0, |b| b.capacity())
    }

    /// Upload `pose` (palette rows + shape weights) when `revision` moved.
    fn upload(
        &mut self,
        ctx: &ID3D11DeviceContext,
        pose: &DeformPose,
        revision: u64,
    ) -> windows::core::Result<()> {
        if self.palette_revision == Some(revision) {
            return Ok(());
        }
        self.palette_scratch.clear();
        self.palette_scratch.extend(
            pose.palette
                .iter()
                .map(|matrix| PaletteEntry::from_mat4(*matrix)),
        );
        self.palette.update(ctx, &self.palette_scratch)?;
        if let Some(weights) = &self.shape_weights {
            weights.update(ctx, &pose.shape_weights)?;
        }
        self.palette_revision = Some(revision);
        Ok(())
    }

    /// Bind the four tables to the vertex stage.
    pub(super) fn bind_vs(&self, ctx: &ID3D11DeviceContext) {
        self.influences.bind_vs(ctx, DEFORM_INFLUENCES_SLOT);
        self.palette.bind_vs(ctx, DEFORM_PALETTE_SLOT);
        if let Some(morph) = &self.morph {
            morph.bind_vs(ctx, DEFORM_MORPH_SLOT);
        }
        if let Some(weights) = &self.shape_weights {
            weights.bind_vs(ctx, DEFORM_WEIGHTS_SLOT);
        }
    }
}

/// The vertex-stage resource slots of the deform tables (`scene.hlsl`
/// `t12..t15`).
pub(super) const DEFORM_INFLUENCES_SLOT: u32 = 12;
const DEFORM_PALETTE_SLOT: u32 = 13;
const DEFORM_MORPH_SLOT: u32 = 14;
const DEFORM_WEIGHTS_SLOT: u32 = 15;
/// How many consecutive slots [`DeformGpu::bind_vs`] touches, for the unbind.
pub(super) const DEFORM_SLOT_COUNT: usize = 4;

/// The build-on-demand derived views (invariant 3): each buffer exists only while
/// its toggle is on, paired with the bake key it was last built for. The 3D line
/// views draw with the shared `line_pipeline` (the pivot marker with the
/// always-on-top `line_overlay_pipeline`); the UV pair belongs to the 2D UV
/// viewport.
#[derive(Default)]
pub(super) struct DerivedViews {
    /// Model wireframe (original-polygon edges); the key is `(color, hidden)`.
    pub(super) wireframe_buf: Option<VertexBuffer>,
    wireframe_baked: Option<([f32; 4], Vec<u32>)>,
    /// Axis-aligned bounding box; `None` while off or when the scope wraps no
    /// geometry.
    pub(super) bounding_box_buf: Option<VertexBuffer>,
    bounding_box_baked: Option<BoundingBoxParams>,
    /// One line per face along its normal; `None` while off.
    pub(super) face_normal_buf: Option<VertexBuffer>,
    face_baked: Option<NormalParams>,
    /// One line per vertex along its normal; `None` while off.
    pub(super) vertex_normal_buf: Option<VertexBuffer>,
    vertex_baked: Option<NormalParams>,
    /// 3-axis pivot marker at the model's origin; `None` while off. The bake key
    /// is the pivot position + half-length (both model-derived).
    pub(super) pivot_buf: Option<VertexBuffer>,
    pivot_baked: Option<PivotParams>,
    /// Vertex buffer parallel to the mesh's own, colored by the selected bones'
    /// influence, drawn *in place of* the mesh while the Skin Weights material is
    /// active. `None` in every other mode (invariant 3).
    pub(super) weights_buf: Option<VertexBuffer>,
    weights_baked: Option<SkinWeightParams>,
    /// The skeleton overlay's solid octahedron fills and their outlines; both
    /// `None` while the toggle is off. Drawn always-on-top (X-ray).
    pub(super) skeleton_fill_buf: Option<VertexBuffer>,
    pub(super) skeleton_line_buf: Option<VertexBuffer>,
    skeleton_baked: Option<SkeletonParams>,
    /// The model's UV edges for the active channel; built per
    /// `(model_revision, channel)`.
    pub(super) uv_wireframe_buf: Option<VertexBuffer>,
    uv_wireframe_baked: Option<(u64, u32)>,
    /// The UV island fill (Shaded / Islands modes only); `None` in Wire mode.
    /// Built per `(model_revision, channel, shading_mode)`.
    pub(super) uv_fill_buf: Option<VertexBuffer>,
    uv_fill_baked: Option<(u64, u32, UvShadingMode)>,
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
    deform_layout: Option<DeformLayout>,
    mesh_revision: u64,
    mesh_uv_channel: u32,
    mesh_material_mode: MaterialMode,
    /// Cached Unique-mode per-triangle mesh-part key + count, baked by model
    /// revision while Unique is active (invariant 3: freed otherwise).
    unique_part_key: Vec<u32>,
    pub(super) unique_part_count: usize,
    unique_baked: Option<u64>,
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
    views_revision: u64,
    /// The selected triangles reordered per-material over a fresh index buffer that
    /// shares the mesh vertex buffer (the solo isolate list + the flash fill source).
    /// `None` while nothing is selected or the selection resolves to no geometry
    /// (invariant 3).
    pub(super) selection_index: Option<IndexBuffer>,
    pub(super) selection_ranges: Vec<MaterialDrawRange>,
    selection_baked: Option<SelectionBaked>,
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
    visibility_baked: Option<VisibilityBaked>,
    /// This model's wireframe as drawn when it is the *ghost* in the Opt
    /// workspace's overlay view. Deliberately separate from `views.wireframe_buf`
    /// rather than reusing it: that one is owned by the user's wireframe toggle
    /// and coloured by it, so sharing would have the two rebuild the same buffer
    /// in opposite directions every frame. `None` whenever this model is not
    /// currently the ghost (invariant 3).
    pub(super) ghost_wireframe_buf: Option<VertexBuffer>,
    pub(super) ghost_wireframe_baked: Option<(u64, Vec<u32>, [f32; 4])>,
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
    pub(super) fn release(&mut self) {
        *self = ModelSlot::new();
    }

    /// The per-corner deform lanes the mesh-derived builders copy (empty for a
    /// model that never deforms).
    fn lanes(&self) -> &[[u32; 4]] {
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

impl SceneGpu {
    /// Free both slots' ghost wireframes (invariant 3) — the overlay view is no
    /// longer showing one.
    pub(super) fn release_ghost_wireframes(&mut self) {
        for slot in [&mut self.active, &mut self.idle] {
            slot.ghost_wireframe_buf = None;
            slot.ghost_wireframe_baked = None;
        }
    }

    /// Free both slots' UV viewport buffers (invariant 3) — this frame is a 3D one,
    /// so nothing draws them.
    ///
    /// The pair is the counterpart to [`Self::sync_uv_view`]'s build arm, and the
    /// only derived view whose builder is reached from a *different* entry point
    /// than the 3D `sync_frame`: without a free arm here one visit to the UV tab
    /// left a full-model edge list (two 64-byte vertices per UV edge — tens of
    /// megabytes on a real asset) plus its island fill resident for the rest of the
    /// session, since the bake keys alone are only invalidated by a model swap.
    pub(super) fn release_uv_views(&mut self) {
        for slot in [&mut self.active, &mut self.idle] {
            slot.views.uv_wireframe_buf = None;
            slot.views.uv_wireframe_baked = None;
            slot.views.uv_fill_buf = None;
            slot.views.uv_fill_baked = None;
        }
    }

    /// Build-on-demand for the UV viewport's derived buffers (invariant 3): the
    /// wireframe is rebuilt only when the model / channel changes; the island fill
    /// when the model / channel / shading mode changes, and freed in Wire mode — so
    /// panning/zooming rebuilds nothing. Leaving the viewport frees both, in
    /// [`Self::release_uv_views`].
    pub(super) fn sync_uv_view(
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
    pub(super) fn sync_unique_parts(
        &mut self,
        model: &ModelData,
        model_revision: u64,
        mode: MaterialMode,
    ) {
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
    pub(super) fn sync_mesh(
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
        // The deform layout is a property of the model alone; rebuild it only on
        // a model change, not on a UV-channel / material-mode switch.
        if self.active.mesh_revision != model_revision {
            self.active.deform_layout = DeformLayout::build(model);
        }
        let (vertices, indices, ranges) = model_mesh(model, self.active.lanes(), uv_channel, key);
        self.active.mesh = if indices.is_empty() {
            None
        } else {
            let deform = match &self.active.deform_layout {
                Some(layout) => DeformGpu::new(device, layout)?,
                None => None,
            };
            Some(MeshBuffers {
                vertices: VertexBuffer::new(device, &vertices)?,
                indices: IndexBuffer::new(device, &indices)?,
                ranges,
                deform,
            })
        };
        self.active.mesh_revision = model_revision;
        self.active.mesh_uv_channel = uv_channel;
        self.active.mesh_material_mode = mode;
        Ok(())
    }

    /// Upload the frame's pose into the active mesh's palette when its revision
    /// moved. A pose that doesn't fit the model's tables (built for another
    /// model, mid-swap) is ignored, which also leaves the deform flag off.
    pub(super) fn sync_pose(
        &mut self,
        ctx: &ID3D11DeviceContext,
        pose: Option<&DeformPose>,
        pose_revision: u64,
    ) -> windows::core::Result<()> {
        let Some(deform) = self
            .active
            .mesh
            .as_mut()
            .and_then(|mesh| mesh.deform.as_mut())
        else {
            return Ok(());
        };
        match pose {
            Some(pose) if deform.accepts(pose) => deform.upload(ctx, pose, pose_revision),
            _ => {
                deform.palette_revision = None;
                Ok(())
            }
        }
    }

    /// Build-on-demand / free-on-off for the derived line views (invariant 3).
    /// A view's buffer is (re)built when its toggle is on and its baked params
    /// drift from the current options, and dropped to `None` when off. Unchanged
    /// views are left untouched, and the drift checks compare against the
    /// *borrowed* frame inputs, so a steady-state frame allocates nothing.
    /// `scene_bounds` is what the All Meshes bounding box draws when given (a
    /// clip's envelope), else the model's own bounds.
    pub(super) fn sync_line_views(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        hidden_meshes: &[u32],
        scene_bounds: Option<Bounds>,
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
                    &wireframe_lines(
                        model,
                        self.active.lanes(),
                        debug.wireframe_color,
                        hidden_meshes,
                    ),
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
        let scope_bounds = match scope {
            BoundingBoxScope::AllMeshes => scene_bounds,
            _ => None,
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
                    && params.bounds == scope_bounds
            }
            _ => false,
        };
        if !bounding_box_unchanged {
            self.active.views.bounding_box_buf = if debug.show_bounding_box {
                let bounds = match scope {
                    BoundingBoxScope::AllMeshes => scope_bounds.or(model.bounds),
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
                    bounds: scope_bounds,
                });
        }

        // The two normal-line views share one shape (length + color + hidden set),
        // differing only in the toggle and line builder.
        let lanes = self
            .active
            .deform_layout
            .as_ref()
            .map_or(&[][..], |layout| layout.corner.as_slice());
        sync_normal_view(
            device,
            model,
            lanes,
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
            lanes,
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
    pub(super) fn sync_skin_weights(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        selected_bones: &[u32],
    ) -> windows::core::Result<()> {
        let active = debug.active_material == ActiveMaterial::SkinWeights && model.skin.is_some();
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let unchanged = match (&self.active.views.weights_baked, active) {
            (None, false) => true,
            (Some(params), true) => {
                params.model_revision == model_revision && params.selected == selected_bones
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        self.active.views.weights_buf = if active {
            optional_vertex_buffer(
                device,
                &skin_weight_vertices(model, self.active.lanes(), selected_bones),
            )?
        } else {
            None
        };
        self.active.views.weights_baked = active.then(|| SkinWeightParams {
            model_revision,
            selected: selected_bones.to_vec(),
        });
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
    pub(super) fn sync_skeleton(
        &mut self,
        device: &ID3D11Device,
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        selected_bones: &[u32],
    ) -> windows::core::Result<()> {
        let on = debug.show_skeleton;
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let unchanged = match (&self.active.views.skeleton_baked, on) {
            (None, false) => true,
            (Some(params), true) => {
                params.model_revision == model_revision
                    && params.selected == selected_bones
                    && params.scale == debug.skeleton_joint_scale
                    && params.color == debug.skeleton_color
                    && params.selected_color == debug.skeleton_selected_color
            }
            _ => false,
        };
        if unchanged {
            return Ok(());
        }
        if on {
            self.active.views.skeleton_fill_buf = optional_vertex_buffer(
                device,
                &skeleton_fill_triangles(
                    model,
                    selected_bones,
                    debug.skeleton_joint_scale,
                    debug.skeleton_color,
                    debug.skeleton_selected_color,
                    SKELETON_FILL_ALPHA,
                ),
            )?;
            self.active.views.skeleton_line_buf = optional_vertex_buffer(
                device,
                &skeleton_lines(
                    model,
                    selected_bones,
                    debug.skeleton_joint_scale,
                    debug.skeleton_color,
                    debug.skeleton_selected_color,
                ),
            )?;
        } else {
            self.active.views.skeleton_fill_buf = None;
            self.active.views.skeleton_line_buf = None;
        }
        self.active.views.skeleton_baked = on.then(|| SkeletonParams {
            model_revision,
            selected: selected_bones.to_vec(),
            scale: debug.skeleton_joint_scale,
            color: debug.skeleton_color,
            selected_color: debug.skeleton_selected_color,
        });
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
    pub(super) fn sync_selection(
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
    pub(super) fn sync_visibility(
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

/// Build an optional vertex buffer from `vertices`: `None` for an empty set (D3D11
/// rejects a zero-byte buffer, and the draw is skipped anyway), else an immutable
/// [`VertexBuffer`]. The build-on-demand line views + the UV wireframe/fill use this
/// so an off / empty view holds no allocation.
pub(super) fn optional_vertex_buffer(
    device: &ID3D11Device,
    vertices: &[crate::scene::SceneVertex],
) -> windows::core::Result<Option<VertexBuffer>> {
    if vertices.is_empty() {
        Ok(None)
    } else {
        Ok(Some(VertexBuffer::new(device, vertices)?))
    }
}

/// A normal-line builder: `(model, deform lanes, length scale, color, hidden
/// nodes)` → line vertices. The face and vertex variants share the shape.
type NormalLineBuilder =
    fn(&ModelData, &[[u32; 4]], f32, [f32; 4], &[u32]) -> Vec<crate::scene::SceneVertex>;

/// Reconcile one normal-line view (face or vertex normals — the same shape,
/// differing only in the toggle and `lines` builder): rebuild when on + drifted
/// (compared against the borrowed inputs, so a steady-state frame allocates
/// nothing), free when off.
#[allow(clippy::too_many_arguments)] // Disjoint &mut field pairs + the view's plain inputs.
fn sync_normal_view(
    device: &ID3D11Device,
    model: &ModelData,
    lanes: &[[u32; 4]],
    buf: &mut Option<VertexBuffer>,
    baked: &mut Option<NormalParams>,
    on: bool,
    length: f32,
    color: [f32; 4],
    hidden: &[u32],
    lines: NormalLineBuilder,
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
        optional_vertex_buffer(device, &lines(model, lanes, length, color, hidden))?
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
