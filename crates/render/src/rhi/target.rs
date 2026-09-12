//! Offscreen render targets: the linear-HDR colour attachments the scene draws into
//! and the Reversed-Z depth buffer it tests against.
//!
//! sokol keeps storage and access apart, so a target is an `sg::Image` plus *two*
//! views over it: one the pass attaches (`color_attachment` / `depth_stencil_
//! attachment`) and one a later pass samples (`texture`). A depth target has only
//! the first — nothing samples scene depth.
//!
//! **MSAA** adds a third: at 2×+ the attached image is multisampled and cannot be
//! sampled at all, so a single-sample twin carries the texture view and a
//! `resolve_attachment` view over it goes in the pass's `resolves` slot. sokol
//! resolves at `end_pass`, which is why the old explicit `ColorTarget::resolve` call
//! has no successor — and why nothing has to remember to make it.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::error::{GpuResult, ResourceKind, require_valid};
use super::format::{Format, SCENE_COLOR_FORMAT, SCENE_DEPTH_FORMAT};

/// An offscreen colour attachment: a pass draws into it, a later pass samples it.
///
/// Recreated whenever the render size changes — which is why it remembers the size
/// it was built at, rather than the caller tracking that separately.
pub(crate) struct ColorTarget {
    /// What the pass draws into: multisampled at 2×+, the single-sample image itself
    /// at 1×.
    image: sg::Image,
    attachment: sg::View,
    /// The single-sample twin a later pass samples, present only at 2×+. Its
    /// `resolve` view is what the pass hands sokol to resolve into.
    resolve: Option<Resolve>,
    /// The view a later pass samples: the resolve twin's at 2×+, `image`'s at 1×.
    texture: sg::View,
    width: u32,
    height: u32,
    sample_count: u32,
}

/// The single-sample twin of a multisampled colour target.
struct Resolve {
    image: sg::Image,
    view: sg::View,
}

impl ColorTarget {
    /// A linear-HDR target at `sample_count` MSAA — one of the scene MRT's two
    /// attachments. `1` is single-sample, with no resolve twin.
    pub(crate) fn hdr(width: u32, height: u32, sample_count: u32, label: &CStr) -> GpuResult<Self> {
        Self::new(width, height, SCENE_COLOR_FORMAT, sample_count, label)
    }

    /// A single-channel single-sample target — the GTAO occlusion buffers
    /// ([`Format::R16F`]) and its depth prefilter chain ([`Format::R32F`]).
    ///
    /// The format is a parameter rather than fixed because those two want different
    /// precision for the same *shape* of target: the occlusion is a 0..1 term that
    /// half-float carries exactly well enough, while the depth chain holds view
    /// depth in metres and needs full float (see [`Format::R32F`]).
    pub(crate) fn single_channel(
        width: u32,
        height: u32,
        format: Format,
        label: &CStr,
    ) -> GpuResult<Self> {
        debug_assert!(
            matches!(format, Format::R8 | Format::R16F | Format::R32F),
            "single_channel is for the one-channel formats only"
        );
        Self::new(width, height, format, 1, label)
    }

    fn new(
        width: u32,
        height: u32,
        format: Format,
        sample_count: u32,
        label: &CStr,
    ) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("target");
        let (width, height) = (width.max(1), height.max(1));
        let sample_count = sample_count.max(1);

        let image = make_image(
            width,
            height,
            format,
            sample_count,
            Attachment::Color,
            label,
        );
        let mut attachment_desc = sg::ViewDesc::new();
        attachment_desc.color_attachment.image = image;
        attachment_desc.label = label.as_ptr();
        let attachment = sg::make_view(&attachment_desc);

        // At 2×+ the attached image is multisampled, which cannot back a texture
        // view; the single-sample twin does, and takes the pass's resolve.
        let resolve = (sample_count > 1).then(|| {
            let resolve_image = make_image(width, height, format, 1, Attachment::Resolve, label);
            let mut desc = sg::ViewDesc::new();
            desc.resolve_attachment.image = resolve_image;
            desc.label = label.as_ptr();
            Resolve {
                image: resolve_image,
                view: sg::make_view(&desc),
            }
        });

        let mut texture_desc = sg::ViewDesc::new();
        texture_desc.texture.image = resolve.as_ref().map_or(image, |twin| twin.image);
        texture_desc.label = label.as_ptr();
        let texture = sg::make_view(&texture_desc);

        let target = Self {
            image,
            attachment,
            resolve,
            texture,
            width,
            height,
            sample_count,
        };
        // Built into `self` first, so a failure below still frees everything through
        // one `Drop` rather than a hand-written unwind per resource.
        target.validate(name)?;
        Ok(target)
    }

    fn validate(&self, name: &str) -> GpuResult<()> {
        require_valid(
            sg::query_image_state(self.image),
            ResourceKind::Target,
            name,
        )?;
        require_valid(
            sg::query_view_state(self.attachment),
            ResourceKind::Target,
            name,
        )?;
        if let Some(resolve) = &self.resolve {
            require_valid(
                sg::query_image_state(resolve.image),
                ResourceKind::Target,
                name,
            )?;
            require_valid(
                sg::query_view_state(resolve.view),
                ResourceKind::Target,
                name,
            )?;
        }
        require_valid(
            sg::query_view_state(self.texture),
            ResourceKind::Target,
            name,
        )
    }

    /// The size it was built at, so a resize can be detected without tracking it
    /// alongside.
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The MSAA level it was built at, so a change of the AA setting can be detected
    /// the same way.
    pub(crate) fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// The view a pass attaches.
    pub(in crate::rhi) fn attachment(&self) -> sg::View {
        self.attachment
    }

    /// The view the pass resolves into, or `None` when single-sample.
    pub(in crate::rhi) fn resolve(&self) -> Option<sg::View> {
        self.resolve.as_ref().map(|twin| twin.view)
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
            if let Some(resolve) = &self.resolve {
                sg::destroy_view(resolve.view);
                sg::destroy_image(resolve.image);
            }
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
    /// A depth target at `sample_count` MSAA — it has to match the colour
    /// attachments it is attached beside.
    pub(crate) fn new(width: u32, height: u32, sample_count: u32, label: &CStr) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("depth");
        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.depth_stencil_attachment = true;
        desc.width = width.max(1) as i32;
        desc.height = height.max(1) as i32;
        desc.num_mipmaps = 1;
        desc.sample_count = sample_count.max(1) as i32;
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

/// Which end of a resolve an image is. sokol takes these as *exclusive* usages: the
/// multisampled image a pass draws into is a colour attachment, and the
/// single-sample image it resolves into is a resolve attachment — never both.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Attachment {
    Color,
    Resolve,
}

/// An attachment image at one sample count.
fn make_image(
    width: u32,
    height: u32,
    format: Format,
    sample_count: u32,
    attachment: Attachment,
    label: &CStr,
) -> sg::Image {
    let mut desc = sg::ImageDesc::new();
    desc._type = sg::ImageType::Dim2;
    match attachment {
        Attachment::Color => desc.usage.color_attachment = true,
        Attachment::Resolve => desc.usage.resolve_attachment = true,
    }
    desc.width = width as i32;
    desc.height = height as i32;
    desc.num_mipmaps = 1;
    desc.sample_count = sample_count as i32;
    desc.pixel_format = format.sg();
    desc.label = label.as_ptr();
    sg::make_image(&desc)
}

impl Drop for DepthTarget {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::destroy_view(self.attachment);
            sg::destroy_image(self.image);
        }
    }
}
