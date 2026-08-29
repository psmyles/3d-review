//! GPU-facing scene data that must stay in lockstep with the HLSL (invariant
//! 11): the per-vertex [`SceneVertex`] layout + [`SceneUniforms`] (`scene.hlsl`),
//! the composite [`PostUniforms`] (`post.hlsl`), the [`GtaoUniforms`]
//! (`gtao.hlsl`), and the small render-option encoders. The `#[repr(C)]` structs
//! below match their HLSL `cbuffer`/`VsInput` byte-for-byte; the matching
//! input-element list lives beside the pipeline builders in [`super::d3d`].
//!
//! Audit index — the remaining invariant-11 structs live with their subsystems:
//! `MaterialUniform` (`material/state.rs` ↔ `scene.hlsl` `b1`), `TexUniforms`
//! (`tex_d3d.rs` ↔ `tex.hlsl`), and the bake-only `FaceUniform` (`ibl.rs` ↔
//! `ibl.hlsl`).

use bytemuck::{Pod, Zeroable};

use crate::{ActiveMaterial, SceneDebugOptions, ShadingMode, VertexColorMode};

/// Composite-pass uniform (cbuffer `b3` in `post.hlsl`): the GTAO enable flag, the
/// tone-map enable + operator, and a raw-passthrough flag, all driven from the
/// live settings each frame.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PostUniforms {
    // The four scalars below fill exactly one 16-byte cbuffer register — HLSL
    // packs them implicitly, so adding/removing any one silently shifts `bg_top`
    // unless `post.hlsl` moves in lockstep. Keep the count a multiple of four.
    pub(crate) gtao_enabled: u32,
    pub(crate) tonemap_enabled: u32,
    pub(crate) tonemap_op: u32,
    /// When non-zero the composite blits the (already display-ready) scene color
    /// straight to the backbuffer — no GTAO, tone map or sRGB encode. Set for the
    /// [`ActiveMaterial::Buffers`] data-inspection view, whose scene shader emits
    /// final display pixels itself so the shown value is faithful.
    pub(crate) passthrough: u32,
    /// Viewport background fill in display (sRGB) space (`xyz`; `w` padding for the
    /// 16-byte cbuffer slot). The composite paints `lerp(bg_top, bg_bottom, v)`
    /// where the scene coverage is below 1; a flat preset sets both equal, the
    /// gradient distinct. Built from `ViewportBackground::gradient_srgb`.
    pub(crate) bg_top: [f32; 4],
    pub(crate) bg_bottom: [f32; 4],
}

// Byte-size lock against `post.hlsl`'s `b3`. The cbuffer is sized from
// `size_of::<T>()` and an upload is rejected only when it is *larger* than the
// buffer, so a field added on one side alone grows both and uploads happily while
// the shader keeps reading the old offsets — wrong pixels, not an error. Changing
// this number means the HLSL moved with it.
const _: () = assert!(std::mem::size_of::<PostUniforms>() == 48);

/// GTAO-pass uniform (cbuffer `b2` in `gtao.hlsl`): a `float4x4` + two `float4`s,
/// all 16-byte aligned. Uploaded each frame so the panel sliders stay live.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GtaoUniforms {
    /// View → clip projection (column-major), for reconstruction + sample projection.
    pub(crate) proj: [[f32; 4]; 4],
    /// x = radius (view units), y = intensity, z = thickness, w unused.
    pub(crate) params: [f32; 4],
    /// x = is_ortho (1.0 / 0.0), y = slice count, z = steps per slice, w unused.
    pub(crate) config: [f32; 4],
}

// Byte-size lock against `gtao.hlsl`'s cbuffer — see [`PostUniforms`] above for
// why a silent size drift is invisible at runtime.
const _: () = assert!(std::mem::size_of::<GtaoUniforms>() == 96);

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
    /// environment yaw in radians (IBL / skybox sample rotation), `z` = active
    /// buffer-inspection view index (-1 when the Buffers view is off), `w` = the
    /// skin-weight heat map is active (>0.5).
    pub(crate) projection_params: [f32; 4],
    /// View matrix (world → view), for writing the view-space normal + depth into
    /// the GTAO G-buffer.
    pub(crate) view: [[f32; 4]; 4],
    /// Selection-flash highlight: `rgb` = gamma-space highlight color, `w` = flash
    /// fade (1 at flash start → 0 when done). Read only by `fs_selection`; zero
    /// while nothing is flashing.
    pub(crate) selection_color: [f32; 4],
}

// Byte-size lock against `scene.hlsl`'s `b0` — see [`PostUniforms`] above.
const _: () = assert!(std::mem::size_of::<SceneUniforms>() == 272);

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

/// Encode the active buffer view into `projection_params.z` for the shader: `-1`
/// when the [`ActiveMaterial::Buffers`] view isn't active, otherwise the
/// [`crate::BufferView::shader_index`]. Carried in a previously-unused
/// `projection_params` slot, so the uniform layout is unchanged.
pub(super) fn buffer_view_value(debug_options: SceneDebugOptions) -> f32 {
    if debug_options.active_material != ActiveMaterial::Buffers {
        return -1.0;
    }
    debug_options.buffer_view.shader_index()
}

/// Encode the [`ActiveMaterial::SkinWeights`] heat map into `projection_params.w`
/// for the shader (`1` active, `0` not). The weight mesh carries real normals so
/// it can be Lambert-shaded, which means it can't be told apart by the zero-normal
/// overlay sentinel — hence an explicit flag, in the last free uniform slot.
pub(super) fn skin_weight_value(debug_options: SceneDebugOptions) -> f32 {
    if debug_options.active_material == ActiveMaterial::SkinWeights {
        1.0
    } else {
        0.0
    }
}
