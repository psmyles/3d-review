//! Everything the scene renderer caches *per model*:
//! [`ModelSlot`](super::slot::ModelSlot) — the uploaded mesh buffers, the
//! build-on-demand derived views and the selection / visibility draw lists, each with
//! the bake key it was built for — and the `sync_*` builders that reconcile them with
//! the live frame.
//!
//! Invariant 3 lives here: a derived view's buffer exists only while its toggle is
//! on, is rebuilt when its baked parameters drift, and is dropped the moment the
//! toggle goes off, so the steady-state shaded view holds no derived buffers. The
//! passes that *draw* these buffers are in [`super::gpu`].
//!
//! What is left here is the mesh upload itself and the UV viewport's views; the
//! rest is beside it: [`super::slot`] defines the per-model state,
//! [`super::deform_gpu`] the deform tables, [`super::line_views`] the derived
//! overlays and [`super::draw_lists`] the index ordering.

use review_model::ModelData;

use crate::geometry::deform::DeformLayout;
use crate::geometry::{UvNodeScope, model_mesh, uv_fill_triangles, uv_wireframe_lines};
use crate::material::build_part_key;
use crate::rhi::{GpuResult, IndexBuffer, VertexBuffer};
use crate::{MaterialMode, UvShadingMode};

use super::deform_gpu::DeformGpu;
use super::gpu::SceneGpu;
use super::line_views::{optional_line_buffer, optional_vertex_buffer};
use super::slot::{MeshBuffers, UvViewParams};

/// How opaque the UV island fill is over a texture: enough to read the islands
/// by, little enough to see the image they cover.
const UV_FILL_OVER_TEXTURE_OPACITY: f32 = 0.35;

impl SceneGpu {
    /// Free both slots' UV viewport buffers (invariant 3) — this frame is a 3D one,
    /// so nothing draws them.
    ///
    /// The pair is the counterpart to [`Self::sync_uv_view`]'s build arm, and the
    /// only derived view whose builder is reached from a *different* entry point
    /// than the 3D `sync_frame`: without a free arm here one visit to the UV tab
    /// left a full-model edge list (two 64-byte vertices per UV edge — tens of
    /// megabytes on a real asset) plus its island fill resident for the rest of the
    /// session, since the bake keys alone are only invalidated by a model swap.
    pub(crate) fn release_uv_views(&mut self) {
        for slot in [&mut self.active, &mut self.idle] {
            slot.views.uv_wireframe_buf = None;
            slot.views.uv_wireframe_baked = None;
            slot.views.uv_fill_buf = None;
            slot.views.uv_fill_baked = None;
        }
        self.uv_texture = None;
        self.uv_checker = None;
    }

    /// Build-on-demand for the UV viewport's derived buffers (invariant 3): the
    /// wireframe is rebuilt only when the model / channel / laid-out nodes change;
    /// the island fill when those or the shading mode change, and freed in Wire
    /// mode — so panning/zooming rebuilds nothing. Both compare against the
    /// borrowed inputs, so a steady frame allocates nothing. Leaving the viewport
    /// frees both, in [`Self::release_uv_views`].
    ///
    /// `selected` is the Outliner's node selection (sorted; empty for none) and
    /// `hidden` its hidden meshes: the view lays out the selection when there is
    /// one, else every node, and never a hidden one ([`UvNodeScope`]). `dimmed`
    /// lowers the fill's opacity, for a texture drawn behind it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sync_uv_view(
        &mut self,
        model: &ModelData,
        model_revision: u64,
        channel: u32,
        shading_mode: UvShadingMode,
        selected: &[u32],
        hidden: &[u32],
        dimmed: bool,
    ) -> GpuResult<()> {
        let views = &mut self.active.views;
        let wireframe_current = views
            .uv_wireframe_baked
            .as_ref()
            .is_some_and(|params| params.matches(model_revision, channel, selected, hidden));
        let fill_current = match (shading_mode, &views.uv_fill_baked) {
            (UvShadingMode::Wire, baked) => baked.is_none(),
            (mode, Some((params, baked_mode, baked_dimmed))) => {
                *baked_mode == mode
                    && *baked_dimmed == dimmed
                    && params.matches(model_revision, channel, selected, hidden)
            }
            (_, None) => false,
        };
        if wireframe_current && fill_current {
            return Ok(());
        }

        let scope = UvNodeScope::new(model, selected, hidden);
        let params = || UvViewParams {
            model_revision,
            channel,
            selected: selected.to_vec(),
            hidden: hidden.to_vec(),
        };
        if !wireframe_current {
            views.uv_wireframe_buf =
                optional_line_buffer(&uv_wireframe_lines(model, channel, &scope))?;
            views.uv_wireframe_baked = Some(params());
        }
        if !fill_current {
            let opacity = if dimmed {
                UV_FILL_OVER_TEXTURE_OPACITY
            } else {
                1.0
            };
            let fill = match shading_mode {
                UvShadingMode::Wire => Vec::new(),
                UvShadingMode::Shaded => uv_fill_triangles(model, channel, false, opacity, &scope),
                UvShadingMode::Islands => uv_fill_triangles(model, channel, true, opacity, &scope),
            };
            views.uv_fill_buf = optional_vertex_buffer(&fill)?;
            views.uv_fill_baked = match shading_mode {
                UvShadingMode::Wire => None,
                mode => Some((params(), mode, dimmed)),
            };
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
        model: &ModelData,
        model_revision: u64,
        uv_channel: u32,
        mode: MaterialMode,
    ) -> GpuResult<()> {
        if self.active.mesh_revision == model_revision
            && self.active.mesh_uv_channel == uv_channel
            && self.active.mesh_material_mode.groups_by_part() == mode.groups_by_part()
        {
            return Ok(());
        }
        let key = match mode {
            MaterialMode::Unique if !self.active.unique_part_key.is_empty() => {
                Some(self.active.unique_part_key.as_slice())
            }
            _ => None,
        };
        // The deform layout and its GPU tables are a property of the model alone:
        // built and uploaded on a model change, and carried across a UV-channel or
        // material-mode rebuild, which only reorders the mesh's own buffers. Once
        // uploaded the influence and morph tables are dropped from the CPU side
        // (invariant 1 - the morph table is the model's blend shapes over again);
        // only the per-corner lanes stay, since every mesh-derived overlay copies
        // them.
        let model_changed = self.active.mesh_revision != model_revision;
        if model_changed {
            self.active.deform_layout = DeformLayout::build(model);
        }
        let (vertices, indices, ranges) = model_mesh(model, self.active.lanes(), uv_channel, key);
        let kept_deform = self.active.mesh.take().and_then(|mesh| mesh.deform);
        self.active.mesh = if indices.is_empty() {
            None
        } else {
            let deform = match (&mut self.active.deform_layout, model_changed) {
                (Some(layout), true) => {
                    let deform = DeformGpu::new(layout)?;
                    layout.release_tables();
                    deform
                }
                (Some(_), false) => kept_deform,
                (None, _) => None,
            };
            Some(MeshBuffers {
                // Pullable, so the wireframe reads these bytes rather than a copy.
                vertices: VertexBuffer::pullable(&vertices, c"mesh")?,
                indices: IndexBuffer::new(&indices, c"mesh")?,
                ranges,
                deform,
            })
        };
        self.active.mesh_revision = model_revision;
        self.active.mesh_uv_channel = uv_channel;
        self.active.mesh_material_mode = mode;
        Ok(())
    }
}
