//! Textures: an image plus the view a shader samples it through.
//!
//! sokol separates the two — an `sg::Image` is storage, an `sg::View` is how a
//! binding slot reads it — so a [`Texture`] owns a matched pair and hands out only
//! the view. The baked IBL maps' BC6H cubes are built here too
//! ([`Texture::cube_block_compressed`]); a render target, which also needs the views
//! a pass attaches, is a `target::ColorTarget` / `target::DepthTarget` instead.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuError, GpuResult, ResourceKind};
use super::format::Format;
use super::make;
use super::mips;

// `mips` stays free of sokol so the build script can compile it; this is where its
// level cap is held to the one sokol actually has.
const _: () = assert!(
    mips::MAX_LEVELS as usize == sg::MAX_MIPMAPS,
    "mips::MAX_LEVELS no longer matches sokol's SG_MAX_MIPMAPS"
);

/// An immutable 2D texture and its sampling view.
pub(crate) struct Texture {
    image: sg::Image,
    view: sg::View,
}

impl Texture {
    /// Upload a single-mip 2D texture from tightly packed rows.
    ///
    /// Immutable: the content is handed over at creation and never updated. That is
    /// deliberate for the egui atlas — sokol's `sg_update_image` is whole-image and
    /// once-per-frame, so a sub-rectangle patch is expressed as "recreate from the
    /// CPU shadow", which an immutable image does with no extra machinery.
    pub(crate) fn immutable_2d(
        pixels: &[u8],
        width: u32,
        height: u32,
        format: Format,
        label: &CStr,
    ) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("texture");
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|texels| texels.checked_mul(format.block_bytes() as usize))
            .ok_or_else(|| {
                GpuError::invalid_arg(format!("'{name}' is {width}x{height}, which overflows"))
            })?;
        if format.is_block_compressed() {
            return Err(GpuError::invalid_arg(format!(
                "'{name}' asks for a block-compressed upload from texel rows; \
                 a block-compressed 2D texture goes through `block_compressed_2d`"
            )));
        }
        if pixels.len() != expected {
            return Err(GpuError::invalid_arg(format!(
                "'{name}' is {width}x{height} ({expected} bytes) but was given {}",
                pixels.len()
            )));
        }

        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.immutable = true;
        desc.width = width as i32;
        desc.height = height as i32;
        desc.num_mipmaps = 1;
        desc.pixel_format = format.sg();
        desc.data.mip_levels[0] = sg::slice_as_range(pixels);
        desc.label = label.as_ptr();
        Self::from_desc(&desc, label)
    }

    /// Upload a decoded RGBA8 image (sRGB or raw) with a full CPU-generated mip
    /// chain — the Tex viewport's images and, when that stage lands, the material
    /// slots. A degenerate or short pixel buffer falls back to a 1×1 white texel: a
    /// decode hiccup must never fail the render path.
    ///
    /// The chain is built here rather than on the decode worker D7 names, because it
    /// is not a property of the decoded image alone — the same pixels are uploaded
    /// raw by the Tex viewport and sRGB by a material slot, and the two chains differ
    /// (see [`mips`]). It costs a few milliseconds once per texture, on the frame it
    /// is first looked at.
    pub(crate) fn rgba8_mipped_or_white(
        width: u32,
        height: u32,
        rgba: &[u8],
        srgb: bool,
        label: &CStr,
    ) -> GpuResult<Self> {
        let format = if srgb {
            Format::Rgba8Srgb
        } else {
            Format::Rgba8
        };
        let (width, height) = (width.max(1), height.max(1));
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|texels| texels.checked_mul(4));
        if expected.is_none_or(|expected| rgba.len() < expected) {
            return Self::immutable_2d(&[255, 255, 255, 255], 1, 1, format, label);
        }

        let levels = mips::levels_below_base(rgba, width, height, srgb);
        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.immutable = true;
        desc.width = width as i32;
        desc.height = height as i32;
        desc.num_mipmaps = levels.len() as i32 + 1;
        desc.pixel_format = format.sg();
        desc.data.mip_levels[0] = sg::slice_as_range(rgba);
        for (level, pixels) in levels.iter().enumerate() {
            desc.data.mip_levels[level + 1] = sg::slice_as_range(pixels);
        }
        desc.label = label.as_ptr();
        // `levels` is alive across the call, which is what the ranges above point at.
        Self::from_desc(&desc, label)
    }

    /// Upload an immutable block-compressed 2D texture with `levels` mips from
    /// level-major block bytes, largest first — the UV checker's chains, as the build
    /// script wrote them. Each level is its own block grid, rounded up to whole 4×4
    /// blocks, so the 2×2 and 1×1 levels still take one block apiece.
    ///
    /// `width` and `height` must be whole blocks: Direct3D 11 refuses a
    /// block-compressed texture whose top level is not, and so this does too, before
    /// sokol is handed anything.
    pub(crate) fn block_compressed_2d(
        width: u32,
        height: u32,
        levels: u32,
        format: Format,
        data: &[u8],
        label: &CStr,
    ) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("texture");
        debug_assert!(
            format.is_block_compressed(),
            "block_compressed_2d needs a block-compressed format"
        );
        if width == 0 || height == 0 || !width.is_multiple_of(4) || !height.is_multiple_of(4) {
            return Err(GpuError::invalid_arg(format!(
                "'{name}' is {width}x{height}, which is not a whole number of 4x4 blocks"
            )));
        }
        if levels == 0 || levels > mips::level_count(width, height) {
            return Err(GpuError::invalid_arg(format!(
                "'{name}' asks for {levels} levels, which a {width}x{height} image cannot have"
            )));
        }
        let block_bytes = format.block_bytes() as usize;
        let level_len = |level: u32| -> usize {
            let blocks_wide = (width >> level).max(1).div_ceil(4) as usize;
            let blocks_high = (height >> level).max(1).div_ceil(4) as usize;
            blocks_wide * blocks_high * block_bytes
        };
        // The whole payload, checked before any range into it is made.
        let total: usize = (0..levels).map(level_len).sum();
        if data.len() != total {
            return Err(GpuError::invalid_arg(format!(
                "'{name}' is {} bytes but its {levels}-level layout needs {total}",
                data.len()
            )));
        }

        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.immutable = true;
        desc.width = width as i32;
        desc.height = height as i32;
        desc.num_mipmaps = levels as i32;
        desc.pixel_format = format.sg();
        desc.label = label.as_ptr();
        let mut offset = 0usize;
        for level in 0..levels {
            let len = level_len(level);
            desc.data.mip_levels[level as usize] = sg::slice_as_range(&data[offset..offset + len]);
            offset += len;
        }
        Self::from_desc(&desc, label)
    }

    /// Upload an immutable block-compressed **cube** texture from mip-major,
    /// faces-contiguous block bytes — exactly the layout the IBL bake wrote.
    ///
    /// That layout is also exactly sokol's: one `mip_levels` range per mip, covering
    /// all six faces in +X -X +Y -Y +Z -Z order. The D3D11 path had to re-index this
    /// into `face * mips + mip` subresources; that step is gone.
    ///
    /// `size` is the base face resolution; the block size comes from `format`, so a
    /// caller cannot disagree with it.
    pub(crate) fn cube_block_compressed(
        size: u32,
        mips: u32,
        format: Format,
        data: &[u8],
        label: &CStr,
    ) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("cube");
        debug_assert!(
            format.is_block_compressed(),
            "cube_block_compressed needs a block-compressed format"
        );
        let block_bytes = format.block_bytes() as usize;
        // Validate the payload against the layout *before* handing sokol pointers
        // into it: a truncated baked asset would otherwise be read past its end.
        let mip_len = |mip: u32| -> usize {
            let blocks = (size >> mip).max(1).div_ceil(4) as usize;
            blocks * blocks * block_bytes * 6
        };
        let total: usize = (0..mips).map(mip_len).sum();
        if data.len() < total {
            return Err(GpuError::invalid_arg(format!(
                "'{name}' is {} bytes but its {mips}-mip layout needs {total}",
                data.len()
            )));
        }

        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Cube;
        desc.usage.immutable = true;
        desc.width = size as i32;
        desc.height = size as i32;
        desc.num_mipmaps = mips as i32;
        desc.pixel_format = format.sg();
        desc.label = label.as_ptr();
        let mut offset = 0usize;
        for mip in 0..mips {
            let len = mip_len(mip);
            desc.data.mip_levels[mip as usize] = sg::slice_as_range(&data[offset..offset + len]);
            offset += len;
        }
        Self::from_desc(&desc, label)
    }

    /// Create the image and the view a binding slot reads it through, destroying
    /// both if either came back invalid.
    fn from_desc(desc: &sg::ImageDesc, label: &CStr) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("texture");
        let image = make::image(desc, name)?;

        let mut view_desc = sg::ViewDesc::new();
        view_desc.texture.image = image;
        view_desc.label = desc.label;
        // The image is this constructor's own, so a failed view has to take it
        // with it.
        let view = match make::view(&view_desc, ResourceKind::Texture, name) {
            Ok(view) => view,
            Err(error) => {
                sg::destroy_image(image);
                return Err(error);
            }
        };
        Ok(Self { image, view })
    }

    /// The sampling view, for an `sg::Bindings` slot.
    pub(crate) fn view(&self) -> sg::View {
        self.view
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_view(self.view);
            sg::destroy_image(self.image);
        }
    }
}
