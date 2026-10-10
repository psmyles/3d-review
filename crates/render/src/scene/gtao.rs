//! The ambient-occlusion passes.
//!
//! Horizon-based occlusion over a *separate single-sample* view-normal/view-Z
//! G-buffer — its own mesh-only pass, not an MSAA attachment, because MSAA edge
//! averaging would corrupt the normals it reads. The composite applies the
//! result as a scalar darkening of the diffuse ambient only, so direct and
//! emissive light are never darkened.
//!
//! The frame's shape, in order:
//!
//! 1. **G-buffer** — the mesh again, writing the view normal + Z and (as a second
//!    attachment) the positive linear depth the chain below reduces.
//! 2. **Depth prefilter** — four passes, each halving the level above. A tap far
//!    from its pixel then reads a level that stands for the whole neighbourhood it
//!    represents, instead of one aliased texel of whatever thin thing was there.
//! 3. **Occlusion** — the horizon search, which also folds its result into the
//!    running mean (see [`super::ao_accum`]) so nothing needs a separate blend pass.
//! 4. **Denoise** — one to three edge-aware passes, ping-ponged.
//!
//! Every one of them is skipped on a frame whose result has converged: the image is
//! still in the target the last run left it in and nothing else writes there, so an
//! idle viewer showing ambient occlusion costs *less* than it did before this, not
//! more.

use crate::rhi::{Bindings, ColorTarget, Frame, Zone};
use crate::shaders::generated;
use crate::{CameraProjection, GtaoSettings, OrbitCamera, SceneFrame};

use super::gpu_types::SceneUniforms;

use super::ao_accum::{AoKey, AoPlan};
use super::gpu::FULLSCREEN_VERTICES;
use super::gpu::SceneGpu;
use super::targets::{DEPTH_MIP_LEVELS, TargetSet, TargetSetId};
use super::uniforms::{
    build_gtao_mip_uniforms, build_gtao_uniforms, flat_display, gtao_view_radius,
};

impl SceneGpu {
    /// Whether GTAO should run for this frame: enabled, a mesh is present, and the
    /// view isn't one of the flat data-inspection ones.
    pub(super) fn gtao_active(&self, scene: &SceneFrame<'_>) -> bool {
        scene.gtao.enabled && self.active.has_mesh() && !flat_display(scene)
    }

    /// Decide what this frame's occlusion should do — average another sample into the
    /// running mean, start it over, or nothing at all — and record it on the target
    /// set for [`Self::record_gtao`] to carry out.
    ///
    /// Split from the recording because this is the one part that *mutates*: it lives
    /// in the `sync_*` family with the rest of the per-frame reconciliation, which
    /// keeps every `record_*` taking `&self`.
    pub(super) fn sync_ao_history(
        &mut self,
        set: TargetSetId,
        camera: OrbitCamera,
        scene: &SceneFrame<'_>,
    ) {
        let active = self.gtao_active(scene);
        let visibility_generation = self.active.visibility_generation;
        let slot = self.active_slot;
        let targets = self.targets_mut(set);
        let key = active.then(|| {
            AoKey::new(
                camera,
                scene,
                visibility_generation,
                slot,
                targets.gtao_gbuffer.size(),
            )
        });
        targets.ao_plan = targets.ao.advance(key);
    }

