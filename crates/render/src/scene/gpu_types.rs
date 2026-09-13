//! GPU-facing scene data that must stay in lockstep with the shader (invariant
//! 11): the per-vertex [`SceneVertex`] layout + [`SceneUniforms`] (the `mesh`
//! program), the composite [`PostUniforms`] (`post`), the [`GtaoUniforms`]
//! (`gtao`), and the small render-option encoders. Every program is generated
//! from the one source, `crates/render/src/shaders/review.glsl`; the vertex
//! layout each pipeline declares lives in [`super::pipelines`].
//!
//! Each struct below is pinned two ways: a literal `size_of` and a `size_of`
//! against **shdc's generated struct** for the same block. The second is what
//! makes the invariant about the shader rather than a hand-typed number, and the
//! per-field `offset_of` assertions beside it are what make it about the
//! *layout* rather than the total — two fields swapped keep the size and change
//! every value the shader reads.
//!
//! Audit index — the remaining invariant-11 structs live with their subsystems:
//! `MaterialUniform` (`material/state.rs` ↔ the `material` block), `TexUniforms`
//! (`tex/gpu.rs` ↔ `tex_params`), and the bake-only `FaceUniform` (`ibl.rs` ↔
//! `face_params`).

use bytemuck::{Pod, Zeroable};

use crate::shaders::assert_same_layout;

use crate::{ActiveMaterial, SceneDebugOptions, ShadingMode, VertexColorMode};

/// Composite-pass uniform (the `post` program's block): the GTAO enable flag, the
/// tone-map enable + operator, and a raw-passthrough flag, all driven from the
/// live settings each frame.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PostUniforms {
    // The four scalars below fill exactly one 16-byte cbuffer register — HLSL
    // packs them implicitly, so adding/removing any one silently shifts `bg_top`
    // unless the `post` program moves in lockstep. Keep the count a multiple of four.
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

// Byte-size lock against the `post` program's block. It is sized from
// `size_of::<T>()` and an upload is rejected only when it is *larger* than the
// buffer, so a field added on one side alone grows both and uploads happily while
// the shader keeps reading the old offsets — wrong pixels, not an error. Changing
// this number means the HLSL moved with it.
const _: () = assert!(std::mem::size_of::<PostUniforms>() == 48);
// ...and against the layout sokol-shdc generated from `review.glsl`, which is what
// turns invariant 11 into a check against the *shader* rather than against a number
// someone typed here. The shader's four flags are `int` (shdc's uniform-block subset
// has no unsigned type); same bits, same slot.
const _: () = assert!(
    std::mem::size_of::<PostUniforms>()
        == std::mem::size_of::<crate::shaders::generated::PostParams>()
);
// ...and field by field, since equal totals do not mean equal offsets.
assert_same_layout!(PostUniforms => crate::shaders::generated::PostParams, {
    gtao_enabled => gtao_enabled,
    tonemap_enabled => tonemap_enabled,
    tonemap_op => tonemap_op,
    passthrough => passthrough,
    bg_top => bg_top,
    bg_bottom => bg_bottom,
});

/// GTAO-pass uniform (`gtao_params` in `review.glsl`): a `float4x4` + three
/// `float4`s, all 16-byte aligned. Uploaded each frame so the panel sliders stay
/// live, and shared by the occlusion and denoise passes.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GtaoUniforms {
    /// View → clip projection (column-major), for reconstruction + sample projection.
    pub(crate) proj: [[f32; 4]; 4],
    /// x = radius (view units), y = intensity, z = thin-occluder compensation,
    /// w = target width in pixels (sokol has no `GetDimensions`).
    pub(crate) params: [f32; 4],
    /// x = is_ortho (1.0 / 0.0), y = slice count, z = steps per slice,
    /// w = target height in pixels.
    pub(crate) config: [f32; 4],
    /// The accumulation's temporal state ([`super::ao_accum`]): x = this frame's
    /// extra slice rotation, y = its extra step offset, z = the weight this frame's
    /// estimate takes in the running mean (exactly 1.0 means "reset — do not read
    /// the history"), w spare.
    pub(crate) temporal: [f32; 4],
}

// Byte-size lock against the shader's block — see [`PostUniforms`] above for
// why a silent size drift is invisible at runtime.
const _: () = assert!(std::mem::size_of::<GtaoUniforms>() == 112);
const _: () = assert!(
    std::mem::size_of::<GtaoUniforms>()
        == std::mem::size_of::<crate::shaders::generated::GtaoParams>()
);
assert_same_layout!(GtaoUniforms => crate::shaders::generated::GtaoParams, {
    proj => proj,
    params => params,
    config => config,
    temporal => temporal,
});

/// Depth-prefilter uniform (`gtao_mip_params` in `review.glsl`): the one row the
/// 2×2 reduction needs.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GtaoMipUniforms {
    /// x = spare, y / z = the *source* level's size in pixels, w = the AO radius in
    /// view units (the filter's falloff is scaled to it).
    pub(crate) mip: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GtaoMipUniforms>() == 16);
const _: () = assert!(
    std::mem::size_of::<GtaoMipUniforms>()
        == std::mem::size_of::<crate::shaders::generated::GtaoMipParams>()
);
assert_same_layout!(GtaoMipUniforms => crate::shaders::generated::GtaoMipParams, {
    mip => mip,
});

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SceneUniforms {
    pub(crate) view_projection: [[f32; 4]; 4],
    /// Inverse view-projection, for reconstructing world ray directions in the
    /// skybox pass.
    pub(crate) inv_view_projection: [[f32; 4]; 4],
    pub(crate) render_options: [f32; 4],
    /// World-space camera eye in `xyz`. Used by the shaded path to build the view
    /// vector for the specular highlight / reflection. `w` flags the GPU deform
    /// path (>0.5): the vertex shader then applies the blend-shape deltas and the
    /// skinning palette bound at `t12..t15` to every vertex carrying a non-empty
    /// [`SceneVertex::deform`] lane.
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
    /// Selection / hover highlight: `rgb` = gamma-space highlight color, `w` = the
    /// fill's opacity. Read only by `fs_selection`; zero when there is nothing to
    /// draw.
    pub(crate) selection_color: [f32; 4],
}

