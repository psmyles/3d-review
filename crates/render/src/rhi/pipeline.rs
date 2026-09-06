//! Pipelines: a shader plus the fixed-function state it draws with.
//!
//! One [`Pipeline`] owns its shader, because nothing in this renderer shares a shader
//! across pipelines with *different* state without also sharing the pipeline — the
//! scene's variants each carry their own — and one owner means one `Drop`.
//!
//! A [`PipelineDesc`] is built from one of the two constructors that fix where it
//! draws — [`PipelineDesc::swapchain`] for the frame's own pass,
//! [`PipelineDesc::scene`] for the offscreen linear-HDR MRT — and then overridden
//! field by field with `..`. That is deliberate: the target formats, the depth format
//! and the sample count must agree with the pass, sokol validates that they do, and
//! spelling them out at every call site is how they drift.
//!
//! A pipeline's `sample_count` has to agree with the pass it draws into, which is
//! why the scene set is rebuilt whole whenever the AA level changes rather than
//! patched — sokol validates the match, but only at draw time.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};
use super::format::{Format, SCENE_COLOR_FORMAT, SCENE_DEPTH_FORMAT};

/// How one vertex attribute's bytes are read. Sequential in the vertex, so the
/// stride and each offset follow from the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VertexFormat {
    /// Two `f32` — a position or a UV.
    Float2,
    /// Three `f32` — a position or a normal.
    Float3,
    /// Four `f32` — a tangent or a colour.
    Float4,
    /// Four `u32` — the `SceneVertex` deform lane.
    Uint4,
    /// Four `u8` normalised to 0..1 — a packed colour.
    Ubyte4N,
}

impl VertexFormat {
    /// Bytes this attribute occupies in the vertex.
    pub(crate) const fn size(self) -> i32 {
        match self {
            Self::Float2 => 8,
            Self::Float3 => 12,
            Self::Float4 | Self::Uint4 => 16,
            Self::Ubyte4N => 4,
        }
    }

    const fn sg(self) -> sg::VertexFormat {
        match self {
            Self::Float2 => sg::VertexFormat::Float2,
            Self::Float3 => sg::VertexFormat::Float3,
            Self::Float4 => sg::VertexFormat::Float4,
            Self::Uint4 => sg::VertexFormat::Uint4,
            Self::Ubyte4N => sg::VertexFormat::Ubyte4n,
        }
    }
}

/// What a draw assembles its vertices into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Topology {
    Triangles,
    /// Independent line segments — every debug line view, and the model wireframe.
    Lines,
}

impl Topology {
    const fn sg(self) -> sg::PrimitiveType {
        match self {
            Self::Triangles => sg::PrimitiveType::Triangles,
            Self::Lines => sg::PrimitiveType::Lines,
        }
    }
}

/// Which faces the rasterizer discards. Front faces are counter-clockwise (see
/// [`Pipeline::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cull {
    /// Draw both — every line and overlay, and the mesh under Backface Rendering.
    None,
    Back,
}

impl Cull {
    const fn sg(self) -> sg::CullMode {
        match self {
            Self::None => sg::CullMode::None,
            Self::Back => sg::CullMode::Back,
        }
    }
}

/// How a fragment's depth is compared with what is already there. Scene depth is
/// **Reversed-Z**, so "in front" is `GreaterEqual`, not `LessEqual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DepthCompare {
    GreaterEqual,
    /// Never rejected: the always-on-top overlays (the pivot marker, the skeleton)
    /// and the skybox, which is behind everything by construction.
    Always,
}

impl DepthCompare {
    const fn sg(self) -> sg::CompareFunc {
        match self {
            Self::GreaterEqual => sg::CompareFunc::GreaterEqual,
            Self::Always => sg::CompareFunc::Always,
        }
    }
}

/// A pipeline's depth behaviour: how it compares, and whether it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Depth {
    pub(crate) compare: DepthCompare,
    pub(crate) write: bool,
}

impl Depth {
    /// Depth-tested but never depth-writing: the line overlays, the UV island fill
    /// and the selection flash all layer onto a surface the mesh pass already wrote,
    /// and must not push each other out of the way.
    pub(crate) const TEST_ONLY: Self = Self {
        compare: DepthCompare::GreaterEqual,
        write: false,
    };
    /// Always-on-top: the comparison always passes, so the draw reads *through*
    /// solid geometry.
    pub(crate) const ALWAYS: Self = Self {
        compare: DepthCompare::Always,
        write: false,
    };
    /// The mesh's own state — the only scene pipeline that writes depth.
    pub(crate) const WRITE: Self = Self {
        compare: DepthCompare::GreaterEqual,
        write: true,
    };
    /// No depth at all, for a pass that has no depth attachment.
    pub(crate) const NONE: Self = Self {
        compare: DepthCompare::Always,
        write: false,
    };
}

