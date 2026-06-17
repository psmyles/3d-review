//! The offscreen render targets the scene draws into — the seed of the
//! `RenderTargets` registry (CLAUDE.md render roadmap, Phase 1). Rather than
//! drawing straight into egui's framebuffer, the scene now renders into these
//! app-of-render-owned textures and is composited back through the post pass
//! (`post.rs`). That offscreen seam is what later passes (tone mapping, bloom,
//! SSAO, IBL, GPU-buffer visualization) hook into.
//!
//! Phase 1 holds exactly three named targets: the MSAA HDR color, its resolve
//! target (sampled by the post pass), and the MSAA depth. Future passes register
//! additional named targets here so framebuffer-size handling stays in one place.

use crate::scene::SCENE_DEPTH_FORMAT;

/// Color format of the offscreen scene target. `Rgba16Float` is a linear,
/// HDR-capable format: it gives later phases headroom above 1.0 for tone mapping
/// and bloom without changing the target. Phase 1 still stores display-space
/// color in it (the scene shader keeps owning lighting / tone map / sRGB encode,
/// so compositing stays byte-for-byte what egui produced before); switching the
/// scene shader to emit linear HDR is deferred to the phase that adds IBL/bloom.
pub(crate) const SCENE_HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The named offscreen targets for one framebuffer size. Recreated wholesale when
/// the framebuffer is resized (textures are immutable in size), via
/// `SceneResources::sync_targets`.
pub(crate) struct SceneTargets {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Multisampled HDR color the scene geometry renders into.
    pub(crate) color_msaa_view: wgpu::TextureView,
    /// Single-sample resolve of `color_msaa_view`; sampled by the post pass. A
    /// `TextureView` keeps its texture alive, so the texture isn't stored.
    pub(crate) color_resolved_view: wgpu::TextureView,
    /// Multisampled depth used while drawing the scene.
    pub(crate) depth_view: wgpu::TextureView,
}

impl SceneTargets {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Self {
        // Guard against a zero-sized framebuffer (minimized window): wgpu rejects
        // a 0 extent. A 1x1 target is harmless until the real size arrives.
        let width = width.max(1);
        let height = height.max(1);
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let color_msaa = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("review_scene_color_msaa"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color_resolved = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("review_scene_color_resolved"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_HDR_FORMAT,
            // RENDER_ATTACHMENT so it can be the MSAA resolve target;
            // TEXTURE_BINDING so the post pass can sample it.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("review_scene_depth"),
            size: extent,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let color_msaa_view = color_msaa.create_view(&wgpu::TextureViewDescriptor::default());
        let color_resolved_view =
            color_resolved.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        // The resolve target is only ever written by the MSAA resolve, never by a
        // clear/discard. wgpu's lazy zero-init doesn't fire for a resolve
        // destination, so D3D12's debug layer flags it as "used uninitialized" on
        // the first resolve. The resolve fully overwrites the texture each frame
        // (so this is benign), but a one-time clear here satisfies the validation
        // requirement that a render-target resource be initialized before first
        // use. The MSAA color and depth targets need no such clear — they're
        // cleared every frame by the offscreen pass's `LoadOp::Clear`.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("review_scene_resolve_init"),
        });
        {
            let _init_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("review_scene_resolve_init_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_resolved_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        queue.submit(std::iter::once(encoder.finish()));

        Self {
            width,
            height,
            color_msaa_view,
            color_resolved_view,
            depth_view,
        }
    }
}