// Byte-size lock against the `mesh` program's scene block — see [`PostUniforms`] above.
const _: () = assert!(std::mem::size_of::<SceneUniforms>() == 272);
// One value, uploaded to both stages, so it is pinned against *both* generated
// blocks — `scene_vs` and `scene_fs` are the same bytes declared twice because a
// sokol uniform block belongs to exactly one stage.
const _: () = assert!(
    std::mem::size_of::<SceneUniforms>()
        == std::mem::size_of::<crate::shaders::generated::SceneVs>()
);
const _: () = assert!(
    std::mem::size_of::<SceneUniforms>()
        == std::mem::size_of::<crate::shaders::generated::SceneFs>()
);
// One value uploaded to both stages, so both blocks are pinned field by field.
assert_same_layout!(SceneUniforms => crate::shaders::generated::SceneVs, {
    view_projection => view_projection,
    inv_view_projection => inv_view_projection,
    render_options => render_options,
    camera_position => camera_position,
    env_params => env_params,
    projection_params => projection_params,
    view => view,
    selection_color => selection_color,
});
assert_same_layout!(SceneUniforms => crate::shaders::generated::SceneFs, {
    view_projection => view_projection,
    inv_view_projection => inv_view_projection,
    render_options => render_options,
    camera_position => camera_position,
    env_params => env_params,
    projection_params => projection_params,
    view => view,
    selection_color => selection_color,
});

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
    /// The deform lane (`BLENDINDICES`): `x` / `y` = first index + count of this
    /// vertex's run in the influence buffer (`t12`), `z` / `w` = first index +
    /// count of its run in the blend-shape delta buffer (`t14`). All zero for
    /// geometry that never deforms (grid, bounding box, UV islands); a rigid
    /// mesh vertex references its node's single-entry run. Built by
    /// `geometry::deform`.
    pub(crate) deform: [u32; 4],
}

// Byte-size lock against the shader's vertex input + `SCENE_VERTEX_LAYOUT` — the
// per-field pins live in the layout test beside the pipelines.
const _: () = assert!(std::mem::size_of::<SceneVertex>() == 80);

/// One entry of the influence buffer (`StructuredBuffer<InfluenceEntry>` at VS
/// `t12`): a palette entry and its weight. A vertex's run is `deform.x ..
/// deform.x + deform.y` of these.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(crate) struct InfluenceEntry {
    pub(crate) entry: u32,
    pub(crate) weight: f32,
}

const _: () = assert!(std::mem::size_of::<InfluenceEntry>() == 8);
const _: () = assert!(
    std::mem::size_of::<InfluenceEntry>()
        == std::mem::size_of::<crate::shaders::generated::Influenceentry>()
);
assert_same_layout!(InfluenceEntry => crate::shaders::generated::Influenceentry, {
    entry => entry,
    weight => weight,
});

/// One palette entry (`StructuredBuffer<PaletteEntry>` at VS `t13`): the three
/// rows of an affine 3×4 matrix, spelled as rows rather than an HLSL matrix type
/// so the structured-buffer packing is unambiguous on both sides. Entry `i <
/// node_count` is node `i`'s rigid delta, the rest are the skin clusters' — see
/// `review_model::anim::build_palette`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(crate) struct PaletteEntry {
    pub(crate) r0: [f32; 4],
    pub(crate) r1: [f32; 4],
    pub(crate) r2: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<PaletteEntry>() == 48);
const _: () = assert!(
    std::mem::size_of::<PaletteEntry>()
        == std::mem::size_of::<crate::shaders::generated::Paletteentry>()
);
assert_same_layout!(PaletteEntry => crate::shaders::generated::Paletteentry, {
    r0 => r0,
    r1 => r1,
    r2 => r2,
});

impl PaletteEntry {
    /// The affine rows of `matrix` (column-major on the Rust side, so the rows
    /// are the transpose's columns).
    pub(crate) fn from_mat4(matrix: glam::Mat4) -> Self {
        let rows = matrix.transpose();
        Self {
            r0: rows.x_axis.to_array(),
            r1: rows.y_axis.to_array(),
            r2: rows.z_axis.to_array(),
        }
    }
}

/// One blend-shape delta (`StructuredBuffer<MorphEntry>` at VS `t14`): the
/// shape whose weight (`t15`) scales it, and the world-oriented position /
/// normal offsets. A vertex's run is `deform.z .. deform.z + deform.w`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(crate) struct MorphEntry {
    pub(crate) shape: u32,
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
}

const _: () = assert!(std::mem::size_of::<MorphEntry>() == 28);
// The one that had to be checked rather than assumed: `review.glsl` declares this
// entry as seven scalars, because a `vec3` member would pad the std430 struct to 48
// bytes and shift every entry after the first. shdc emits `align(4)`, so the two
// agree at 28 — but only as long as the shader keeps spelling it out.
const _: () = assert!(
    std::mem::size_of::<MorphEntry>()
        == std::mem::size_of::<crate::shaders::generated::Morphentry>()
);
// The shader spells the two vectors out as scalars (see above), so each Rust
// array is pinned against the first of its three — which is the offset that
// matters: a `vec3` member here would pad the entry and shift every one after it.
assert_same_layout!(MorphEntry => crate::shaders::generated::Morphentry, {
    shape => shape,
    position => px,
    normal => nx,
});

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
