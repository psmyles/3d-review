//! GPU-facing scene data that must stay in lockstep with `scene.wgsl`
//! (invariant 11): the per-vertex [`SceneVertex`] layout, the [`SceneUniforms`]
//! block, and the small render-option encoders. The shader source + its naga
//! validation test live here too, beside the structs they must match, so the
//! lockstep is obvious and guarded in one place.

use bytemuck::{Pod, Zeroable};

use crate::{ActiveMaterial, SceneDebugOptions, ShadingMode, VertexColorMode};

/// The scene shader (shaded / unlit / wireframe / uv-checker paths). Kept in a
/// sibling `.wgsl` file but the `SceneUniforms` / `SceneVertex` layouts below must
/// track the `#[repr(C)]` structs (invariant 11).
pub(super) const SHADER: &str = include_str!("../scene.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct SceneUniforms {
    pub(super) view_projection: [[f32; 4]; 4],
    /// Inverse view-projection, for reconstructing world ray directions in the
    /// skybox pass.
    pub(super) inv_view_projection: [[f32; 4]; 4],
    pub(super) render_options: [f32; 4],
    /// World-space camera eye in `xyz` (`w` is padding). Used by the shaded path
    /// to build the view vector for the specular highlight / reflection.
    pub(super) camera_position: [f32; 4],
    /// Image-based lighting: `x` = IBL enabled (>0.5), `y` = intensity, `z` =
    /// show background skybox (>0.5), `w` = prefiltered-cube max mip LOD.
    pub(super) env_params: [f32; 4],
    /// Projection metadata: `x` = orthographic projection (>0.5), `y` =
    /// environment yaw in radians (IBL / skybox sample rotation), z/w unused.
    pub(super) projection_params: [f32; 4],
    /// View matrix (world → view), for writing the view-space normal + depth into
    /// the SSAO G-buffer.
    pub(super) view: [[f32; 4]; 4],
    /// Selection-flash highlight: `rgb` = gamma-space highlight color, `w` = flash
    /// fade (1 at flash start → 0 when done). Read only by `fs_selection`; zero
    /// while nothing is flashing.
    pub(super) selection_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SceneVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) uv: [f32; 2],
    /// World-space tangent (`xyz`) + handedness sign (`w`), forwarded for normal
    /// mapping (Phase 3). Zeroed-but-valid (`[1,0,0,1]`) on overlay/line/UV-fill
    /// geometry, which never samples a normal map.
    pub(crate) tangent: [f32; 4],
    /// The DCC vertex-color attribute on the mesh; on overlay/line/UV-fill
    /// geometry (zero normal) it instead carries the flat color the shader's
    /// overlay path returns. The material base color / smoothness are no longer
    /// baked per vertex (Phase 1) — they come from the group-3 material uniform.
    pub(crate) vertex_color: [f32; 4],
}

impl SceneVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4
    ];

    pub(super) fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
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

#[cfg(test)]
mod shader_tests {
    /// The hand-written scene shader must parse + validate. The real check is GPU
    /// pipeline creation, but validating with naga here catches type / control-flow
    /// / binding-layout mistakes (e.g. a `textureSample` in non-uniform flow, or a
    /// `MaterialUniform` field mismatch) without a device.
    #[test]
    fn scene_shader_validates() {
        let module = naga::front::wgsl::parse_str(super::SHADER).expect("scene.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("scene.wgsl should validate");
    }
}
