//! The deform tables on the GPU: the influence / palette / morph buffers the
//! vertex shader reads at `t12..t15`.
//!
//! The palette and shape weights are re-uploaded only when `pose_revision`
//! moves, so a steady-state frame of a playing clip uploads two small buffers
//! rather than rebuilding anything.

use review_model::DeformPose;

use crate::geometry::deform::DeformLayout;
use crate::rhi::{Bindings, GpuResult, StorageBuffer};
use crate::shaders::generated;

use super::gpu::SceneGpu;
use super::gpu_types::{InfluenceEntry, MorphEntry, PaletteEntry};

/// The vertex shader's deform inputs for one model (`t12..t15`): the immutable
/// influence + blend-shape tables built with the mesh, and the dynamic palette +
/// shape weights re-uploaded when the pose revision moves.
pub(super) struct DeformGpu {
    pub(super) influences: StorageBuffer<InfluenceEntry>,
    /// `None` when the model has no blend shapes; the shader never reads it then
    /// (every morph lane is empty), but the slot must still be bound, so the scene
    /// binds its one-element dummy instead.
    pub(super) morph: Option<StorageBuffer<MorphEntry>>,
    pub(super) palette: StorageBuffer<PaletteEntry>,
    pub(super) shape_weights: Option<StorageBuffer<f32>>,
    /// The `pose_revision` the palette currently holds; `None` until a pose has
    /// been uploaded (the deform flag stays off until then).
    pub(super) palette_revision: Option<u64>,
    /// Scratch for the palette conversion, kept so a pose change allocates nothing.
    pub(super) palette_scratch: Vec<PaletteEntry>,
}

impl DeformGpu {
    pub(super) fn new(layout: &DeformLayout) -> GpuResult<Option<Self>> {
        if layout.influences.is_empty() || layout.palette_len == 0 {
            return Ok(None);
        }
        let morph = if layout.morph.is_empty() {
            None
        } else {
            Some(StorageBuffer::immutable(&layout.morph, c"morph deltas")?)
        };
        let shape_weights = if layout.shape_count == 0 {
            None
        } else {
            Some(StorageBuffer::dynamic(
                layout.shape_count,
                c"shape weights",
            )?)
        };
        Ok(Some(Self {
            influences: StorageBuffer::immutable(&layout.influences, c"deform influences")?,
            morph,
            palette: StorageBuffer::dynamic(layout.palette_len, c"deform palette")?,
            shape_weights,
            palette_revision: None,
            palette_scratch: Vec::with_capacity(layout.palette_len),
        }))
    }

    /// Whether `pose` fits these tables — a pose built for another model would
    /// index past the palette, so it is refused rather than clamped.
    pub(super) fn accepts(&self, pose: &DeformPose) -> bool {
        pose.palette.len() == self.palette.capacity()
            && pose.shape_weights.len() == self.shape_weights.as_ref().map_or(0, |b| b.capacity())
    }

    /// Upload `pose` (palette rows + shape weights) when `revision` moved.
    pub(super) fn upload(&mut self, pose: &DeformPose, revision: u64) -> GpuResult<()> {
        if self.palette_revision == Some(revision) {
            return Ok(());
        }
        self.palette_scratch.clear();
        self.palette_scratch.extend(
            pose.palette
                .iter()
                .map(|matrix| PaletteEntry::from_mat4(*matrix)),
        );
        self.palette.update(&self.palette_scratch)?;
        if let Some(weights) = &self.shape_weights {
            weights.update(&pose.shape_weights)?;
        }
        self.palette_revision = Some(revision);
        Ok(())
    }

    /// Bind this model's real tables over whatever `bindings` already holds. The
    /// two optional ones are left as the caller set them — every declared slot must
    /// be bound, so the scene fills all four with dummies first (`DeformDummies`).
    pub(super) fn bind(&self, bindings: &mut Bindings) {
        bindings.storage(generated::VIEW_DEFORM_INFLUENCES, &self.influences);
        bindings.storage(generated::VIEW_DEFORM_PALETTE, &self.palette);
        if let Some(morph) = &self.morph {
            bindings.storage(generated::VIEW_MORPH_DELTAS, morph);
        }
        if let Some(weights) = &self.shape_weights {
            bindings.storage(generated::VIEW_MORPH_WEIGHTS, weights);
        }
    }
}

impl SceneGpu {
    /// Upload the frame's pose into the active mesh's palette when its revision
    /// moved. A pose that doesn't fit the model's tables (built for another
    /// model, mid-swap) is ignored, which also leaves the deform flag off.
    pub(super) fn sync_pose(
        &mut self,
        pose: Option<&DeformPose>,
        pose_revision: u64,
    ) -> GpuResult<()> {
        let Some(deform) = self
            .active
            .mesh
            .as_mut()
            .and_then(|mesh| mesh.deform.as_mut())
        else {
            return Ok(());
        };
        match pose {
            Some(pose) if deform.accepts(pose) => deform.upload(pose, pose_revision),
            _ => {
                deform.palette_revision = None;
                Ok(())
            }
        }
    }
}