/// Slope-scaled depth bias in depth-buffer units. The mesh pushes its surface back
/// so coplanar line overlays win the Reversed-Z test; everything else uses zero.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DepthBias {
    pub(crate) constant: f32,
    pub(crate) slope_scaled: f32,
}

/// How a draw's fragments combine with what is already in the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Blend {
    /// No blending: the fragment overwrites what is there. The composite, the GTAO
    /// passes, the skybox and the Tex viewport's checker.
    Opaque,
    /// Straight (non-premultiplied) alpha: `src·srcA + dst·(1−srcA)`. What an image
    /// whose stored alpha is its own coverage composites with — the Tex viewport's
    /// image over its background, and every scene draw over the MRT.
    StraightAlpha,
    /// egui's premultiplied-alpha contract: `src·1 + dst·(1−srcA)` for colour, and
    /// `src·(1−dstA) + dst·1` for alpha, which composes correctly when the chrome is
    /// drawn over an already-opaque backbuffer *and* when it overlaps itself.
    PremultipliedAlpha,
}

impl Blend {
    fn state(self) -> sg::BlendState {
        match self {
            Self::Opaque => sg::BlendState::new(),
            Self::StraightAlpha => sg::BlendState {
                enabled: true,
                src_factor_rgb: sg::BlendFactor::SrcAlpha,
                dst_factor_rgb: sg::BlendFactor::OneMinusSrcAlpha,
                op_rgb: sg::BlendOp::Add,
                src_factor_alpha: sg::BlendFactor::One,
                dst_factor_alpha: sg::BlendFactor::OneMinusSrcAlpha,
                op_alpha: sg::BlendOp::Add,
            },
            Self::PremultipliedAlpha => sg::BlendState {
                enabled: true,
                src_factor_rgb: sg::BlendFactor::One,
                dst_factor_rgb: sg::BlendFactor::OneMinusSrcAlpha,
                op_rgb: sg::BlendOp::Add,
                src_factor_alpha: sg::BlendFactor::OneMinusDstAlpha,
                dst_factor_alpha: sg::BlendFactor::One,
                op_alpha: sg::BlendOp::Add,
            },
        }
    }
}

/// What a pipeline draws with.
///
/// Built from [`Self::swapchain`] or [`Self::scene`] — which fix the target formats,
/// the depth format and the sample count together — and then overridden with `..`.
pub(crate) struct PipelineDesc<'a> {
    /// The shader, which the built pipeline takes ownership of.
    pub(crate) shader: sg::Shader,
    /// The vertex attributes, in `layout(location=…)` order, packed sequentially into
    /// one interleaved vertex buffer. Empty for a shader that builds its own vertices
    /// from `gl_VertexIndex`.
    pub(crate) attributes: &'a [VertexFormat],
    /// Whether draws are indexed with `u32` indices.
    pub(crate) indexed: bool,
    pub(crate) topology: Topology,
    pub(crate) cull: Cull,
    pub(crate) depth: Depth,
    pub(crate) depth_bias: DepthBias,
    pub(crate) blend: Blend,
    /// The colour attachments this pipeline writes, in order. Must match the pass's.
    pub(crate) colors: &'a [Format],
    /// The pass's depth format, or `None` for a pass with no depth attachment.
    pub(crate) depth_format: Option<Format>,
    /// MSAA level of the pass's attachments. 1 is single-sample.
    pub(crate) sample_count: u32,
    pub(crate) label: &'a CStr,
}

/// The one colour attachment of the swapchain pass.
const SWAPCHAIN_COLORS: &[Format] = &[Format::Swapchain];
/// The scene pass's two linear-HDR attachments: location 0 radiance, location 1
/// AO-eligible diffuse ambient.
const SCENE_COLORS: &[Format] = &[SCENE_COLOR_FORMAT, SCENE_COLOR_FORMAT];
/// The GTAO G-buffer's one attachment: view normal in `xyz`, view Z in `w`.
pub(crate) const GBUFFER_COLORS: &[Format] = &[SCENE_COLOR_FORMAT];
/// The occlusion and blur passes' one attachment.
pub(crate) const OCCLUSION_COLORS: &[Format] = &[Format::R8];

impl<'a> PipelineDesc<'a> {
    /// A pipeline drawing into the frame's swapchain pass: one backbuffer-format
    /// colour attachment, no depth (the scene renders offscreen), single-sample.
    pub(crate) fn swapchain(shader: sg::Shader, label: &'a CStr) -> Self {
        Self {
            shader,
            attributes: &[],
            indexed: false,
            topology: Topology::Triangles,
            cull: Cull::None,
            depth: Depth::NONE,
            depth_bias: DepthBias::default(),
            blend: Blend::Opaque,
            colors: SWAPCHAIN_COLORS,
            depth_format: None,
            sample_count: 1,
            label,
        }
    }

