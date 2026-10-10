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

use super::ao_accum::{AoPlan, temporal_pattern};
use super::gpu_types::{
    GtaoMipUniforms, GtaoUniforms, LineUniforms, PostUniforms, SceneUniforms, buffer_view_value,
    shading_mode_value, skin_weight_value, vertex_color_value,
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
/// selection and debug options. The selection highlight rides in `selection_color`
/// (gamma-space rgb + the fill's opacity, zero while nothing is selected), read
/// only by `fs_selection`.
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
        selection.highlight_color
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

/// Width of every line overlay but the wireframe — the grid, the bounding box, the
/// normal and seam lines, the pivot, the skeleton's outlines and the UV view's lines —
/// in points. One point is one pixel at 100% and two on a Retina screen, so they keep
/// the same weight on every display, as the wireframe's own setting does.
pub(super) const OVERLAY_LINE_WIDTH: f32 = 1.0;
/// Width, in points, of the Aud workspace's offender edges.
pub(super) const AUDIT_EDGE_WIDTH: f32 = 2.5;
/// Size, in points, of the Aud workspace's offender dots.
pub(super) const AUDIT_DOT_WIDTH: f32 = 7.0;

/// The line programs' uniforms for one view: the wireframe at the user's width and
/// every other line at [`OVERLAY_LINE_WIDTH`], both against the view's target size.
#[derive(Clone, Copy)]
pub(super) struct LineWidths {
    pub(super) wireframe: LineUniforms,
    pub(super) overlay: LineUniforms,
    /// The Aud workspace's offender edges: heavier than an overlay line, so they
    /// read over the wireframe.
    pub(super) audit_edge: LineUniforms,
    /// The Aud workspace's offender dots — zero-length lines, drawn as squares of
    /// this width.
    pub(super) audit_dot: LineUniforms,
}

impl LineWidths {
    pub(super) fn new(scene: &SceneFrame<'_>, target_size: (u32, u32)) -> Self {
        Self {
            wireframe: line_uniforms(
                target_size,
                scene.debug.wireframe_width,
                scene.pixels_per_point,
            ),
            overlay: line_uniforms(target_size, OVERLAY_LINE_WIDTH, scene.pixels_per_point),
            audit_edge: line_uniforms(target_size, AUDIT_EDGE_WIDTH, scene.pixels_per_point),
            audit_dot: line_uniforms(target_size, AUDIT_DOT_WIDTH, scene.pixels_per_point),
        }
    }
}

/// Build a line draw's [`LineUniforms`] for a target of `target_size` pixels, with
/// the line `width_points` wide on a display of `pixels_per_point`.
///
/// The target size is the one the pass renders into, not the window — the Opt split
/// draws each half into its own half-width target, and the quad's pixel offsets are
/// turned back into clip space against whichever it is.
pub(super) fn line_uniforms(
    target_size: (u32, u32),
    width_points: f32,
    pixels_per_point: f32,
) -> LineUniforms {
    let width = width_points * pixels_per_point;
    LineUniforms {
        params: [
            target_size.0.max(1) as f32,
            target_size.1.max(1) as f32,
            if width.is_finite() {
                width.max(0.0)
            } else {
                0.0
            },
            0.0,
        ],
    }
}

/// Build the per-frame [`GtaoUniforms`]. The settings' `radius` is a multiplier over
/// the radius derived for this view ([`gtao_view_radius`]), which is what keeps the
/// occlusion looking the same on a 10 cm prop and a 10 km landscape.
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
    plan: AoPlan,
) -> GtaoUniforms {
    let (rotation, offset) = temporal_pattern(plan.temporal_index);
    GtaoUniforms {
        proj: camera.projection_matrix(projection).to_cols_array_2d(),
        params: [
            gtao_view_radius(camera, gtao),
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
            gtao.quality.slices_steps().0.max(1) as f32,
            gtao.quality.slices_steps().1.max(1) as f32,
            dims.1.max(1) as f32,
        ],
        temporal: [rotation, offset, plan.weight, 0.0],
    }
}

/// The AO radius in view units for this frame's camera. Shared with the depth
/// prefilter, whose falloff is scaled to the same radius.
///
/// Derived from what the viewport is *showing* rather than from the model's size —
/// see [`GtaoSettings::effective_radius`], which is where that choice is explained
/// and which the Ambient Occlusion panel reads too so the two can never disagree.
pub(super) fn gtao_view_radius(camera: OrbitCamera, gtao: GtaoSettings) -> f32 {
    gtao.effective_radius(camera.view_extent(), camera.scene_radius)
}

/// Build a depth-prefilter level's uniform: the size of the level being *read* (the
/// filter addresses its source's texels directly) and the AO radius its falloff is
/// scaled to.
pub(super) fn build_gtao_mip_uniforms(source_dims: (u32, u32), radius: f32) -> GtaoMipUniforms {
    GtaoMipUniforms {
        mip: [
            0.0,
            source_dims.0.max(1) as f32,
            source_dims.1.max(1) as f32,
            radius,
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
