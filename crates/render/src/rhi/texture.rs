//! Textures: an image plus the view a shader samples it through.
//!
//! sokol separates the two — an `sg::Image` is storage, an `sg::View` is how a
//! binding slot reads it — so a [`Texture`] owns a matched pair and hands out only
//! the view. Mipped uploads, the BC6H cubes and the render-target views arrive with
//! the scene stages that need them (`mac-port-plan.md` Phase 1 step 4); the CPU mip
//! chain (D7) lands with the first of those.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuError, GpuResult, ResourceKind, require_valid};
use super::format::Format;

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
                "'{name}' asks for a block-compressed 2D upload, which has no caller yet"
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
        let image = sg::make_image(&desc);

        let mut view_desc = sg::ViewDesc::new();
        view_desc.texture.image = image;
        view_desc.label = label.as_ptr();
        let view = sg::make_view(&view_desc);

        let made = require_valid(sg::query_image_state(image), ResourceKind::Texture, name)
            .and_then(|()| require_valid(sg::query_view_state(view), ResourceKind::Texture, name));
        if let Err(err) = made {
            sg::destroy_view(view);
            sg::destroy_image(image);
            return Err(err);
        }
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
