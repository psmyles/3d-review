//! The scene's pipelines: the shader programs the scene and line passes draw with,
//! the `SceneVertex` layout the triangle pipelines read, and the depth states that
//! distinguish them.
//!
//! Two sets, split by the pass they draw into. [`ScenePipelineSet`] is everything in
//! the multisampled scene pass: its formats *and its sample count* are baked into
//! every one of them at creation, so an AA change replaces the whole set at once.
//! [`LinePipelineSet`] is everything in the single-sample line pass — the line
//! overlays, which antialias their own edges, and what has to layer with them — so it
//! is built once and never depends on the AA level. At 1× the scene pass is
//! single-sample as well, and the line set draws straight into it.

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
/// line overlays win the Reversed-Z test against it. The line pass's depth-only redraw
/// of the mesh uses the same bias, or the lines would lose to their own surface there.
const MESH_DEPTH_BIAS: DepthBias = DepthBias {
    constant: -2.0,
    slope_scaled: -2.0,
};

/// Every pipeline that draws into the multisampled scene pass. Held as one set rather
/// than unpacked into fields: an AA change replaces all of them at once, and one left
/// behind by a missed assignment would stay baked at the old sample count — a
/// validation error and undefined rendering, with nothing to catch it at compile
/// time.
pub(super) struct ScenePipelineSet {
    /// The shaded mesh: back-face culled, depth-writing, biased back.
    pub(super) mesh: Pipeline,
    /// [`Self::mesh`] unculled, for the Backface Rendering option.
    pub(super) mesh_double_sided: Pipeline,
    /// The environment background, drawn first behind all geometry.
    pub(super) skybox: Pipeline,
    /// Flat-colour triangle fill for the UV islands (`fs_main`'s zero-normal overlay
    /// branch — the same shader as the mesh, filled rather than lined).
    pub(super) uv_fill: Pipeline,
    /// Selection-highlight fill: flat highlight colour × its opacity, depth-tested but
    /// not depth-writing, alpha-blended. Doubles as the Opt overlay's x-ray ghost.
    pub(super) selection: Pipeline,
    /// The one line drawn *inside* the multisampled pass: the UV view's 0..1 grid,
    /// which lies under the texture and the island fill and so has to be drawn before
    /// them. A handful of lines, so its per-sample cost is nothing.
    pub(super) underlay_line: Pipeline,
}

/// Every pipeline that draws into the single-sample line pass — or, at 1×, at the end
/// of the scene pass, which is then single-sample too. Built once: nothing here
/// depends on the AA level.
pub(super) struct LinePipelineSet {
    /// Line overlays (grid / bounding box / normal and seam lines / the UV view's
    /// edges): depth-tested against the mesh, so lines behind it are occluded.
    pub(super) line: Pipeline,
    /// Always-on-top line variant: the pivot marker and the skeleton outlines, which
    /// live *inside* the mesh and would otherwise be invisible.
    pub(super) line_overlay: Pipeline,
    /// The model wireframe (the `wire` program): its edges pulled from the mesh's own
    /// vertex buffer through an edge list, its colour from the `selection_color`
    /// uniform, which is what makes a colour change free. Depth behaviour matches
    /// [`Self::line`].
    pub(super) wireframe: Pipeline,
    /// Always-on-top triangle fill, for the skeleton overlay's octahedra. Here rather
    /// than in the scene set because it layers between the line views and the
    /// skeleton's own outlines; its edges are under those outlines, so the lost MSAA
    /// does not show.
    pub(super) fill_overlay: Pipeline,
    /// The mesh again, depth only: what the line pass tests against when the scene
    /// pass was multisampled and its depth cannot be read. Same culling and bias as
    /// [`ScenePipelineSet::mesh`], so a line is hidden exactly where it was before.
    pub(super) depth_prepass: Pipeline,
    /// [`Self::depth_prepass`] unculled, for the Backface Rendering option.
    pub(super) depth_prepass_double_sided: Pipeline,
}

/// The line programs take no vertex input: `vs_line` / `vs_wire` build their quads
/// from `gl_VertexIndex` and the storage buffers. Unculled — a quad's winding depends
/// on which way its line runs on screen — and alpha-blended, the scene defaults.
fn line_pipeline(
    desc_fn: crate::rhi::shader::ShaderDescFn,
    bytecode: &'static crate::rhi::shader::ShaderBytecode,
    depth: Depth,
    sample_count: u32,
    label: &'static std::ffi::CStr,
) -> GpuResult<Pipeline> {
    Pipeline::new(&PipelineDesc {
        depth,
        ..PipelineDesc::scene(shader::make(desc_fn, bytecode, label)?, sample_count, label)
    })
}