    /// The GTAO passes, in the order described at the top of this module. Hands back
    /// the target the composite should read — which is the last denoise output
    /// whether or not anything ran this frame.
    ///
    /// Nothing is unbound between passes: a sokol pass ends before the next begins,
    /// so the target a pass wrote is free to be sampled by the next one — which is
    /// what the old `unbind_ps_srvs` calls were guarding by hand.
    pub(super) fn record_gtao<'t>(
        &self,
        frame: &mut Frame<'_>,
        camera: OrbitCamera,
        projection: CameraProjection,
        gtao: GtaoSettings,
        targets: &'t TargetSet,
        scene_uniforms: &SceneUniforms,
    ) -> &'t ColorTarget {
        let plan = targets.ao_plan;
        let output = targets.ao_output(gtao.quality);
        if !plan.run {
            // Converged: the result cannot improve, so the cheapest correct thing is
            // to leave every target exactly as it is.
            return output;
        }

        let uniforms = build_gtao_uniforms(camera, projection, gtao, targets.gtao_raw_size(), plan);

        self.record_gtao_gbuffer(frame, targets, scene_uniforms);
        self.record_depth_mips(frame, targets, gtao_view_radius(camera, gtao));
        self.record_occlusion(frame, targets, &uniforms, plan);
        self.record_denoise(frame, targets, &uniforms, gtao);
        output
    }

    /// The mesh again, into the single-sample normal/Z target and the full-resolution
    /// level of the depth chain beside it, clearing both and its own depth.
    fn record_gtao_gbuffer(
        &self,
        frame: &mut Frame<'_>,
        targets: &TargetSet,
        scene_uniforms: &SceneUniforms,
    ) {
        frame.zone_begin(Zone::GtaoGbuffer);
        frame.begin_offscreen_pass(
            &[&targets.gtao_gbuffer, &targets.gtao_depth_mips[0]],
            Some(&targets.scratch_depth),
            [0.0; 4],
            c"gtao gbuffer",
        );
        if let Some(mesh) = &self.active.mesh {
            // Match the shaded mesh's visibility so a hidden mesh casts no AO; solo is
            // deliberately left out, so only the per-mesh hide filters the occlusion.
            // `None` while the filter is active means every mesh is hidden.
            let index = if self.active.visible_active {
                self.active.visible_index.as_ref()
            } else {
                Some(&mesh.indices)
            };
            if let Some(index) = index {
                let mut bindings = self.line_bindings();
                bindings.mesh_vertices(&mesh.vertices);
                bindings.mesh_indices(index);
                frame.apply_pipeline(&self.gtao_gbuffer_pipeline);
                frame.apply_bindings(&bindings);
                frame.apply_uniforms(generated::UB_SCENE_VS, scene_uniforms);
                frame.apply_uniforms(generated::UB_SCENE_FS, scene_uniforms);
                frame.draw(0, index.count());
            }
        }
        frame.end_pass();
        frame.zone_end(Zone::GtaoGbuffer);
    }

    /// Reduce the depth chain, one level per pass. Level 0 already holds the
    /// G-buffer pass's output, so this starts at 1.
    fn record_depth_mips(&self, frame: &mut Frame<'_>, targets: &TargetSet, radius: f32) {
        frame.zone_begin(Zone::GtaoDepthMips);
        for level in 1..DEPTH_MIP_LEVELS {
            let source = &targets.gtao_depth_mips[level - 1];
            let uniforms = build_gtao_mip_uniforms(source.size(), radius);
            frame.begin_offscreen_pass(
                &[&targets.gtao_depth_mips[level]],
                None,
                [0.0; 4],
                c"gtao depth mip",
            );
            let mut bindings = Bindings::new();
            bindings.target(generated::VIEW_DEPTH_SRC, source);
            bindings.sampler(generated::SMP_GTAO_SAMPLER, &self.gtao_sampler);
            frame.apply_pipeline(&self.gtao_depth_mip_pipeline);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_GTAO_MIP_PARAMS, &uniforms);
            frame.draw(0, FULLSCREEN_VERTICES);
            frame.end_pass();
        }
        frame.zone_end(Zone::GtaoDepthMips);
    }

    /// The horizon search over the G-buffer + depth chain, blended into the running
    /// mean as it is written.
    fn record_occlusion(
        &self,
        frame: &mut Frame<'_>,
        targets: &TargetSet,
        uniforms: &super::gpu_types::GtaoUniforms,
        plan: AoPlan,
    ) {
        frame.zone_begin(Zone::Gtao);
        frame.begin_offscreen_pass(&[&targets.ao_history[plan.write]], None, [0.0; 4], c"gtao");
        let mut bindings = Bindings::new();
        bindings.target(generated::VIEW_GBUFFER, &targets.gtao_gbuffer);
        bindings.target(generated::VIEW_DEPTH_MIP0, &targets.gtao_depth_mips[0]);
        bindings.target(generated::VIEW_DEPTH_MIP1, &targets.gtao_depth_mips[1]);
        bindings.target(generated::VIEW_DEPTH_MIP2, &targets.gtao_depth_mips[2]);
        bindings.target(generated::VIEW_DEPTH_MIP3, &targets.gtao_depth_mips[3]);
        bindings.target(generated::VIEW_DEPTH_MIP4, &targets.gtao_depth_mips[4]);
        // Bound even on a reset frame, when the shader will not read it: sokol
        // requires every view the program declares to be filled.
        bindings.target(generated::VIEW_AO_HISTORY, &targets.ao_history[plan.read]);
        bindings.sampler(generated::SMP_GTAO_SAMPLER, &self.gtao_sampler);
        frame.apply_pipeline(&self.gtao_pipeline);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_GTAO_PARAMS, uniforms);
        frame.draw(0, FULLSCREEN_VERTICES);
        frame.end_pass();
        frame.zone_end(Zone::Gtao);
    }

    /// The edge-aware denoise, one to three passes by quality, ping-ponging between
    /// the two denoise targets. The first reads the accumulated occlusion; each later
    /// one reads what the one before it wrote.
    fn record_denoise(
        &self,
        frame: &mut Frame<'_>,
        targets: &TargetSet,
        uniforms: &super::gpu_types::GtaoUniforms,
        gtao: GtaoSettings,
    ) {
        let passes = gtao.quality.denoise_passes().max(1) as usize;
        let plan = targets.ao_plan;
        frame.zone_begin(Zone::GtaoDenoise);
        for pass in 0..passes {
            let source = if pass == 0 {
                &targets.ao_history[plan.write]
            } else {
                &targets.ao_denoise[(pass - 1) % 2]
            };
            frame.begin_offscreen_pass(
                &[&targets.ao_denoise[pass % 2]],
                None,
                [0.0; 4],
                c"gtao denoise",
            );
            let mut bindings = Bindings::new();
            bindings.target(generated::VIEW_GBUFFER, &targets.gtao_gbuffer);
            bindings.target(generated::VIEW_AO_IN, source);
            bindings.sampler(generated::SMP_GTAO_SAMPLER, &self.gtao_sampler);
            frame.apply_pipeline(&self.gtao_denoise_pipeline);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_GTAO_PARAMS, uniforms);
            frame.draw(0, FULLSCREEN_VERTICES);
            frame.end_pass();
        }
        frame.zone_end(Zone::GtaoDenoise);
    }

    /// Whether any view's occlusion would still improve with another frame. The app
    /// turns this into one more paced redraw (invariant 6); it goes false on its own
    /// once the average has converged.
    pub(crate) fn ao_converging(&self) -> bool {
        self.targets.ao.converging()
            || self
                .split_targets
                .as_ref()
                .is_some_and(|split| split.ao.converging())
    }
}
