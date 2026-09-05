//! Samplers: how a shader filters and wraps the texture it reads.
//!
//! Anisotropy, the mixed min-linear/mag-point sampler and the mip-LOD clamps arrive
//! with the material and Tex stages that need them (`mac-port-plan.md` Phase 1
//! step 4). What is here is what egui asks for, which is every combination of two
//! filters and two wrap modes — the chrome sets them per texture, and picking the
//! wrong one shows as a blurred icon or a bled atlas edge.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};

/// How a texture is filtered when it is not sampled 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Filter {
    /// Bilinear — text, images, anything continuous.
    Linear,
    /// Point — pixel art, and any texture that must not be softened.
    Nearest,
}

impl Filter {
    const fn sg(self) -> sg::Filter {
        match self {
            Self::Linear => sg::Filter::Linear,
            Self::Nearest => sg::Filter::Nearest,
        }
    }
}

/// What a sample outside 0..1 reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Wrap {
    /// Repeat the texture.
    Repeat,
    /// Hold the edge texel.
    ClampToEdge,
    /// Repeat, flipping every other copy.
    MirroredRepeat,
}

impl Wrap {
    const fn sg(self) -> sg::Wrap {
        match self {
            Self::Repeat => sg::Wrap::Repeat,
            Self::ClampToEdge => sg::Wrap::ClampToEdge,
            Self::MirroredRepeat => sg::Wrap::MirroredRepeat,
        }
    }
}

/// A sampler state object.
pub(crate) struct Sampler(sg::Sampler);

impl Sampler {
    /// A sampler with separate minify and magnify filters and one wrap mode on both
    /// axes. The mipmap filter follows the minify filter, which is what a caller
    /// asking for "linear minification" always means.
    pub(crate) fn new(
        min_filter: Filter,
        mag_filter: Filter,
        wrap: Wrap,
        label: &CStr,
    ) -> GpuResult<Self> {
        let mut desc = sg::SamplerDesc::new();
        desc.min_filter = min_filter.sg();
        desc.mag_filter = mag_filter.sg();
        desc.mipmap_filter = min_filter.sg();
        desc.wrap_u = wrap.sg();
        desc.wrap_v = wrap.sg();
        desc.wrap_w = wrap.sg();
        desc.label = label.as_ptr();
        let sampler = sg::make_sampler(&desc);
        require_valid(
            sg::query_sampler_state(sampler),
            ResourceKind::Sampler,
            label.to_str().unwrap_or("sampler"),
        )?;
        Ok(Self(sampler))
    }

    /// The sokol handle, for an `sg::Bindings` slot.
    pub(crate) fn handle(&self) -> sg::Sampler {
        self.0
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_sampler(self.0);
        }
    }
}
