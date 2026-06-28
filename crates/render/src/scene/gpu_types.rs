//! GPU-facing scene data that must stay in lockstep with `scene.hlsl`
//! (invariant 11): the per-vertex [`SceneVertex`] layout, the [`SceneUniforms`]
//! block, and the small render-option encoders. The `#[repr(C)]` structs below
//! match the HLSL `cbuffer`/`VsInput` byte-for-byte; the matching input-element
//! list lives beside the pipeline builders in [`super::d3d`].

use bytemuck::{Pod, Zeroable};

use crate::{ActiveMaterial, SceneDebugOptions, ShadingMode, VertexColorMode};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SceneUniforms {
    pub(crate) view_projection: [[f32; 4]; 4],
    /// Inverse view-projection, for reconstructing world ray directions in the
    /// skybox pass.
    pub(crate) inv_view_projection: [[f32; 4]; 4],
    pub(crate) render_options: [f32; 4],
    /// World-space camera eye in `xyz` (`w` is padding). Used by the shaded path
    /// to build the view vector for the specular highlight / reflection.
    pub(crate) camera_position: [f32; 4],
    /// Image-based lighting: `x` = IBL enabled (>0.5), `y` = intensity, `z` =
    /// show background skybox (>0.5), `w` = prefiltered-cube max mip LOD.
    pub(crate) env_params: [f32; 4],
    /// Projection metadata: `x` = orthographic projection (>0.5), `y` =
    /// environment yaw in radians (IBL / skybox sample rotation), z/w unused.
    pub(crate) projection_params: [f32; 4],
    /// View matrix (world → view), for writing the view-space normal + depth into
    /// the GTAO G-buffer.
    pub(crate) view: [[f32; 4]; 4],
    /// Selection-flash highlight: `rgb` = gamma-space highlight color, `w` = flash
    /// fade (1 at flash start → 0 when done). Read only by `fs_selection`; zero
    /// while nothing is flashing.
    pub(crate) selection_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SceneVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) uv: [f32; 2],
    /// World-space tangent (`xyz`) + handedness sign (`w`), forwarded for normal
    /// mapping. Zeroed-but-valid (`[1,0,0,1]`) on overlay/line/UV-fill geometry,
    /// which never samples a normal map.
    pub(crate) tangent: [f32; 4],
    /// The DCC vertex-color attribute on the mesh; on overlay/line/UV-fill
    /// geometry (zero normal) it instead carries the flat color the shader's
    /// overlay path returns. The material base color / smoothness come from the
    /// group-3 material uniform, not per vertex.
    pub(crate) vertex_color: [f32; 4],
}

pub(super) fn shading_mode_value(mode: ShadingMode) -> f32 {
    match mode {
        ShadingMode::Wireframe => 0.0,
        ShadingMode::Unlit => 1.0,
        ShadingMode::Shaded => 2.0,
    }
}

/// Encode the vertex-color view into `render_options.w` for the shader: `-1`
/// when the active material isn't vertex colors, otherwise the mode index (`0`
/// RGB, `1` Alpha, `2` RGB+A). One float keeps the uniform layout unchanged.
pub(super) fn vertex_color_value(debug_options: SceneDebugOptions) -> f32 {
    if debug_options.active_material != ActiveMaterial::VertexColors {
        return -1.0;
    }
    match debug_options.vertex_color_mode {
        VertexColorMode::Rgb => 0.0,
        VertexColorMode::Alpha => 1.0,
        VertexColorMode::RgbAlpha => 2.0,
    }
}
