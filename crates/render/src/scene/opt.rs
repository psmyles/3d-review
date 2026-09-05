//! The Opt workspace's comparison render: the source and processed meshes side
//! by side, or one of them ghosted over the other.
//!
//! Everything here is layout and a second model — the shading, wireframe, normal,
//! AA, AO and tone-map settings are the ones [`SceneGpu::render`] already uses, and
//! both halves go through the same passes and the same offscreen targets. The two
//! meshes stay resident across the swap between them in the active/idle
//! [`ModelSlot`] pair, so alternating between them within a frame is a `mem::swap`
//! rather than two mesh uploads.
//!
//! [`ModelSlot`]: super::resources::ModelSlot

use crate::geometry::wireframe_lines;
use crate::material::MaterialState;
use crate::rhi::{Gpu, GpuResult};
use crate::{GhostStyle, OptSceneFrame, OptView, OrbitCamera, ProcessedModelRef, SceneFrame};

use super::d3d::{BackbufferRect, SceneGpu, scene_uniforms};
use super::resources::{SlotId, optional_vertex_buffer};

impl SceneGpu {
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
    ) -> GpuResult<()> {
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
    ) -> GpuResult<()> {
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
    ) -> GpuResult<()> {
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
            self.sync_ghost_wireframe(gpu, ghost_frame, tint)?;
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

    /// Draw the idle slot's mesh as a see-through ghost over the solid one,
    /// inside the scene pass the caller has already opened.
    ///
    /// Both styles reuse pipelines that already exist. The x-ray is the
    /// selection-flash fill — a flat tinted colour, alpha-blended, depth-tested
    /// but not depth-writing, which is exactly ghost behaviour — with the tint fed
    /// through the same `selection_color` uniform it always reads. The wireframe
    /// ghost is the derived wireframe view drawn with the line pipeline. Neither
    /// needs a shader change, so the committed DXBC stays valid.
    pub(super) fn record_ghost(
        &mut self,
        gpu: &Gpu,
        camera: OrbitCamera,
        frame: &SceneFrame<'_>,
        style: GhostStyle,
        tint: [f32; 3],
    ) -> GpuResult<()> {
        // The ghost's own uniform: same camera and projection, but the flat fill
        // colour swapped in. Restored to the frame's own uniform afterwards so the
        // GTAO pass (which reads `view` from `b0`) still sees the right one.
        // The ghost is the idle slot's mesh, drawn in its bind pose: the Opt
        // workspace compares static geometry, so neither mesh deforms there.
        let mut uniforms = scene_uniforms(
            camera,
            frame.projection,
            frame.environment,
            frame.selection,
            frame.debug,
            false,
        );
        uniforms.selection_color = ghost_tint(style, tint);
        self.uniforms.update(gpu, &uniforms)?;

        match style {
            GhostStyle::Xray => {
                if let Some(mesh) = &self.idle.mesh {
                    self.scene.selection.bind(gpu);
                    mesh.vertices.bind(gpu);
                    mesh.indices.bind(gpu);
                    gpu.draw_indexed_range(mesh.indices.count(), 0);
                }
            }
            GhostStyle::Wireframe => {
                if let Some(lines) = &self.idle.ghost_wireframe_buf {
                    self.scene.line.bind(gpu);
                    lines.bind(gpu);
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
            self.active.deform_enabled(),
        );
        self.uniforms.update(gpu, &restored)?;
        Ok(())
    }

    /// Build the active slot's ghost wireframe, which exists regardless of the
    /// user's wireframe toggle — in the wireframe ghost style it *is* the ghost,
    /// not an overlay on it.
    fn sync_ghost_wireframe(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        tint: [f32; 3],
    ) -> GpuResult<()> {
        let colour = ghost_tint(GhostStyle::Wireframe, tint);
        // Drift check against the borrowed inputs — no per-frame key allocation.
        let unchanged = match &self.active.ghost_wireframe_baked {
            Some((revision, hidden, baked_colour)) => {
                *revision == frame.model_revision
                    && hidden == frame.hidden_meshes
                    && *baked_colour == colour
            }
            None => false,
        };
        if unchanged {
            return Ok(());
        }
        let lines = wireframe_lines(frame.model, &[], colour, frame.hidden_meshes);
        self.active.ghost_wireframe_buf = optional_vertex_buffer(gpu, &lines)?;
        self.active.ghost_wireframe_baked =
            Some((frame.model_revision, frame.hidden_meshes.to_vec(), colour));
        Ok(())
    }
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
