//! The per-frame uniform blocks, built from the frame's inputs.
//!
//! Their `#[repr(C)]` definitions and the size assertions that pin them to the
//! shader live in [`super::gpu_types`] (invariant 11); this is only the filling
//! in.

use crate::ibl::PREFILTER_MAX_LOD;
use crate::selection::SelectionView;
use crate::{
    ActiveMaterial, CameraProjection, EnvironmentSettings, GtaoSettings, OrbitCamera,
    SceneDebugOptions, SceneFrame, TonemapSettings, UvCamera, ViewportBackground,
};

use super::gpu_types::{
    GtaoUniforms, PostUniforms, SceneUniforms, buffer_view_value, shading_mode_value,
    skin_weight_value, vertex_color_value,
};

/// Whether this frame is one of the flat data-inspection views, which emit final
/// display pixels from the scene shader.
///
/// They bypass lighting, the composite's tone map and GTAO entirely — a value shown
/// through a tone curve is no longer the value — so the composite blits straight
/// through and the occlusion passes are skipped.
pub(super) fn flat_display(frame: &SceneFrame<'_>) -> bool {
    matches!(
        frame.debug.active_material,
        ActiveMaterial::Buffers | ActiveMaterial::SkinWeights
    )
}

/// Build the composite pass's [`PostUniforms`] from the live settings.
pub(super) fn post_uniforms(
    background: ViewportBackground,
    gtao_active: bool,
    tonemap: TonemapSettings,
    passthrough: bool,
) -> PostUniforms {
    let (top, bottom) = background.gradient_srgb();
    PostUniforms {
        gtao_enabled: u32::from(gtao_active),
        tonemap_enabled: u32::from(tonemap.enabled),
        tonemap_op: tonemap.operator.shader_index(),
        passthrough: u32::from(passthrough),
        bg_top: [top[0], top[1], top[2], 0.0],
        bg_bottom: [bottom[0], bottom[1], bottom[2], 0.0],
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera, projection, environment,
/// selection and debug options. The selection flash rides in `selection_color`
/// (gamma-space rgb + the flash fade in alpha, zero while nothing is
/// selected/flashing), read only by `fs_selection`.
pub(super) fn scene_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    environment: EnvironmentSettings,
    selection: SelectionView,
    debug: SceneDebugOptions,
    deform: bool,
) -> SceneUniforms {
    let view_projection = camera.view_projection(projection);
    let selection_color = if selection.selection.is_active() {
        let [r, g, b, _] = selection.highlight_color;
        [r, g, b, selection.fade.clamp(0.0, 1.0)]
    } else {
        [0.0; 4]
    };
    SceneUniforms {
        view_projection: view_projection.to_cols_array_2d(),
        inv_view_projection: view_projection.inverse().to_cols_array_2d(),
        render_options: [
            shading_mode_value(debug.shading_mode),
            if debug.active_material == ActiveMaterial::UvChecker {
                1.0
            } else {
                0.0
            },
            debug.uv_checker_tiling.max(1) as f32,
            vertex_color_value(debug),
        ],
        // `w` enables the vertex-stage deform path (skinning + blend shapes).
        camera_position: camera
            .eye_position()
            .extend(if deform { 1.0 } else { 0.0 })
            .to_array(),
        env_params: [
            if environment.ibl_enabled { 1.0 } else { 0.0 },
            environment.intensity.max(0.0),
            if environment.show_background {
                1.0
            } else {
                0.0
            },
            PREFILTER_MAX_LOD,
        ],
        projection_params: [
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            environment.rotation_degrees.to_radians(),
            // `z` carries the active buffer-inspection view index (-1 when off).
            buffer_view_value(debug),
            // `w` flags the skin-weight heat map.
            skin_weight_value(debug),
        ],
        view: camera.view_matrix().to_cols_array_2d(),
        selection_color,
    }
}

/// Build the per-frame [`GtaoUniforms`]. The settings' `radius` is a fraction of the
/// framed model's bounding-sphere radius, so it is scaled into view units by the live
/// `scene_radius` here, keeping the AO look scale-invariant.
///
/// `dims` is the occlusion target's size in pixels, and it rides in the two `w` slots
/// the D3D11 version left unused: sokol has no `GetDimensions`, so the shader cannot
/// ask. Leaving them zero does not fail — it silently makes the horizon search one
/// pixel wide, which is a scene with no ambient occlusion in it at all.
pub(super) fn build_gtao_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    gtao: GtaoSettings,
    dims: (u32, u32),
) -> GtaoUniforms {
    let scene_radius = camera.scene_radius.max(1e-3);
    let (slices, steps) = gtao.quality.slices_steps();
    GtaoUniforms {
        proj: camera.projection_matrix(projection).to_cols_array_2d(),
        params: [
            (gtao.radius * scene_radius).max(1e-4),
            gtao.intensity.max(0.0),
            gtao.thickness.clamp(0.0, 1.0),
            dims.0.max(1) as f32,
        ],
        config: [
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            slices.max(1) as f32,
            steps.max(1) as f32,
            dims.1.max(1) as f32,
        ],
    }
}

/// Build the [`SceneUniforms`] for the 2D UV viewport: only the UV camera's
/// orthographic view-projection matters (the grid / wireframe / fill return their own
/// vertex colour, never reaching the IBL / shading / selection code).
/// `projection_params.x = 1.0` marks orthographic.
pub(super) fn uv_scene_uniforms(uv_camera: UvCamera) -> SceneUniforms {
    SceneUniforms {
        view_projection: uv_camera.view_projection().to_cols_array_2d(),
        inv_view_projection: [[0.0; 4]; 4],
        render_options: [0.0; 4],
        camera_position: [0.0; 4],
        env_params: [0.0; 4],
        projection_params: [1.0, 0.0, 0.0, 0.0],
        view: [[0.0; 4]; 4],
        selection_color: [0.0; 4],
    }
}
