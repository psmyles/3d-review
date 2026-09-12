//! The pixel formats the renderer uses, named once so no caller outside
//! [`crate::rhi`] has to spell a backend enum.
//!
//! This is deliberately a *closed* list, not a mapping of everything the backend
//! offers: it is exactly the nine formats the scene, GTAO, IBL and Tex paths draw
//! into or sample from. Adding a format means adding a variant here and its
//! translation below — which is the point, since that translation is the only place
//! a format ever has to be named twice.
//!
//! The translation target is `sg::PixelFormat` now rather than `DXGI_FORMAT`, so
//! this file is one of the many that stopped being platform code when the renderer
//! moved onto sokol_gfx. The **swapchain** format is the exception and lives in the
//! backend leaf: Metal's `CAMetalLayer` refuses RGBA8 and wants BGRA8, so the two
//! OSes genuinely cannot agree on it (D20).
//!
//! Vertex-attribute formats are *not* here; they are their own closed enum in
//! [`super::pipeline::VertexFormat`], because they are a different question (how a
//! byte range in a vertex buffer is read) with a different answer per backend.

use sokol::gfx as sg;

/// A texture / render-target pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 8-bit RGBA, gamma space, sampled and written **without** hardware sRGB
    /// conversion. The egui atlas and the Tex viewport's images.
    Rgba8,
    /// 8-bit RGBA that the hardware converts to/from linear on every access — the
    /// material colour maps, whose source art is authored in sRGB.
    Rgba8Srgb,
    /// Half-float RGBA. The linear-HDR scene MRT, and the raw IBL cubes at bake time.
    Rgba16F,
    /// Half-float RG. The shared split-sum BRDF lookup table.
    Rg16F,
    /// Single-channel 8-bit unorm. Nothing draws into it any more — the GTAO
    /// occlusion moved to [`Self::R16F`] because 1/255 quantization banded a term
    /// that is a smooth gradient over most of a surface — but it stays the named
    /// single-channel LDR format.
    R8,
    /// Single-channel half-float. The GTAO occlusion buffers: the accumulated
    /// history and the denoiser ping-pong. Half is plenty for a 0..1 visibility
    /// term and costs half the bandwidth of `R32F` on the passes that read it most.
    R16F,
    /// Single-channel float. The GTAO depth prefilter chain, which stores *linear
    /// view depth in metres* rather than a 0..1 term: at 10 m a half-float's steps
    /// are ~8 mm, coarse enough to move a horizon test on a contact crease, so the
    /// chain is full float even though the AO it feeds is not.
    R32F,
    /// BC6H unsigned-float block compression — the three shipped HDR cubes.
    /// GPU-native, so sampling costs nothing and there is no decode step; core in
    /// Direct3D 11 feature level 11_0 and on Apple silicon, the renderer's floors.
    Bc6hUf16,
    /// 32-bit float depth. Scene depth, cleared to 0 and tested `GreaterEqual`
    /// (Reversed-Z).
    Depth32F,
    /// Whatever the window's backbuffer is — `R8G8B8A8_UNORM` on Direct3D 11,
    /// `BGRA8Unorm` on Metal, which is why it lives in the backend leaf and not in
    /// the list above (D20). Only meaningful as a *pipeline's* colour-target format:
    /// nothing creates a texture in it.
    Swapchain,
}

impl Format {
    /// The sokol_gfx format this maps to.
    ///
    /// `pub(in crate::rhi)` on purpose: this is the one function that names the
    /// drawing API's own enum, and the visibility is what stops an `sg::PixelFormat`
    /// leaking back out into the scene or material code the way `DXGI_FORMAT` used
    /// to.
    pub(in crate::rhi) const fn sg(self) -> sg::PixelFormat {
        match self {
            Self::Rgba8 => sg::PixelFormat::Rgba8,
            Self::Rgba8Srgb => sg::PixelFormat::Srgb8a8,
            Self::Rgba16F => sg::PixelFormat::Rgba16f,
            Self::Rg16F => sg::PixelFormat::Rg16f,
            Self::R8 => sg::PixelFormat::R8,
            Self::R16F => sg::PixelFormat::R16f,
            Self::R32F => sg::PixelFormat::R32f,
            Self::Bc6hUf16 => sg::PixelFormat::Bc6hRgbuf,
            Self::Depth32F => sg::PixelFormat::Depth,
            Self::Swapchain => super::backend::SWAPCHAIN_FORMAT,
        }
    }

    /// Whether the format is block-compressed (4×4 texels per block), which is what
    /// decides whether a mip's row pitch is counted in texels or in blocks.
    pub(in crate::rhi) const fn is_block_compressed(self) -> bool {
        matches!(self, Self::Bc6hUf16)
    }

    /// Bytes per texel, or per 4×4 block for a block-compressed format.
    pub(in crate::rhi) const fn block_bytes(self) -> u32 {
        match self {
            Self::R8 => 1,
            Self::R16F => 2,
            Self::Rg16F
            | Self::R32F
            | Self::Rgba8
            | Self::Rgba8Srgb
            | Self::Depth32F
            | Self::Swapchain => 4,
            Self::Rgba16F => 8,
            // BC6H: one 16-byte block per 4×4 texels.
            Self::Bc6hUf16 => 16,
        }
    }
}

/// The linear-HDR format both scene colour attachments use.
pub const SCENE_COLOR_FORMAT: Format = Format::Rgba16F;

/// The scene depth format (Reversed-Z).
pub const SCENE_DEPTH_FORMAT: Format = Format::Depth32F;
