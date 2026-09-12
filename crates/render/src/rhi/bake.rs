//! Offline IBL bake plumbing — only compiled with the `bake` feature
//! (`mac-port-plan.md` D19).
//!
//! A headless sokol_gfx: a device with no window and no swapchain, plus the render
//! targets the IBL precompute draws into and reads back. A render-target **cube**
//! (`Rgba16Float`, six faces × mips, one attachment view per face/mip and one cube
//! texture view the convolution passes sample) and a plain 2D target (the BRDF LUT).
//!
//! Almost all of what the Direct3D 11 version of this file did is gone, because
//! sokol does it: no `OMSetRenderTargets`, no RTVs, no SRV slots, and — the one that
//! mattered — no manual unbinding. The old code had to release the env cube's render
//! target before sampling it, because D3D11 resolves an RTV/SRV conflict by silently
//! nulling the SRV; forgetting once baked six flat black cubes over the top of good
//! ones. A sokol pass is closed by `sg_end_pass`, so the hazard has no way to occur.
//! What is left that sokol cannot do is readback, which is a backend leaf.
//!
//! The runtime renderer never touches this module. It loads the committed baked maps
//! through [`crate::ibl::IblMaps::from_baked`], a pure upload.

use std::ffi::CStr;

use sokol::gfx as sg;

use super::backend;
use super::error::{GpuError, GpuResult, ResourceKind, require_valid};
use super::format::Format;

/// One bake pass's destination: a cube face at a mip, or the LUT target.
///
/// A newtype rather than the `sg::View` itself, so `ibl.rs` can name a destination
/// without naming a sokol type — the same boundary [`super::Format`] keeps for pixel
/// formats, and what stops the backend leaking back out through the bake.
#[derive(Clone, Copy)]
pub(crate) struct BakeAttachment(sg::View);

/// Build one of the bake's fullscreen pipelines: a single colour target, no depth,
/// no vertex input.
///
/// It lives here rather than at the call site so `ibl.rs` never has to name
/// `sg::Backend` or `sg::ShaderDesc` to pass a generated `*_shader_desc` along.
pub(crate) fn pipeline(
    desc_fn: fn(sg::Backend) -> sg::ShaderDesc,
    bytecode: &super::shader::ShaderBytecode,
    format: Format,
    label: &'static CStr,
) -> GpuResult<super::Pipeline> {
    let shader = super::shader::make(desc_fn, bytecode, label)?;
    super::Pipeline::new(&super::PipelineDesc::offscreen(shader, &[format], label))
}

/// Headless sokol_gfx for the bake: a device with no window, and `sg_setup` on it.
///
/// This is the same bring-up [`super::Gpu`] does, minus the swapchain — which is
/// what D19 means by "a headless `Gpu`". It is a distinct type rather than a `Gpu`
/// with an optional swapchain because every frame path would then carry an `Option`
/// it can never see be `None`.
///
/// Only one may exist at a time: `sg_setup` is process-global. The bake tool creates
/// exactly one and drops it at the end.
pub(crate) struct Baker {
    /// Held for the life of the bake: sokol keeps a borrowed pointer to it, and
    /// dropping it early would pull the device out from under `sg_shutdown`.
    _device: backend::Device,
}

impl Baker {
    /// Create the headless device and set sokol_gfx up on it.
    pub(crate) fn new() -> GpuResult<Self> {
        let device = backend::Device::create()?;
        let mut desc = sg::Desc::new();
        // No swapchain pass is ever begun, so these defaults only have to be
        // self-consistent; every bake pass declares its own attachment formats.
        desc.environment.defaults = sg::EnvironmentDefaults {
            color_format: backend::SWAPCHAIN_FORMAT,
            depth_format: sg::PixelFormat::None,
            sample_count: 1,
        };
        device.fill_environment(&mut desc.environment);
        desc.logger = sg::Logger {
            func: Some(super::log::log_sokol),
            user_data: std::ptr::null_mut(),
        };
        sg::setup(&desc);
        if !sg::isvalid() {
            return Err(GpuError::Resource {
                kind: ResourceKind::Device,
                label: "sokol_gfx".into(),
                detail: "sg_setup did not come up on the headless bake device".into(),
            });
        }
        Ok(Self { _device: device })
    }

    /// Render one fullscreen triangle into `attachment`, a `size`×`size` target.
    ///
    /// Every bake pass has this exact shape — one target, no depth, a
    /// `gl_VertexIndex`-built triangle that covers it — so the whole pass is one
    /// call. The clear is redundant (the triangle overwrites every texel) and kept
    /// because a `LoadAction::DontCare` on a target that is later read back would
    /// leave any gap filled with whatever the allocation held.
    ///
    /// `bindings` is `None` for a shader that reads nothing — the BRDF LUT pass,
    /// which is pure math over its own fragment coordinate. sokol **rejects an empty
    /// `sg_apply_bindings`** rather than treating it as a no-op, the same rule
    /// [`super::SwapchainJob`] carries an `Option` for.
    pub(crate) fn draw_fullscreen<T: bytemuck::Pod>(
        &self,
        pipeline: &super::Pipeline,
        attachment: BakeAttachment,
        size: u32,
        bindings: Option<&super::Bindings>,
        uniforms: &[(usize, &T)],
    ) {
        let mut pass = sg::Pass::new();
        pass.attachments.colors[0] = attachment.0;
        pass.action.colors[0] = sg::ColorAttachmentAction {
            load_action: sg::LoadAction::Clear,
            store_action: sg::StoreAction::Store,
            clear_value: sg::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        };
        pass.label = c"ibl bake".as_ptr();
        sg::begin_pass(&pass);
        sg::apply_viewport(0, 0, size as i32, size as i32, true);
        pipeline.apply();
        if let Some(bindings) = bindings {
            sg::apply_bindings(bindings.raw());
        }
        for (slot, value) in uniforms {
            sg::apply_uniforms(*slot, &sg::value_as_range(*value));
        }
        sg::draw(0, 3, 1);
        sg::end_pass();
    }

