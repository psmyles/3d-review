//! Samplers: how a shader filters and wraps the texture it reads.
//!
//! [`Sampler::new`] is the general one — egui sets a filter pair and a wrap mode per
//! texture, and picking the wrong one shows as a blurred icon or a bled atlas edge.
//! The scene's four are named constructors below, because each is a decision with a
//! reason rather than a combination.

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

/// Anisotropic sample count for the material sampler — 16x is the common hardware
/// ceiling.
const MATERIAL_ANISOTROPY: u32 = 16;

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
        Self::build(min_filter, mag_filter, wrap, 1, label)
    }

    fn build(
        min_filter: Filter,
        mag_filter: Filter,
        wrap: Wrap,
        max_anisotropy: u32,
        label: &CStr,
    ) -> GpuResult<Self> {
        let mut desc = sg::SamplerDesc::new();
        desc.max_anisotropy = max_anisotropy;
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

    /// Trilinear, clamp — the composite's 1:1 input sampler and the IBL sampler.
    /// Addressing is moot at 1:1, but clamp avoids edge wrap on the cube faces.
    pub(crate) fn linear_clamp() -> GpuResult<Self> {
        Self::new(
            Filter::Linear,
            Filter::Linear,
            Wrap::ClampToEdge,
            c"linear-clamp",
        )
    }

    /// Trilinear, repeat — the UV checker, whose tiled UVs run past 0..1.
    pub(crate) fn linear_repeat() -> GpuResult<Self> {
        Self::new(
            Filter::Linear,
            Filter::Linear,
            Wrap::Repeat,
            c"linear-repeat",
        )
    }

    /// Point, clamp — the GTAO passes. View normals and depths must **not** be
    /// blended across geometry edges (that would bleed occlusion); clamp keeps border
    /// samples from wrapping.
    pub(crate) fn point_clamp() -> GpuResult<Self> {
        Self::new(
            Filter::Nearest,
            Filter::Nearest,
            Wrap::ClampToEdge,
            c"point-clamp",
        )
    }

    /// Anisotropic, repeat — the material sampler. With the per-texture mip chain
    /// this is what removes grazing-angle shimmer; 16× is the common hardware
    /// ceiling.
    pub(crate) fn aniso_repeat() -> GpuResult<Self> {
        Self::build(
            Filter::Linear,
            Filter::Linear,
            Wrap::Repeat,
            MATERIAL_ANISOTROPY,
            c"aniso-repeat",
        )
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
