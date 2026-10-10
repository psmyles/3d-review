//! The Aud workspace's split view: the model on the left, the focused finding's
//! UV layout on the right, the offenders highlighted in both.
//!
//! Laid out like the Opt split (`opt.rs`) — each half in its own target set
//! sized to half the viewport, composited into its own half — but the right
//! half is the UV view's recording rather than a second 3D view. Which is why
//! the UV buffers are released by the single-view 3D entry points rather than by
//! `sync_frame`: this frame keeps both.

use crate::material::MaterialState;
use crate::rhi::{Frame, GpuResult};
use crate::{OrbitCamera, SceneFrame, SceneViewport, UvCamera, UvFrame};

use super::gpu::SceneGpu;
use super::slot::SlotId;
use super::targets::{BackbufferRect, TargetSetId};

impl SceneGpu {
    #[allow(clippy::too_many_arguments)] // The two views' inputs, each needed as given.
    pub(crate) fn render_aud_split(
        &mut self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        uv: &UvFrame<'_>,
        camera: OrbitCamera,
        uv_camera: UvCamera,
        viewport: SceneViewport,
        material_states: &[MaterialState],
        material_revision: u64,
    ) -> GpuResult<()> {
        self.activate(SlotId::Source);
        self.release_ghost_wireframes();
        // The composites cover only the chrome-free rect; black under the chrome.
        frame.set_clear([0.0; 3]);

        let height = viewport.height.max(1);
        let half = (viewport.width / 2).max(1);
        let target = (half, height);
        let mut camera = camera;
        camera.aspect_ratio = half as f32 / height as f32;
        let mut uv_camera = uv_camera;
        uv_camera.aspect_ratio = camera.aspect_ratio;

        self.sync_frame(frame, scene, material_states, material_revision, target)?;
        self.sync_ao_history(TargetSetId::Primary, camera, scene);
        self.sync_split_targets(target)?;
        self.sync_uv_frame(uv, target)?;

        let left = self.record_view(
            frame,
            scene,
            camera,
            self.targets(),
            BackbufferRect {
                x: viewport.x,
                y: viewport.y,
                width: half,
                height,
            },
        );
        self.record_uv_view(
            frame,
            uv,
            uv_camera,
            self.split_targets(),
            BackbufferRect {
                x: viewport.x + half,
                y: viewport.y,
                width: viewport.width.saturating_sub(half).max(1),
                height,
            },
        );
        left
    }
}
