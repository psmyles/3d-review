//! The MSAA-dependent scene pipelines: the shader blobs they are built from, the
//! `SceneVertex` input layout every one of them reads, the named depth states that
//! distinguish them, and [`build_scene_pipelines`], which produces the whole set at
//! one sample count.
//!
//! They live together because they change together: their sample count is baked in
//! at creation, so an anti-aliasing change replaces the entire set at once. The
//! single-sample composite / GTAO pipelines are built in [`super::d3d`] alongside
//! the passes that use them.

use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::rhi::{
    BlendMode, Cull, DepthBias, DepthCompare, DepthState, InputElement, Pipeline, PipelineDesc,
    Topology, VertexFormat,
};

/// Compiled DXBC — see `build.rs`.
pub(super) const SCENE_VS: &[u8] = include_bytes!("../hlsl/scene.vs.dxbc");
const SCENE_LINE_PS: &[u8] = include_bytes!("../hlsl/scene.line.ps.dxbc");
const SCENE_MESH_PS: &[u8] = include_bytes!("../hlsl/scene.mesh.ps.dxbc");
const SCENE_SKYBOX_VS: &[u8] = include_bytes!("../hlsl/scene.skybox.vs.dxbc");
const SCENE_SKYBOX_PS: &[u8] = include_bytes!("../hlsl/scene.skybox.ps.dxbc");
const SCENE_SELECTION_PS: &[u8] = include_bytes!("../hlsl/scene.selection.ps.dxbc");

/// The `SceneVertex` input layout, in field order (offsets auto-computed). Must
/// match `#[repr(C)] SceneVertex` (`gpu_types`) and `VsInput` in `scene.hlsl`.
pub(super) const SCENE_VERTEX_LAYOUT: [InputElement; 6] = [
    InputElement::new("POSITION", 0, VertexFormat::Float3),
    InputElement::new("NORMAL", 0, VertexFormat::Float3),
    InputElement::new("TEXCOORD", 0, VertexFormat::Float2),
    InputElement::new("TANGENT", 0, VertexFormat::Float4),
    InputElement::new("COLOR", 0, VertexFormat::Float4),
    InputElement::new("BLENDINDICES", 0, VertexFormat::Uint4),
];

/// Every MSAA-dependent scene pipeline. They all draw into the MSAA scene MRT, so
/// their sample count is baked at the live AA level; [`SceneGpu`] stores this whole
/// set as one field and [`SceneGpu::rebuild_scene_pipelines`] replaces it wholesale,
/// so an AA change can't leave one of them behind at the old sample count.
///
/// [`SceneGpu`]: super::d3d::SceneGpu
/// [`SceneGpu::rebuild_scene_pipelines`]: super::d3d::SceneGpu
pub(super) struct ScenePipelineSet {
    /// Line overlays (grid / wireframe / bounding box / normal lines): depth-tested
    /// against the mesh, so edges on hidden faces are occluded.
    pub(super) line: Pipeline,
    /// Always-on-top line variant (depth compare `Always`, no write): used by the
    /// pivot marker and the skeleton outlines so they show through the mesh rather
    /// than being occluded inside it.
    pub(super) line_overlay: Pipeline,
    /// Always-on-top triangle fill, for the skeleton overlay's octahedra.
    pub(super) fill_overlay: Pipeline,
    /// The shaded mesh: back-face culled, depth-writing, biased back so coplanar
    /// line overlays win the test.
    pub(super) mesh: Pipeline,
    /// [`Self::mesh`] unculled, for the Backface Rendering option.
    pub(super) mesh_double_sided: Pipeline,
    /// The environment background, drawn first behind all geometry.
    pub(super) skybox: Pipeline,
    /// Flat-color triangle fill for the UV islands (`fs_main`'s zero-normal overlay
    /// branch — same as the line views but filled).
    pub(super) uv_fill: Pipeline,
    /// Selection-flash fill (`fs_selection`): flat highlight color × fade,
    /// depth-tested (Reversed-Z `GreaterEqual`) but no depth write, alpha-blended.
    /// Doubles as the Opt overlay's x-ray ghost.
    pub(super) selection: Pipeline,
}

/// Depth-tested against the mesh (Reversed-Z) but never depth-writing: the line
/// overlays, the UV island fill and the selection flash all layer onto a surface
/// the mesh pass already wrote, and must not push each other out of the way.
const DEPTH_TEST_ONLY: DepthState = DepthState {
    test: true,
    write: false,
    compare: DepthCompare::GreaterEqual,
};

/// Always-on-top: the comparison always passes, so the draw reads *through* solid
/// geometry. The pivot marker and the skeleton live inside the mesh, so depth-testing
/// them would hide them entirely; the skybox is behind everything by construction.
const DEPTH_ALWAYS: DepthState = DepthState {
    test: true,
    write: false,
    compare: DepthCompare::Always,
};

/// The mesh's own depth state — the only scene pipeline that writes depth.
const DEPTH_WRITE: DepthState = DepthState {
    test: true,
    write: true,
    compare: DepthCompare::GreaterEqual,
};

