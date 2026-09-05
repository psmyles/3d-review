//! Offscreen render targets: the linear-HDR colour attachments the scene draws into
//! and the Reversed-Z depth buffer it tests against.
//!
//! sokol keeps storage and access apart, so a target is an `sg::Image` plus *two*
//! views over it: one the pass attaches (`color_attachment` / `depth_stencil_
//! attachment`) and one a later pass samples (`texture`). A depth target has only
//! the first — nothing samples scene depth.
//!
//! Single-sample for now. MSAA adds a multisampled image whose resolve view points
//! at the single-sample twin, and arrives with the AA stage that needs it
//! (`mac-port-plan.md` Phase 1 step 4).

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};
use super::format::{Format, SCENE_COLOR_FORMAT, SCENE_DEPTH_FORMAT};

/// An offscreen colour attachment: a pass draws into it, a later pass samples it.
///
/// Recreated whenever the render size changes — which is why it remembers the size
/// it was built at, rather than the caller tracking that separately.
pub(crate) struct ColorTarget {
    image: sg::Image,
    attachment: sg::View,
    texture: sg::View,
    width: u32,
    height: u32,
}

impl ColorTarget {
    /// A linear-HDR target — one of the scene MRT's two attachments, or the GTAO
    /// view-normal/Z G-buffer.
    pub(crate) fn hdr(width: u32, height: u32, label: &CStr) -> GpuResult<Self> {
        Self::new(width, height, SCENE_COLOR_FORMAT, label)
    }

    fn new(width: u32, height: u32, format: Format, label: &CStr) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("target");
        let (width, height) = (width.max(1), height.max(1));

        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.color_attachment = true;
        desc.width = width as i32;
        desc.height = height as i32;
        desc.num_mipmaps = 1;
        desc.pixel_format = format.sg();
        desc.label = label.as_ptr();
        let image = sg::make_image(&desc);

        let mut attachment_desc = sg::ViewDesc::new();
        attachment_desc.color_attachment.image = image;
        attachment_desc.label = label.as_ptr();
        let attachment = sg::make_view(&attachment_desc);

        let mut texture_desc = sg::ViewDesc::new();
        texture_desc.texture.image = image;
        texture_desc.label = label.as_ptr();
        let texture = sg::make_view(&texture_desc);

        let made = require_valid(sg::query_image_state(image), ResourceKind::Target, name)
            .and_then(|()| {
                require_valid(sg::query_view_state(attachment), ResourceKind::Target, name)
            })
            .and_then(|()| {
                require_valid(sg::query_view_state(texture), ResourceKind::Target, name)
            });
        if let Err(err) = made {
            sg::destroy_view(texture);
            sg::destroy_view(attachment);
            sg::destroy_image(image);
            return Err(err);
        }
        Ok(Self {
            image,
            attachment,
            texture,
            width,
            height,
        })
    }

    /// The size it was built at, so a resize can be detected without tracking it
    /// alongside.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The view a pass attaches.
    pub(in crate::rhi) fn attachment(&self) -> sg::View {
        self.attachment
    }

    /// The view a later pass samples it through.
    pub(in crate::rhi) fn texture(&self) -> sg::View {
        self.texture
    }
}

impl Drop for ColorTarget {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_view(self.texture);
            sg::destroy_view(self.attachment);
            sg::destroy_image(self.image);
        }
    }
}

/// The scene depth buffer ([`Format::Depth32F`], Reversed-Z: cleared to 0 and tested
/// `GreaterEqual`). Attachment-only — nothing samples it.
pub(crate) struct DepthTarget {
    image: sg::Image,
    attachment: sg::View,
}

impl DepthTarget {
    pub(crate) fn new(width: u32, height: u32, label: &CStr) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("depth");
        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.depth_stencil_attachment = true;
        desc.width = width.max(1) as i32;
        desc.height = height.max(1) as i32;
        desc.num_mipmaps = 1;
        desc.pixel_format = SCENE_DEPTH_FORMAT.sg();
        desc.label = label.as_ptr();
        let image = sg::make_image(&desc);

        let mut attachment_desc = sg::ViewDesc::new();
        attachment_desc.depth_stencil_attachment.image = image;
        attachment_desc.label = label.as_ptr();
        let attachment = sg::make_view(&attachment_desc);

        let made = require_valid(sg::query_image_state(image), ResourceKind::Target, name)
            .and_then(|()| {
                require_valid(sg::query_view_state(attachment), ResourceKind::Target, name)
            });
        if let Err(err) = made {
            sg::destroy_view(attachment);
            sg::destroy_image(image);
            return Err(err);
        }
        Ok(Self { image, attachment })
    }

    pub(in crate::rhi) fn attachment(&self) -> sg::View {
        self.attachment
    }
}

impl Drop for DepthTarget {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_view(self.attachment);
            sg::destroy_image(self.image);
        }
    }
}
