//! The 2D UV viewport.
//!
//! Its own camera, its own targets and its own derived views — the one scene
//! path that shares no state with the 3D pass, which is why `release_uv_views`
//! is called from the 3D path's `sync_frame` rather than from here.

use review_model::ModelData;

use crate::rhi::{Frame, GpuResult};
use crate::shaders::generated;
use crate::{AntiAliasing, TonemapSettings, UvCamera, UvShadingMode, ViewportBackground};

use super::slot::SlotId;

use super::gpu::SceneGpu;
use super::targets::BackbufferRect;
use super::uniforms::{post_uniforms, uv_scene_uniforms};

impl SceneGpu {
    /// Render the 2D UV viewport (instead of the 3D scene): the 0..1 grid, the
    /// optional island fill (Shaded / Islands modes), then the model's UV edges on
    /// top — all framed by the 2D `uv_camera` and composited like the 3D scene
    /// (tone-mapped, no GTAO).
    // Independent per-frame inputs (frame + model + revision + camera + channel +
    // shading mode + background); none is redundant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_uv(
        &mut self,
        frame: &mut Frame<'_>,
        model: &ModelData,
        model_revision: u64,
        uv_camera: UvCamera,
        channel: u32,
        shading_mode: UvShadingMode,
        anti_aliasing: AntiAliasing,
        background: ViewportBackground,
    ) -> GpuResult<()> {
        // The UV viewport shows the source model, so its derived buffers belong in the
        // source slot (see the note in `render`).
        self.activate(SlotId::Source);
        let size = frame.size();
        self.sync_targets(frame, size, anti_aliasing.effective_sample_count())?;
        self.sync_uv_view(model, model_revision, channel, shading_mode)?;

        // The UV camera's orthographic view-projection; the rest of the uniform is
        // unused by the UV path (lines and fills return their own vertex colour).
        let uniforms = uv_scene_uniforms(uv_camera);

        // Clear to zero (radiance + coverage); the background is painted in the
        // composite, matching the 3D path.
        let targets = &self.targets;
        frame.begin_offscreen_pass(
            &[&targets.color, &targets.ambient],
            Some(&targets.depth),
            [0.0; 4],
            c"uv scene",
        );

        // Reference grid first.
        self.draw_lines(frame, &self.scene.line, &[&self.uv_grid], &uniforms);

        // Island fill (Shaded / Islands), under the wireframe. It runs `fs_main`, so
        // it binds the full material set even though the zero-normal branch it takes
        // never uses the sampled values.
        if let Some(fill) = &self.active.views.uv_fill_buf {
            let material = self.materials.fallback();
            let mut bindings = self.mesh_bindings(&self.checker_greyscale, material);
            bindings.mesh_vertices(fill);
            frame.apply_pipeline(&self.scene.uv_fill);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, &uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, &uniforms);
            frame.apply_uniforms(generated::UB_MATERIAL, material.uniform());
            frame.draw(0, fill.count());
        }

        // The model's UV edges on top.
        if let Some(wireframe) = &self.active.views.uv_wireframe_buf {
            self.draw_lines(frame, &self.scene.line, &[wireframe], &uniforms);
        }
        frame.end_pass();

        // Composite: tone-mapped (the default operator, so shaded fills read like the
        // 3D scene), no GTAO (the flat UV viewport has no depth to occlude), over the
        // chosen viewport background.
        let post = post_uniforms(background, false, TonemapSettings::default(), false);
        self.queue_composite(frame, &post, targets, None, BackbufferRect::full(size));
        Ok(())
    }
}
