//! The Opt workspace's comparison render: the source and processed meshes side by
//! side, or one of them ghosted over the other.
//!
//! Everything here is layout and a second model — the shading, wireframe, normal, AA,
//! AO and tone-map settings are the ones [`SceneGpu::render`] already uses, and both
//! halves go through the same passes. The two meshes stay resident across the swap
//! between them in the active/idle [`ModelSlot`] pair, so alternating between them
//! within a frame is a `mem::swap` rather than two mesh uploads.
//!
//! The split is the one path that needs **two target sets**, at half width each. The
//! composite is a deferred job (`mac-port-plan.md` §3.2), so by the time either half's
//! composite runs, both halves' passes have already been recorded — a shared set would
//! have the second view's contents in it and both halves would show the same mesh. Two
//! half-width sets cost what one full-width set does, and the second is released the
//! moment the split is not on screen (invariant 3).
//!
//! [`ModelSlot`]: super::resources::ModelSlot

use crate::geometry::wireframe_edge_indices;
use crate::material::MaterialState;
use crate::rhi::{Frame, GpuResult, IndexBuffer};
use crate::{GhostStyle, OptSceneFrame, OptView, ProcessedModelRef, SceneFrame};

use super::gpu::SceneGpu;
use super::slot::SlotId;
use super::targets::{BackbufferRect, TargetSetId};

impl SceneGpu {
    /// Render the Opt workspace: the source and processed meshes side by side, or one
    /// over the other with the second drawn as a ghost.
    ///
    /// The split renders each half into its own target set, both sized to half the
    /// viewport, and composites each into its own half. What both need is for the two
    /// meshes to stay uploaded across the swap between them, which is what
    /// [`ModelSlot`] is for.
    ///
    /// [`ModelSlot`]: super::resources::ModelSlot
    pub(crate) fn render_opt(
        &mut self,
        frame: &mut Frame<'_>,
        opt: &OptSceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
    ) -> GpuResult<()> {
        match opt.view {
            // The overlay needs two meshes to have anything to overlay; without a
            // processed one it is simply the 3D scene.
            OptView::Overlay { ghost, swap, tint } => match opt.processed {
                Some(processed) => self.render_overlay(
                    frame,
                    opt,
                    processed,
                    material_states,
                    material_revision,
                    (ghost, swap, tint),
                ),
                None => {
                    self.activate(SlotId::Source);
                    self.release_ghost_wireframes();
                    self.release_split_targets();
                    self.render(
                        frame,
                        &opt.base,
                        material_states,
                        material_revision,
                        opt.source_camera,
                    )
                }
            },
            // The split always draws two views. With nothing processed yet both show
            // the source: the layout the user picked is the layout they get, and the
            // right-hand view fills in as soon as a run lands.
            OptView::Split => self.render_split(frame, opt, material_states, material_revision),
        }
    }

