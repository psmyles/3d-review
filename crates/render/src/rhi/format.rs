//! The pixel formats the renderer uses, named once so no caller outside
//! [`crate::rhi`] has to spell a backend enum.
//!
//! This is deliberately a *closed* list, not a mapping of everything the backend
//! offers: it is exactly the seven formats the scene, GTAO, IBL and Tex paths draw
//! into or sample from. Adding a format means adding a variant here and its backend
//! translation below — which is the point, since that translation is the only place
//! a format ever has to be named twice.
//!
//! Vertex-attribute formats are *not* here; they are their own closed enum in
//! [`super::pipeline::VertexFormat`], because they are a different question (how a
//! byte range in a vertex buffer is read) with a different answer per backend.

use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_BC6H_UF16, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R16G16_FLOAT,
    DXGI_FORMAT_R16G16B16A16_FLOAT,
};

/// A texture / render-target pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 8-bit RGBA, gamma space, sampled and written **without** hardware sRGB
    /// conversion. The swapchain backbuffer and the Tex viewport's images.
    Rgba8,
    /// 8-bit RGBA that the hardware converts to/from linear on every access — the
    /// material colour maps, whose source art is authored in sRGB.
    Rgba8Srgb,
    /// Half-float RGBA. The linear-HDR scene MRT, and the raw IBL cubes at bake time.
    Rgba16F,
    /// Half-float RG. The shared split-sum BRDF lookup table.
    Rg16F,
    /// Single-channel 8-bit unorm. The GTAO occlusion buffer, raw and blurred.
    R8,
    /// BC6H unsigned-float block compression — the three shipped HDR cubes.
    /// GPU-native, so sampling costs nothing and there is no decode step; core in
    /// Direct3D 11 feature level 11_0, the renderer's floor.
    Bc6hUf16,
    /// 32-bit float depth. Scene depth, cleared to 0 and tested `GreaterEqual`
    /// (Reversed-Z).
    Depth32F,
}

impl Format {
    /// The Direct3D format this maps to.
    ///
    /// `pub(in crate::rhi)` on purpose: this is the one function that names a
    /// backend enum, and the visibility is what stops a `DXGI_FORMAT` from leaking
    /// back out into the scene or material code the way it used to.
    pub(in crate::rhi) const fn dxgi(self) -> DXGI_FORMAT {
        match self {
            Self::Rgba8 => DXGI_FORMAT_R8G8B8A8_UNORM,
            Self::Rgba8Srgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            Self::Rgba16F => DXGI_FORMAT_R16G16B16A16_FLOAT,
            Self::Rg16F => DXGI_FORMAT_R16G16_FLOAT,
            Self::R8 => DXGI_FORMAT_R8_UNORM,
            Self::Bc6hUf16 => DXGI_FORMAT_BC6H_UF16,
            Self::Depth32F => DXGI_FORMAT_D32_FLOAT,
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
            Self::Rg16F | Self::Rgba8 | Self::Rgba8Srgb | Self::Depth32F => 4,
            Self::Rgba16F => 8,
            // BC6H: one 16-byte block per 4×4 texels.
            Self::Bc6hUf16 => 16,
        }
    }
}

/// The swapchain backbuffer format.
///
/// Plain UNORM, **not** `_SRGB`: egui blends in gamma space and our own scene
/// composite already encodes sRGB in the post pass, so a hardware conversion on top
/// would double-encode. It lives beside the backend for the port — Metal's
/// `CAMetalLayer` refuses RGBA8 and wants BGRA8, so this constant is one of the few
/// things that genuinely differs per OS.
pub const SWAPCHAIN_FORMAT: Format = Format::Rgba8;

/// The linear-HDR format both scene colour attachments use.
pub const SCENE_COLOR_FORMAT: Format = Format::Rgba16F;

/// The scene depth format (Reversed-Z).
pub const SCENE_DEPTH_FORMAT: Format = Format::Depth32F;
