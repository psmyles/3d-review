//! Everything the scene renderer caches *per model*: [`ModelSlot`] — the uploaded
//! mesh buffers, the build-on-demand derived views and the selection / visibility
//! draw lists, each with the bake key it was built for — and the `sync_*` builders
//! that reconcile them with the live frame.
//!
//! Invariant 3 lives here: a derived view's buffer exists only while its toggle is
//! on, is rebuilt when its baked parameters drift, and is dropped the moment the
//! toggle goes off, so the steady-state shaded view holds no derived buffers. The
//! passes that *draw* these buffers are in [`super::gpu`].//!
//! What is left here is the mesh upload itself and the UV viewport's views; the
//! rest is beside it: [`super::slot`] defines the per-model state,
//! [`super::deform_gpu`] the deform tables, [`super::line_views`] the derived
//! overlays and [`super::draw_lists`] the index ordering.

use review_model::ModelData;

use crate::geometry::deform::DeformLayout;
use crate::geometry::{model_mesh, uv_fill_triangles, uv_wireframe_lines};
use crate::material::build_part_key;
use crate::rhi::{GpuResult, IndexBuffer, VertexBuffer};
use crate::{MaterialMode, UvShadingMode};

use super::deform_gpu::DeformGpu;
use super::gpu::SceneGpu;
use super::line_views::optional_vertex_buffer;
use super::slot::MeshBuffers;

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
        model: &ModelData,
        model_revision: u64,
        channel: u32,
        shading_mode: UvShadingMode,
    ) -> GpuResult<()> {
        let want_wireframe = Some((model_revision, channel));
        if self.active.views.uv_wireframe_baked != want_wireframe {
            self.active.views.uv_wireframe_buf =
                optional_vertex_buffer(&uv_wireframe_lines(model, channel))?;
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
            self.active.views.uv_fill_buf = optional_vertex_buffer(&fill)?;
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
        model: &ModelData,
        model_revision: u64,
        uv_channel: u32,
        mode: MaterialMode,
    ) -> GpuResult<()> {
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
                Some(layout) => DeformGpu::new(layout)?,
                None => None,
            };
            Some(MeshBuffers {
                vertices: VertexBuffer::new(&vertices, c"mesh")?,
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