/// A pipeline over the `SceneVertex` layout, drawing into the scene's formats at
/// `sample_count`: everything that runs `vs_main`. Each call spells out only what
/// distinguishes it — the program, topology, culling, depth behaviour, bias, and
/// whether its draws are indexed (the mesh and the selection highlight index into the
/// shared vertex buffer; every fill view is its own vertex stream).
#[allow(clippy::too_many_arguments)]
fn vertex_pipeline(
    desc_fn: crate::rhi::shader::ShaderDescFn,
    bytecode: &'static crate::rhi::shader::ShaderBytecode,
    label: &'static std::ffi::CStr,
    topology: Topology,
    cull: Cull,
    depth: Depth,
    depth_bias: DepthBias,
    indexed: bool,
    sample_count: u32,
) -> GpuResult<Pipeline> {
    Pipeline::new(&PipelineDesc {
        attributes: &SCENE_VERTEX_LAYOUT,
        indexed,
        topology,
        cull,
        depth,
        depth_bias,
        ..PipelineDesc::scene(shader::make(desc_fn, bytecode, label)?, sample_count, label)
    })
}

/// Build every pipeline of the multisampled scene pass at `sample_count`.
pub(super) fn build_scene_pipelines(sample_count: u32) -> GpuResult<ScenePipelineSet> {
    let mesh = vertex_pipeline(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"scene mesh",
        Topology::Triangles,
        Cull::Back,
        Depth::WRITE,
        MESH_DEPTH_BIAS,
        true,
        sample_count,
    )?;
    let mesh_double_sided = vertex_pipeline(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"scene mesh double-sided",
        Topology::Triangles,
        Cull::None,
        Depth::WRITE,
        MESH_DEPTH_BIAS,
        true,
        sample_count,
    )?;
    // Everything the UV fill draws sits at z = 0, so grid / fill / wireframe layer by
    // draw order rather than by depth.
    let uv_fill = vertex_pipeline(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"uv fill",
        Topology::Triangles,
        Cull::None,
        Depth::TEST_ONLY,
        DepthBias::default(),
        false,
        sample_count,
    )?;
    let selection = vertex_pipeline(
        generated::selection_shader_desc,
        shader::bytecode!("selection"),
        c"scene selection",
        Topology::Triangles,
        Cull::None,
        Depth::TEST_ONLY,
        DepthBias::default(),
        true,
        sample_count,
    )?;
    let underlay_line = line_pipeline(
        generated::line_shader_desc,
        shader::bytecode!("line"),
        Depth::TEST_ONLY,
        sample_count,
        c"scene underlay line",
    )?;

    // Its own vertex shader builds a fullscreen triangle from `gl_VertexIndex`, so it
    // takes no vertex input and no index buffer.
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
        mesh,
        mesh_double_sided,
        skybox,
        uv_fill,
        selection,
        underlay_line,
    })
}

/// Build every pipeline of the single-sample line pass.
pub(super) fn build_line_pipelines() -> GpuResult<LinePipelineSet> {
    const SINGLE_SAMPLE: u32 = 1;
    let line = line_pipeline(
        generated::line_shader_desc,
        shader::bytecode!("line"),
        Depth::TEST_ONLY,
        SINGLE_SAMPLE,
        c"scene line",
    )?;
    let line_overlay = line_pipeline(
        generated::line_shader_desc,
        shader::bytecode!("line"),
        Depth::ALWAYS,
        SINGLE_SAMPLE,
        c"scene line overlay",
    )?;
    let wireframe = line_pipeline(
        generated::wire_shader_desc,
        shader::bytecode!("wire"),
        Depth::TEST_ONLY,
        SINGLE_SAMPLE,
        c"scene wireframe",
    )?;
    // Double-sided: an octahedron is viewed from every angle as the camera orbits.
    let fill_overlay = vertex_pipeline(
        generated::mesh_shader_desc,
        shader::bytecode!("mesh"),
        c"scene fill overlay",
        Topology::Triangles,
        Cull::None,
        Depth::ALWAYS,
        DepthBias::default(),
        false,
        SINGLE_SAMPLE,
    )?;
    // Depth only, through the cheapest program `vs_main` has a pairing for: the
    // selection fill, whose colour never lands because nothing is written.
    let depth_prepass = |cull: Cull, label: &'static std::ffi::CStr| {
        Pipeline::new(&PipelineDesc {
            attributes: &SCENE_VERTEX_LAYOUT,
            indexed: true,
            cull,
            depth: Depth::WRITE,
            depth_bias: MESH_DEPTH_BIAS,
            color_write: false,
            ..PipelineDesc::scene(
                shader::make(
                    generated::selection_shader_desc,
                    shader::bytecode!("selection"),
                    label,
                )?,
                SINGLE_SAMPLE,
                label,
            )
        })
    };
    Ok(LinePipelineSet {
        line,
        line_overlay,
        wireframe,
        fill_overlay,
        depth_prepass: depth_prepass(Cull::Back, c"line pass depth")?,
        depth_prepass_double_sided: depth_prepass(Cull::None, c"line pass depth double-sided")?,
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
