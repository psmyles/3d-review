//! The derived line overlays: built when their view turns on, freed when it
//! turns off (invariant 3).
//!
//! Each `sync_*` compares against the **borrowed** frame inputs and only
//! `to_vec()`s its bake key on the rebuild path, so a steady-state frame
//! allocates nothing. The model wireframe is the one documented exception: it is
//! an *index* buffer over the mesh's own vertices, kept across toggles.

use review_model::{Bounds, ModelData};

use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_pivot, pivot_half_extent, pivot_lines,
    skeleton_fill_triangles, skeleton_lines, skin_weight_vertices, uv_seam_lines,
    vertex_normal_lines, wireframe_edge_indices,
};
use crate::rhi::{GpuResult, IndexBuffer, VertexBuffer};
use crate::selection::{Selection, selection_bounds};
use crate::{ActiveMaterial, BoundingBoxScope, SceneDebugOptions, ShadingMode};

use super::gpu::SceneGpu;
use super::slot::{
    BoundingBoxParams, DerivedViews, NormalParams, PivotParams, SKELETON_FILL_ALPHA,
    SkeletonParams, SkinWeightParams, UvSeamParams,
};

/// Build an optional vertex buffer from `vertices`: `None` for an empty set (D3D11
/// rejects a zero-byte buffer, and the draw is skipped anyway), else an immutable
/// [`VertexBuffer`]. The build-on-demand line views + the UV wireframe/fill use this
/// so an off / empty view holds no allocation.
pub(super) fn optional_vertex_buffer(
    vertices: &[crate::scene::SceneVertex],
) -> GpuResult<Option<VertexBuffer>> {
    if vertices.is_empty() {
        Ok(None)
    } else {
        Ok(Some(VertexBuffer::new(vertices, c"derived view")?))
    }
}

/// A normal-line builder: `(model, deform lanes, length scale, color, hidden
/// nodes)` → line vertices. The face and vertex variants share the shape.
pub(super) type NormalLineBuilder =
    fn(&ModelData, &[[u32; 4]], f32, [f32; 4], &[u32]) -> Vec<crate::scene::SceneVertex>;

/// Reconcile one normal-line view (face or vertex normals — the same shape,
/// differing only in the toggle and `lines` builder): rebuild when on + drifted
/// (compared against the borrowed inputs, so a steady-state frame allocates
/// nothing), free when off.
#[allow(clippy::too_many_arguments)] // Disjoint &mut field pairs + the view's plain inputs.
pub(super) fn sync_normal_view(
    model: &ModelData,
    lanes: &[[u32; 4]],
    buf: &mut Option<VertexBuffer>,
    baked: &mut Option<NormalParams>,
    on: bool,
    length: f32,
    color: [f32; 4],
    hidden: &[u32],
    lines: NormalLineBuilder,
) -> GpuResult<()> {
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
        optional_vertex_buffer(&lines(model, lanes, length, color, hidden))?
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

impl SceneGpu {
    /// Free both slots' ghost wireframes (invariant 3) — the overlay view is no
    /// longer showing one.
    pub(super) fn release_ghost_wireframes(&mut self) {
        for slot in [&mut self.active, &mut self.idle] {
            slot.ghost_wireframe_index = None;
            slot.ghost_wireframe_baked = None;
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
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        hidden_meshes: &[u32],
        scene_bounds: Option<Bounds>,
    ) -> GpuResult<()> {
        // A different mesh invalidates every view derived from it, whatever the
        // view's own parameters are doing (see `ModelSlot::views_revision`).
        // Dropping them here is what lets each bake key below stay purely about
        // its own settings; the ones still switched on rebuild in this same call.
        if self.active.views_revision != model_revision {
            self.active.views = DerivedViews::default();
            self.active.views_revision = model_revision;
        }

        // Wireframe: built on first use and then *kept* across toggles — the one
        // documented exception to invariant 3 (see `DerivedViews::wireframe_index`).
        // On in both the wireframe overlay and the wireframe-only shading mode.
        // Its colour is a uniform now, so only the Outliner's hidden set can drift
        // it (edges of a hidden mesh disappear with the mesh); a colour drag
        // rebuilds nothing.
        let wireframe_on =
            debug.wireframe_overlay || matches!(debug.shading_mode, ShadingMode::Wireframe);
        let wireframe_stale = self.active.views.wireframe_baked.as_deref() != Some(hidden_meshes);
        if wireframe_on && wireframe_stale {
            let edges = wireframe_edge_indices(model, hidden_meshes);
            self.active.views.wireframe_index = if edges.is_empty() {
                None
            } else {
                Some(IndexBuffer::new(&edges, c"wireframe")?)
            };
            self.active.views.wireframe_baked = Some(hidden_meshes.to_vec());
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
                    Some(bounds) => optional_vertex_buffer(&bounding_box_lines(
                        bounds,
                        debug.bounding_box_color,
                    ))?,
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

        // UV seams: the edges where the chosen UV set is cut. Its own shape rather
        // than a third `sync_normal_view` — there is no line length, and the UV
        // channel is part of the key because changing it changes which edges are
        // seams at all, not merely how they look.
        let uv_seam_unchanged = match (&self.active.views.uv_seam_baked, debug.uv_seams) {
            (None, false) => true,
            (Some(params), true) => {
                params.color == debug.uv_seam_color
                    && params.channel == debug.uv_seam_channel
                    && params.hidden == hidden_meshes
            }
            _ => false,
        };
        if !uv_seam_unchanged {
            self.active.views.uv_seam_buf = if debug.uv_seams {
                optional_vertex_buffer(&uv_seam_lines(
                    model,
                    lanes,
                    debug.uv_seam_color,
                    debug.uv_seam_channel,
                    hidden_meshes,
                ))?
            } else {
                None
            };
            self.active.views.uv_seam_baked = debug.uv_seams.then(|| UvSeamParams {
                color: debug.uv_seam_color,
                channel: debug.uv_seam_channel,
                hidden: hidden_meshes.to_vec(),
            });
        }

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
                    optional_vertex_buffer(&pivot_lines(*pivot, *half))?
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
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        selected_bones: &[u32],
    ) -> GpuResult<()> {
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
            optional_vertex_buffer(&skin_weight_vertices(
                model,
                self.active.lanes(),
                selected_bones,
            ))?
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
        model: &ModelData,
        model_revision: u64,
        debug: SceneDebugOptions,
        selected_bones: &[u32],
    ) -> GpuResult<()> {
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
            self.active.views.skeleton_fill_buf =
                optional_vertex_buffer(&skeleton_fill_triangles(
                    model,
                    selected_bones,
                    debug.skeleton_joint_scale,
                    debug.skeleton_color,
                    debug.skeleton_selected_color,
                    SKELETON_FILL_ALPHA,
                ))?;
            self.active.views.skeleton_line_buf = optional_vertex_buffer(&skeleton_lines(
                model,
                selected_bones,
                debug.skeleton_joint_scale,
                debug.skeleton_color,
                debug.skeleton_selected_color,
            ))?;
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
}