    /// The side-by-side comparison: two views of the same scene, laid out inside the
    /// chrome-free viewport so the divider falls where the user sees it fall.
    fn render_split(
        &mut self,
        frame: &mut Frame<'_>,
        opt: &OptSceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
    ) -> GpuResult<()> {
        // No ghost is drawn in the split view, so its buffers go (invariant 3).
        self.release_ghost_wireframes();

        // The two composites below cover only the chrome-free rect, and the swapchain
        // pass's clear is what defines the rest — the chrome is drawn over it, so
        // black costs nothing and leaves no strip showing an undefined backbuffer.
        frame.set_clear([0.0; 3]);

        let view = opt.viewport;
        let height = view.height.max(1);
        // Each half renders at half the viewport's width. An odd width gives the right
        // half the leftover column so no strip is left unpainted; the sub-pixel
        // stretch that implies is invisible, whereas an unpainted column showing the
        // previous frame is not.
        let half = (view.width / 2).max(1);
        let target = (half, height);
        // Each view is half as wide as the area it is drawn into, so its camera's
        // aspect has to say so — `app` set it from the whole window, which would
        // stretch both halves horizontally. The renderer owns the split geometry, so
        // it owns this correction.
        let aspect = half as f32 / height as f32;
        // Both views share one camera unless the user has unlinked them, and the
        // aspect correction is the same for both: the two halves are the same size, so
        // anything that differs between them is a difference in the mesh, which is the
        // entire point of the comparison.
        let mut source_camera = opt.source_camera;
        source_camera.aspect_ratio = aspect;
        let mut processed_camera = opt.processed_camera;
        processed_camera.aspect_ratio = aspect;

        self.activate(SlotId::Source);
        self.sync_frame(frame, &opt.base, material_states, material_revision, target)?;
        self.sync_ao_history(TargetSetId::Primary, source_camera, &opt.base);
        // The right half's set, sized to match — built here, after `sync_frame` has
        // settled the MSAA level both sets must share.
        self.sync_split_targets(target)?;
        // Deliberately not `?`: the right half must be attempted either way, so one
        // failed half leaves the other drawn rather than the whole frame blank.
        let left = self.record_view(
            frame,
            &opt.base,
            source_camera,
            self.targets(),
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
        let right = match opt.processed {
            Some(processed) => {
                let processed_frame = opt.base.with_model(processed.model, processed.revision);
                self.activate(SlotId::Processed);
                self.sync_frame(
                    frame,
                    &processed_frame,
                    material_states,
                    material_revision,
                    target,
                )
                .and_then(|()| {
                    self.sync_ao_history(TargetSetId::Split, processed_camera, &processed_frame);
                    self.record_view(
                        frame,
                        &processed_frame,
                        processed_camera,
                        self.split_targets(),
                        right_rect,
                    )
                })
            }
            // Nothing processed: the right half is the same scene again, drawn from the
            // slot already active — no swap, no second upload.
            None => {
                self.sync_ao_history(TargetSetId::Split, processed_camera, &opt.base);
                self.record_view(
                    frame,
                    &opt.base,
                    processed_camera,
                    self.split_targets(),
                    right_rect,
                )
            }
        };
        left.and(right)
    }

    /// The single-view comparison: one mesh shaded, the other over it as a ghost.
    ///
    /// Both meshes are drawn in the same pass so they occlude each other properly,
    /// which is the whole point — a silhouette that has moved shows up as ghost
    /// spilling past the solid surface.
    fn render_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        opt: &OptSceneFrame<'_>,
        processed: ProcessedModelRef<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        (ghost, swap, tint): (GhostStyle, bool, [f32; 3]),
    ) -> GpuResult<()> {
        let size = frame.size();
        let processed_frame = opt.base.with_model(processed.model, processed.revision);
        // `swap` decides which mesh reads as solid; the other becomes the ghost.
        let (solid_slot, solid_frame, ghost_slot, ghost_frame) = if swap {
            (
                SlotId::Source,
                &opt.base,
                SlotId::Processed,
                &processed_frame,
            )
        } else {
            (
                SlotId::Processed,
                &processed_frame,
                SlotId::Source,
                &opt.base,
            )
        };

        // Build the ghost's buffers first, then leave the solid slot active so the
        // scene pass draws it normally. The ghost is drawn from the idle slot, which is
        // only safe because a slot owns its own buffers outright.
        self.activate(ghost_slot);
        self.sync_frame(frame, ghost_frame, material_states, material_revision, size)?;
        if ghost == GhostStyle::Wireframe {
            self.sync_ghost_wireframe(ghost_frame)?;
        } else {
            self.release_ghost_wireframes();
        }

        self.activate(solid_slot);
        self.sync_frame(frame, solid_frame, material_states, material_revision, size)?;
        // One view, so one target set (invariant 3).
        self.release_split_targets();
        self.sync_ao_history(TargetSetId::Primary, opt.source_camera, solid_frame);
        self.record_view_with_ghost(
            frame,
            solid_frame,
            opt.source_camera,
            self.targets(),
            BackbufferRect::full(size),
            Some((ghost, tint)),
        )
    }

    /// Build the active slot's ghost wireframe, which exists regardless of the user's
    /// wireframe toggle — in the wireframe ghost style it *is* the ghost, not an
    /// overlay on it.
    fn sync_ghost_wireframe(&mut self, scene: &SceneFrame<'_>) -> GpuResult<()> {
        // Drift check against the borrowed inputs — no per-frame key allocation.
        // The colour is no longer part of the key: it rides in the `selection_color`
        // uniform at draw time, so a tint change rebuilds nothing.
        let unchanged = match &self.active.ghost_wireframe_baked {
            Some((revision, hidden)) => {
                *revision == scene.model_revision && hidden == scene.hidden_meshes
            }
            None => false,
        };
        if unchanged {
            return Ok(());
        }
        let edges = wireframe_edge_indices(scene.model, scene.hidden_meshes);
        self.active.ghost_wireframe_index = if edges.is_empty() {
            None
        } else {
            Some(IndexBuffer::new(&edges, c"ghost wireframe")?)
        };
        self.active.ghost_wireframe_baked =
            Some((scene.model_revision, scene.hidden_meshes.to_vec()));
        Ok(())
    }
}

/// The ghost's colour, as the `selection_color` uniform's gamma-space RGB plus alpha.
///
/// The hue comes from the caller — the chrome shows the same colour in the overlay's
/// legend, and a swatch that disagreed with the mesh would be worse than no legend.
/// The alpha is decided here because it is a rendering matter: the x-ray's is low
/// enough that the solid mesh stays legible through it but high enough that a
/// silhouette spilling past that surface is obvious — the spill is the whole signal
/// the overlay exists to show. The wireframe ghost is opaque: an alpha-faded line a
/// pixel wide would simply disappear.
pub(super) fn ghost_tint(style: GhostStyle, tint: [f32; 3]) -> [f32; 4] {
    let [r, g, b] = tint;
    match style {
        GhostStyle::Xray => [r, g, b, 0.28],
        GhostStyle::Wireframe => [r, g, b, 1.0],
    }
}