/// Build every MSAA-dependent scene pipeline at `sample_count`.
///
/// They share a vertex shader, the `SceneVertex` input layout and that sample
/// count, so `scene_desc` fills those in and each call below spells out only what
/// distinguishes its pipeline: the pixel shader, topology, culling, depth
/// behaviour and blending. The mesh adds a depth bias; the skybox is the one that
/// cannot use the helper, running its own vertex shader with no vertex input.
pub(super) fn build_scene_pipelines(
    device: &ID3D11Device,
    sample_count: u32,
) -> windows::core::Result<ScenePipelineSet> {
    let scene_desc =
        |ps: &'static [u8], topology: Topology, cull: Cull, depth: DepthState, blend: BlendMode| {
            PipelineDesc {
                vs: SCENE_VS,
                ps,
                input: &SCENE_VERTEX_LAYOUT,
                topology,
                cull,
                depth,
                blend,
                depth_bias: DepthBias::default(),
                sample_count,
            }
        };

    let line = Pipeline::new(
        device,
        &scene_desc(
            SCENE_LINE_PS,
            Topology::LineList,
            Cull::None,
            DEPTH_TEST_ONLY,
            BlendMode::AlphaBlend,
        ),
    )?;
    let line_overlay = Pipeline::new(
        device,
        &scene_desc(
            SCENE_LINE_PS,
            Topology::LineList,
            Cull::None,
            DEPTH_ALWAYS,
            BlendMode::AlphaBlend,
        ),
    )?;

    // Double-sided: an octahedron is viewed from every angle as the camera orbits.
    let fill_overlay = Pipeline::new(
        device,
        &scene_desc(
            SCENE_MESH_PS,
            Topology::TriangleList,
            Cull::None,
            DEPTH_ALWAYS,
            BlendMode::AlphaBlend,
        ),
    )?;

    // The mesh alone pushes its surface back (slope-scaled bias) so the coplanar
    // line overlays win the depth test against it.
    let mesh_desc = |cull| PipelineDesc {
        depth_bias: DepthBias {
            constant: -2,
            slope_scaled: -2.0,
        },
        ..scene_desc(
            SCENE_MESH_PS,
            Topology::TriangleList,
            cull,
            DEPTH_WRITE,
            BlendMode::AlphaBlend,
        )
    };
    let mesh = Pipeline::new(device, &mesh_desc(Cull::Back))?;
    let mesh_double_sided = Pipeline::new(device, &mesh_desc(Cull::None))?;

    // The one pipeline outside `scene_desc`: its own vertex shader builds a
    // fullscreen triangle from `SV_VertexID`, so it takes no vertex input.
    let skybox = Pipeline::new(
        device,
        &PipelineDesc {
            vs: SCENE_SKYBOX_VS,
            ps: SCENE_SKYBOX_PS,
            input: &[],
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: DEPTH_ALWAYS,
            blend: BlendMode::Opaque,
            depth_bias: DepthBias::default(),
            sample_count,
        },
    )?;

    // Everything the UV fill draws sits at z=0, so grid / fill / wireframe layer by
    // draw order rather than depth.
    let uv_fill = Pipeline::new(
        device,
        &scene_desc(
            SCENE_MESH_PS,
            Topology::TriangleList,
            Cull::None,
            DEPTH_TEST_ONLY,
            BlendMode::AlphaBlend,
        ),
    )?;
    let selection = Pipeline::new(
        device,
        &scene_desc(
            SCENE_SELECTION_PS,
            Topology::TriangleList,
            Cull::None,
            DEPTH_TEST_ONLY,
            BlendMode::AlphaBlend,
        ),
    )?;

    Ok(ScenePipelineSet {
        line,
        line_overlay,
        fill_overlay,
        mesh,
        mesh_double_sided,
        skybox,
        uv_fill,
        selection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant 11: the hand-maintained input-element list must cover exactly
    /// the `#[repr(C)] SceneVertex` — a drifted field order/size would misfeed
    /// the vertex shader with no runtime error.
    ///
    /// Checked per element rather than by total stride: the layout is
    /// `Float3, Float3, Float2, Float4, Float4, Uint4`, so swapping the two `[f32; 3]`
    /// fields (or the two `[f32; 4]` ones) on either side leaves the stride at 80
    /// while every affected attribute reads another field's bytes. Each element's
    /// running `APPEND_ALIGNED` offset is therefore compared against the offset of
    /// the struct field it is meant to feed, and its semantic against the name the
    /// shader reads that field by.
    #[test]
    fn scene_vertex_layout_matches_struct_fields() {
        use crate::scene::SceneVertex;
        use std::mem::{offset_of, size_of};

        // In layout order: the HLSL semantic, and the `SceneVertex` field whose
        // bytes it must land on.
        let fields = [
            ("POSITION", offset_of!(SceneVertex, position)),
            ("NORMAL", offset_of!(SceneVertex, normal)),
            ("TEXCOORD", offset_of!(SceneVertex, uv)),
            ("TANGENT", offset_of!(SceneVertex, tangent)),
            ("COLOR", offset_of!(SceneVertex, vertex_color)),
            ("BLENDINDICES", offset_of!(SceneVertex, deform)),
        ];
        assert_eq!(
            SCENE_VERTEX_LAYOUT.len(),
            fields.len(),
            "SCENE_VERTEX_LAYOUT must have one element per SceneVertex field"
        );

        let mut offset = 0;
        for (element, (semantic, field_offset)) in SCENE_VERTEX_LAYOUT.iter().zip(fields) {
            assert_eq!(
                element.semantic, semantic,
                "SCENE_VERTEX_LAYOUT element order must match SceneVertex field order"
            );
            assert_eq!(
                offset, field_offset,
                "{semantic} must start at the byte offset of the SceneVertex field it feeds"
            );
            offset += element.format.byte_size();
        }
        assert_eq!(
            offset,
            size_of::<SceneVertex>(),
            "SCENE_VERTEX_LAYOUT must cover every byte of SceneVertex"
        );
    }
}
