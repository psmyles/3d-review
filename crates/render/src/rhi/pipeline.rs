//! Pipelines: a shader plus the fixed-function state it draws with.
//!
//! One [`Pipeline`] owns its shader, because nothing in this renderer shares a shader
//! across pipelines with *different* state without also sharing the pipeline — the
//! scene's variants each carry their own — and one owner means one `Drop`.
//!
//! The knobs here are exactly the ones something built so far actually sets. The
//! renderer is being ported onto sokol_gfx one stage at a time (`mac-port-plan.md`
//! Phase 1 step 4), and a knob nobody sets is a knob nobody has checked: depth state,
//! culling, line topology, MSAA sample counts and offscreen colour formats arrive
//! with the scene stages that need them. Growing it that way is deliberate — the
//! alternative is a wrapper full of untested surface.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};

/// How one vertex attribute's bytes are read. Sequential in the vertex, so the
/// stride and each offset follow from the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VertexFormat {
    /// Two `f32` — a position or a UV.
    Float2,
    /// Four `u8` normalised to 0..1 — a packed colour.
    Ubyte4N,
}

impl VertexFormat {
    /// Bytes this attribute occupies in the vertex.
    const fn size(self) -> i32 {
        match self {
            Self::Float2 => 8,
            Self::Ubyte4N => 4,
        }
    }

    const fn sg(self) -> sg::VertexFormat {
        match self {
            Self::Float2 => sg::VertexFormat::Float2,
            Self::Ubyte4N => sg::VertexFormat::Ubyte4n,
        }
    }
}

/// How a draw's fragments combine with what is already in the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Blend {
    /// No blending: the fragment overwrites what is there. The Tex viewport's
    /// checker, which is a background.
    Opaque,
    /// Straight (non-premultiplied) alpha: `src·srcA + dst·(1−srcA)`. What an image
    /// whose stored alpha is its own coverage composites with — the Tex viewport's
    /// image over its background.
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
/// The colour target is the swapchain's and the sample count is 1, because the
/// swapchain pass is the only pass that exists so far; both become fields when the
/// offscreen scene targets land.
pub(crate) struct PipelineDesc<'a> {
    /// The shader, which the built pipeline takes ownership of.
    pub(crate) shader: sg::Shader,
    /// The vertex attributes, in `layout(location=…)` order, packed sequentially into
    /// one interleaved vertex buffer.
    pub(crate) attributes: &'a [VertexFormat],
    /// Whether draws are indexed with `u32` indices.
    pub(crate) indexed: bool,
    pub(crate) blend: Blend,
    pub(crate) label: &'a CStr,
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
        pd.color_count = 1;
        pd.colors[0].pixel_format = super::backend::SWAPCHAIN_FORMAT;
        pd.colors[0].blend = desc.blend.state();
        pd.sample_count = 1;

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