    /// A pipeline drawing into a single offscreen colour attachment with no depth —
    /// the GTAO occlusion and blur passes, which name their own format.
    pub(crate) fn offscreen(shader: sg::Shader, colors: &'a [Format], label: &'a CStr) -> Self {
        Self {
            colors,
            ..Self::swapchain(shader, label)
        }
    }

    /// A pipeline drawing into the offscreen 2-MRT linear-HDR scene pass at
    /// `sample_count` MSAA, with the Reversed-Z depth buffer. Both attachments
    /// alpha-blend.
    pub(crate) fn scene(shader: sg::Shader, sample_count: u32, label: &'a CStr) -> Self {
        Self {
            colors: SCENE_COLORS,
            depth: Depth::TEST_ONLY,
            depth_format: Some(SCENE_DEPTH_FORMAT),
            blend: Blend::StraightAlpha,
            sample_count,
            ..Self::swapchain(shader, label)
        }
    }
}

/// A built pipeline and the shader it owns.
pub(crate) struct Pipeline {
    pipeline: sg::Pipeline,
    shader: sg::Shader,
}

impl Pipeline {
    /// Build the pipeline, or fail naming it.
    ///
    /// On failure the shader is destroyed here: it was handed over by value, so
    /// there is no one left to do it, and leaking a shader per failed pipeline would
    /// hide the real error behind a pool exhaustion later.
    pub(crate) fn new(desc: &PipelineDesc<'_>) -> GpuResult<Self> {
        let mut pd = sg::PipelineDesc::new();
        pd.shader = desc.shader;
        pd.label = desc.label.as_ptr();
        let mut stride = 0;
        for (slot, &format) in desc.attributes.iter().enumerate() {
            pd.layout.attrs[slot] = sg::VertexAttrState {
                buffer_index: 0,
                offset: stride,
                format: format.sg(),
            };
            stride += format.size();
        }
        pd.layout.buffers[0].stride = stride;
        if desc.indexed {
            pd.index_type = sg::IndexType::Uint32;
        }
        pd.primitive_type = desc.topology.sg();
        pd.cull_mode = desc.cull.sg();
        pd.color_count = desc.colors.len() as i32;
        for (slot, &format) in desc.colors.iter().enumerate() {
            pd.colors[slot].pixel_format = format.sg();
            pd.colors[slot].blend = desc.blend.state();
        }
        pd.depth = sg::DepthState {
            // A pass with no depth attachment must say so here too, or sokol rejects
            // the pipeline against it.
            pixel_format: desc
                .depth_format
                .map_or(sg::PixelFormat::None, |format| format.sg()),
            compare: desc.depth.compare.sg(),
            write_enabled: desc.depth.write,
            bias: desc.depth_bias.constant,
            bias_slope_scale: desc.depth_bias.slope_scaled,
            bias_clamp: 0.0,
        };
        // Front faces are **counter-clockwise** throughout this renderer — glam's
        // `_rh` projections plus the winding every geometry builder emits. sokol's
        // default is the opposite (clockwise), and getting it wrong is not a subtle
        // shading difference: back-face culling then keeps the far side of every
        // solid, so a closed mesh renders as its own interior.
        pd.face_winding = sg::FaceWinding::Ccw;
        pd.sample_count = desc.sample_count.max(1) as i32;

        let pipeline = sg::make_pipeline(&pd);
        if let Err(err) = require_valid(
            sg::query_pipeline_state(pipeline),
            ResourceKind::Pipeline,
            desc.label.to_str().unwrap_or("pipeline"),
        ) {
            sg::destroy_pipeline(pipeline);
            sg::destroy_shader(desc.shader);
            return Err(err);
        }
        Ok(Self {
            pipeline,
            shader: desc.shader,
        })
    }

    /// Make this pipeline current for the following draws.
    pub(crate) fn apply(&self) {
        sg::apply_pipeline(self.pipeline);
    }

    /// The sokol handle, for a deferred [`super::SwapchainJob`].
    pub(in crate::rhi) fn handle(&self) -> sg::Pipeline {
        self.pipeline
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // Only while sokol_gfx is still up: `Gpu`'s own `Drop` shuts it down, and a
        // field dropped after it would be destroying handles into a dead library.
        if sg::isvalid() {
            sg::destroy_pipeline(self.pipeline);
            sg::destroy_shader(self.shader);
        }
    }
}
