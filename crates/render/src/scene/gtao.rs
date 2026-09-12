//! The ambient-occlusion passes.
//!
//! Horizon-based occlusion over a *separate single-sample* view-normal/view-Z
//! G-buffer — its own mesh-only pass, not an MSAA attachment, because MSAA edge
//! averaging would corrupt the normals it reads. The composite applies the
//! result as a scalar darkening of the diffuse ambient only, so direct and
//! emissive light are never darkened.

use crate::rhi::{Bindings, Frame, Zone};
use crate::shaders::generated;
use crate::{CameraProjection, GtaoSettings, OrbitCamera, SceneFrame};

use super::gpu_types::SceneUniforms;

use super::gpu::FULLSCREEN_VERTICES;
use super::gpu::SceneGpu;
use super::targets::TargetSet;
use super::uniforms::{build_gtao_uniforms, flat_display};

impl SceneGpu {
    /// Whether GTAO should run for this frame: enabled, a mesh is present, and the
    /// view isn't one of the flat data-inspection ones.
    pub(super) fn gtao_active(&self, scene: &SceneFrame<'_>) -> bool {
        scene.gtao.enabled && self.active.has_mesh() && !flat_display(scene)
    }

    /// The GTAO passes: a single-sample mesh-only G-buffer (view normal + Z), then the
    /// horizon occlusion pass and the bilateral blur the composite darkens the ambient
    /// radiance by.
    ///
    /// The G-buffer shares the scene uniforms (they carry `view`); the two fullscreen
    /// passes read the GTAO block and the point sampler. Nothing is unbound between
    /// them: a sokol pass ends before the next begins, so the target a pass wrote is
    /// free to be sampled by the next one — which is what the old
    /// `unbind_ps_srvs` calls were guarding by hand.
    pub(super) fn record_gtao(
        &self,
        frame: &mut Frame<'_>,
        camera: OrbitCamera,
        projection: CameraProjection,
        gtao: GtaoSettings,
        targets: &TargetSet,
        scene_uniforms: &SceneUniforms,
    ) {
        let uniforms = build_gtao_uniforms(camera, projection, gtao, targets.gtao_raw.size());

        // G-buffer: redraw the mesh (its material is irrelevant) into the
        // single-sample normal/Z target, clearing it and its own depth.
        frame.zone_begin(Zone::GtaoGbuffer);
        frame.begin_offscreen_pass(
            &[&targets.gtao_gbuffer],
            Some(&targets.gtao_depth),
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

        // Occlusion: a fullscreen pass over the G-buffer → raw AO.
        frame.zone_begin(Zone::Gtao);
        frame.begin_offscreen_pass(&[&targets.gtao_raw], None, [0.0; 4], c"gtao");
        let mut bindings = Bindings::new();
        bindings.target(generated::VIEW_GBUFFER, &targets.gtao_gbuffer);
        bindings.sampler(generated::SMP_GTAO_SAMPLER, &self.gtao_sampler);
        frame.apply_pipeline(&self.gtao_pipeline);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_GTAO_PARAMS, &uniforms);
        frame.draw(0, FULLSCREEN_VERTICES);
        frame.end_pass();
        frame.zone_end(Zone::Gtao);

        // Bilateral blur: the G-buffer again (for the edge stopping) plus the raw AO.
        frame.zone_begin(Zone::GtaoBlur);
        frame.begin_offscreen_pass(&[&targets.gtao_blur], None, [0.0; 4], c"gtao blur");
        let mut bindings = Bindings::new();
        bindings.target(generated::VIEW_GBUFFER, &targets.gtao_gbuffer);
        bindings.target(generated::VIEW_RAW_AO, &targets.gtao_raw);
        bindings.sampler(generated::SMP_GTAO_SAMPLER, &self.gtao_sampler);
        frame.apply_pipeline(&self.gtao_blur_pipeline);
        frame.apply_bindings(&bindings);
        frame.apply_uniforms(generated::UB_GTAO_PARAMS, &uniforms);
        frame.draw(0, FULLSCREEN_VERTICES);
        frame.end_pass();
        frame.zone_end(Zone::GtaoBlur);
    }
}
