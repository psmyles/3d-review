//! The scene pipeline set: the shader programs the scene pass draws with, the
//! `SceneVertex` layout every one of them reads, and the depth states that
//! distinguish them.
//!
//! They live together because they change together — they all render into the
//! offscreen 2-MRT scene pass, so its formats *and its sample count* are baked into
//! every one of them at creation, and an AA change replaces the whole set at once.

use crate::rhi::{
    Blend, Cull, Depth, DepthBias, GpuResult, Pipeline, PipelineDesc, Topology, VertexFormat,
    shader,
};
use crate::shaders::generated;

/// The `SceneVertex` attribute layout, in `layout(location=…)` order. Pinned to the
/// struct by the test below and to the shader by `shaders::tests`.
pub(super) const SCENE_VERTEX_LAYOUT: [VertexFormat; 6] = [
    VertexFormat::Float3, // position
    VertexFormat::Float3, // normal
    VertexFormat::Float2, // uv
    VertexFormat::Float4, // tangent
    VertexFormat::Float4, // vertex colour
    VertexFormat::Uint4,  // deform lane
];

/// Slope-scaled depth bias for the mesh: it pushes its surface back so the coplanar
/// line overlays win the Reversed-Z test against it.
const MESH_DEPTH_BIAS: DepthBias = DepthBias {
    constant: -2.0,
    slope_scaled: -2.0,
};

/// Every pipeline that draws into the scene pass. Held as one set rather than
/// unpacked into fields: an AA change replaces all of them at once, and one left
/// behind by a missed assignment would stay baked at the old sample count — a
/// validation error and undefined rendering, with nothing to catch it at compile
/// time.
pub(super) struct ScenePipelineSet {
    /// Line overlays (grid / wireframe / bounding box / normal lines): depth-tested
    /// against the mesh, so edges on hidden faces are occluded.
    pub(super) line: Pipeline,
    /// Always-on-top line variant: the pivot marker and the skeleton outlines, which
    /// live *inside* the mesh and would otherwise be invisible.
    pub(super) line_overlay: Pipeline,
    /// Always-on-top triangle fill, for the skeleton overlay's octahedra.
    pub(super) fill_overlay: Pipeline,
    /// The shaded mesh: back-face culled, depth-writing, biased back.
    pub(super) mesh: Pipeline,
    /// [`Self::mesh`] unculled, for the Backface Rendering option.
    pub(super) mesh_double_sided: Pipeline,
    /// The environment background, drawn first behind all geometry.
    pub(super) skybox: Pipeline,
    /// Flat-colour triangle fill for the UV islands (`fs_main`'s zero-normal overlay
    /// branch — the same shader as the mesh, filled rather than lined).
    pub(super) uv_fill: Pipeline,
    /// Selection-flash fill: flat highlight colour × fade, depth-tested but not
    /// depth-writing, alpha-blended. Doubles as the Opt overlay's x-ray ghost.
    pub(super) selection: Pipeline,
    /// The model wireframe: an *indexed* line draw over the mesh's own vertex
    /// buffer. It runs `fs_selection` rather than `fs_line` because those
    /// vertices carry the mesh's colours — the wireframe colour rides in the
    /// `selection_color` uniform instead, which is also what makes a colour
    /// change free. Depth behaviour matches [`Self::line`].
    pub(super) wireframe: Pipeline,
}