    /// Submit everything drawn so far. Called before a readback, so nothing the
    /// bake is about to map is still queued.
    pub(crate) fn flush(&self) {
        sg::commit();
    }
}

impl Drop for Baker {
    fn drop(&mut self) {
        if sg::isvalid() {
            sg::shutdown();
        }
    }
}

/// A render-target cubemap (`size`×`size`, six faces × `mips`) for the IBL
/// precompute: each face/mip is attachable on its own, and the whole texture is
/// sampleable as a cube by the convolution passes.
pub(crate) struct CubeTarget {
    image: sg::Image,
    /// One attachment view per (mip, face), indexed `mip * 6 + face`.
    attachments: Vec<sg::View>,
    /// The cube view the irradiance and prefilter passes sample.
    texture: sg::View,
}

impl CubeTarget {
    pub(crate) fn new(size: u32, mips: u32, format: Format, label: &CStr) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("cube target");
        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Cube;
        desc.usage.color_attachment = true;
        desc.width = size as i32;
        desc.height = size as i32;
        desc.num_mipmaps = mips as i32;
        desc.pixel_format = format.sg();
        desc.label = label.as_ptr();
        let image = sg::make_image(&desc);
        require_valid(sg::query_image_state(image), ResourceKind::Texture, name)?;

        let mut attachments = Vec::with_capacity((mips * 6) as usize);
        for mip in 0..mips {
            for face in 0..6u32 {
                let mut view_desc = sg::ViewDesc::new();
                view_desc.color_attachment = sg::ImageViewDesc {
                    image,
                    mip_level: mip as i32,
                    slice: face as i32,
                };
                view_desc.label = label.as_ptr();
                let view = sg::make_view(&view_desc);
                require_valid(sg::query_view_state(view), ResourceKind::Texture, name)?;
                attachments.push(view);
            }
        }

        let mut view_desc = sg::ViewDesc::new();
        view_desc.texture.image = image;
        view_desc.label = label.as_ptr();
        let texture = sg::make_view(&view_desc);
        require_valid(sg::query_view_state(texture), ResourceKind::Texture, name)?;

        Ok(Self {
            image,
            attachments,
            texture,
        })
    }

    /// The attachment for `face` (0..6) at `mip`.
    pub(crate) fn attachment(&self, face: u32, mip: u32) -> BakeAttachment {
        let index = (mip * 6 + face) as usize;
        debug_assert!(
            face < 6 && index < self.attachments.len(),
            "cube attachment (mip {mip}, face {face}) out of range"
        );
        BakeAttachment(self.attachments[index])
    }

    /// The cube view a convolution pass samples. `rhi`-internal: it reaches a draw
    /// through [`super::Bindings::cube_target`], never through the caller.
    pub(in crate::rhi) fn texture(&self) -> sg::View {
        self.texture
    }

    /// The image itself, for [`backend::read_image_subresource`].
    pub(crate) fn image(&self) -> sg::Image {
        self.image
    }
}

impl Drop for CubeTarget {
    fn drop(&mut self) {
        if !sg::isvalid() {
            return;
        }
        sg::destroy_view(self.texture);
        for view in self.attachments.drain(..) {
            sg::destroy_view(view);
        }
        sg::destroy_image(self.image);
    }
}

/// A plain single-mip 2D render target — the BRDF integration LUT, which is rendered
/// into and read back and never sampled.
pub(crate) struct Target2D {
    image: sg::Image,
    attachment: sg::View,
}

impl Target2D {
    pub(crate) fn new(width: u32, height: u32, format: Format, label: &CStr) -> GpuResult<Self> {
        let name = label.to_str().unwrap_or("2d target");
        let mut desc = sg::ImageDesc::new();
        desc._type = sg::ImageType::Dim2;
        desc.usage.color_attachment = true;
        desc.width = width as i32;
        desc.height = height as i32;
        desc.num_mipmaps = 1;
        desc.pixel_format = format.sg();
        desc.label = label.as_ptr();
        let image = sg::make_image(&desc);
        require_valid(sg::query_image_state(image), ResourceKind::Texture, name)?;

        let mut view_desc = sg::ViewDesc::new();
        view_desc.color_attachment.image = image;
        view_desc.label = label.as_ptr();
        let attachment = sg::make_view(&view_desc);
        require_valid(
            sg::query_view_state(attachment),
            ResourceKind::Texture,
            name,
        )?;

        Ok(Self { image, attachment })
    }

    pub(crate) fn attachment(&self) -> BakeAttachment {
        BakeAttachment(self.attachment)
    }

    /// The image itself, for [`backend::read_image_subresource`].
    pub(crate) fn image(&self) -> sg::Image {
        self.image
    }
}

impl Drop for Target2D {
    fn drop(&mut self) {
        if !sg::isvalid() {
            return;
        }
        sg::destroy_view(self.attachment);
        sg::destroy_image(self.image);
    }
}