/// Build every scene pipeline.
///
/// They share `vs_main`, the `SceneVertex` layout and the pass's formats, so
/// `program` fills those in and each call spells out only what distinguishes it: the
/// program, topology, culling, depth behaviour, bias, and whether its draws are
/// indexed — the mesh and the selection flash index into the shared vertex buffer;
/// every line and fill view is its own vertex stream. The skybox is the one that
/// cannot use the helper: it runs its own vertex shader and takes no vertex input.
pub(super) fn build_scene_pipelines(sample_count: u32) -> GpuResult<ScenePipelineSet> {
    let program = |desc_fn: fn(sokol::gfx::Backend) -> sokol::gfx::ShaderDesc,
                   bytecode: &'static crate::rhi::shader::ShaderBytecode,
                   label: &'static std::ffi::CStr,
                   topology: Topology,
                   cull: Cull,
                   depth: Depth,
                   depth_bias: DepthBias,
                   indexed: bool|
     -> GpuResult<Pipeline> {
        Pipeline::new(&PipelineDesc {
            attributes: &SCENE_VERTEX_LAYOUT,
            indexed,
            topology,
            cull,
            depth,
            depth_bias,
            ..PipelineDesc::scene(shader::make(desc_fn, bytecode, label)?, sample_count, label)
        })
    };

    let line = program(
        generated::line_shader_desc,
        shader::bytecode!("line"),
        c"scene line",
        Topology::Lines,
        Cull::None,
        Depth::TEST_ONLY,
        DepthBias::default(),
        false,
    )?;
    let line_overlay = program(
        generated::line_shader_desc,
        shader::bytecode!("line"),
        c"scene line overlay",
        Topology::Lines,
        Cull::None,
        Depth::ALWAYS,
        DepthBias::default(),
        false,
    )?;
    // Double-sided: an octahedron is viewed from every angle as the camera orbits.
    let fill_overlay = program(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"scene fill overlay",
        Topology::Triangles,
        Cull::None,
        Depth::ALWAYS,
        DepthBias::default(),
        false,
    )?;
    let mesh = program(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"scene mesh",
        Topology::Triangles,
        Cull::Back,
        Depth::WRITE,
        MESH_DEPTH_BIAS,
        true,
    )?;
    let mesh_double_sided = program(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"scene mesh double-sided",
        Topology::Triangles,
        Cull::None,
        Depth::WRITE,
        MESH_DEPTH_BIAS,
        true,
    )?;
    // Everything the UV fill draws sits at z = 0, so grid / fill / wireframe layer by
    // draw order rather than by depth.
    let uv_fill = program(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"uv fill",
        Topology::Triangles,
        Cull::None,
        Depth::TEST_ONLY,
        DepthBias::default(),
        false,
    )?;
    let selection = program(
        generated::selection_shader_desc,
        shader::bytecode!("selection"),
        c"scene selection",
        Topology::Triangles,
        Cull::None,
        Depth::TEST_ONLY,
        DepthBias::default(),
        true,
    )?;

    let wireframe = program(
        generated::selection_shader_desc,
        shader::bytecode!("selection"),
        c"scene wireframe",
        Topology::Lines,
        Cull::None,
        Depth::TEST_ONLY,
        DepthBias::default(),
        true,
    )?;

    // The one pipeline outside the helper: its own vertex shader builds a fullscreen
    // triangle from `gl_VertexIndex`, so it takes no vertex input and no index buffer.
    let skybox_shader = shader::make(
        generated::skybox_shader_desc,
        shader::bytecode!("skybox"),
        c"skybox",
    )?;
    let skybox = Pipeline::new(&PipelineDesc {
        depth: Depth::ALWAYS,
        blend: Blend::Opaque,
        ..PipelineDesc::scene(skybox_shader, sample_count, c"skybox")
    })?;

    Ok(ScenePipelineSet {
        line,
        line_overlay,
        fill_overlay,
        mesh,
        mesh_double_sided,
        skybox,
        uv_fill,
        selection,
        wireframe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneVertex;
    use std::mem::{offset_of, size_of};

    /// Invariant 11: the hand-maintained attribute list must cover exactly the
    /// `#[repr(C)] SceneVertex` — a drifted field order or size would misfeed the
    /// vertex shader with no runtime error.
    ///
    /// Checked per attribute rather than by total stride: the layout is
    /// `Float3, Float3, Float2, Float4, Float4, Uint4`, so swapping the two `[f32; 3]`
    /// fields (or the two 16-byte ones) on either side leaves the stride at 80 while
    /// every affected attribute reads another field's bytes.
    #[test]
    fn the_scene_vertex_layout_matches_the_struct_fields() {
        let offsets = [
            offset_of!(SceneVertex, position),
            offset_of!(SceneVertex, normal),
            offset_of!(SceneVertex, uv),
            offset_of!(SceneVertex, tangent),
            offset_of!(SceneVertex, vertex_color),
            offset_of!(SceneVertex, deform),
        ];
        assert_eq!(
            SCENE_VERTEX_LAYOUT.len(),
            offsets.len(),
            "SCENE_VERTEX_LAYOUT must have one attribute per SceneVertex field"
        );
        let mut offset = 0;
        for (attribute, field_offset) in SCENE_VERTEX_LAYOUT.iter().zip(offsets) {
            assert_eq!(
                offset, field_offset as i32,
                "each attribute must start at the byte offset of the field it feeds"
            );
            offset += attribute.size();
        }
        assert_eq!(
            offset as usize,
            size_of::<SceneVertex>(),
            "SCENE_VERTEX_LAYOUT must cover every byte of SceneVertex"
        );
    }
}
