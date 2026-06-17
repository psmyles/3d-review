//! The offscreen render targets the scene draws into — the seed of the
//! `RenderTargets` registry (CLAUDE.md render roadmap, Phase 1). Rather than
//! drawing straight into egui's framebuffer, the scene renders into these
//! render-owned textures and is composited back through the post pass
//! (`post.rs`). That offscreen seam is what later passes (tone mapping, bloom,
//! SSAO, IBL, GPU-buffer visualization) hook into.
//!
//! The targets carry the scene's MSAA level (`AntiAliasing::msaa`, Phase 2): at
//! 2×/4×/8×/16× the color is multisampled and resolved into a single-sample
//! texture the post pass samples; at `Off` (1×) the single render texture *is*
//! the sampled texture and there is no resolve. Future passes register
//! additional named targets here so framebuffer-size handling stays in one place.

use crate::scene::SCENE_DEPTH_FORMAT;

/// Color format of the offscreen scene target. `Rgba16Float` is a linear,
/// HDR-capable format: it gives later phases headroom above 1.0 for tone mapping
/// and bloom without changing the target. Phase 1 still stores display-space
/// color in it (the scene shader keeps owning lighting / tone map / sRGB encode,
/// so compositing stays byte-for-byte what egui produced before); switching the
/// scene shader to emit linear HDR is deferred to the phase that adds IBL/bloom.
pub(crate) const SCENE_HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The named offscreen targets for one framebuffer size + MSAA level. Recreated
/// wholesale when the framebuffer is resized or the sample count changes
/// (textures are immutable in both), via `SceneResources::sync_targets`.
///
/// The scene geometry pass renders to **two** color attachments (MRT): the
/// display-space `color` the composite shows (tone-mapped + sRGB-encoded in the
/// scene shader, exactly as before bloom existed) and a parallel **linear HDR**
/// `bloom` target carrying the pre-tone-map radiance of the lit surfaces / skybox
/// (overlays write 0 there, so they never glow). Bloom thresholds + blurs the
/// `bloom` target; keeping the two separate is what lets bloom read true HDR
/// without disturbing the display color or the overlays.
pub(crate) struct SceneTargets {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// MSAA sample count the scene renders at (1 = no multisampling).
    pub(crate) sample_count: u32,
    /// Display-space color attachment the scene geometry renders into.
    /// Multisampled when `sample_count > 1`, otherwise the single-sample sampled
    /// texture itself.
    pub(crate) color_render_view: wgpu::TextureView,
    /// Single-sample resolve of `color_render_view`, present only when MSAA is on
    /// (`sample_count > 1`). `None` at 1× — the render texture is already
    /// single-sample and is sampled directly.
    pub(crate) color_resolve_view: Option<wgpu::TextureView>,
    /// Linear-HDR bloom-source attachment (MRT location 1), MSAA-matched to
    /// `color_render_view`.
    pub(crate) bloom_render_view: wgpu::TextureView,
    /// Single-sample resolve of `bloom_render_view` (the texture bloom samples),
    /// present only when MSAA is on.
    pub(crate) bloom_resolve_view: Option<wgpu::TextureView>,
    /// Depth used while drawing the scene, matching `sample_count`.
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
        let sample_count = sample_count.max(1);
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let multisampled = sample_count > 1;

        // The texture downstream passes sample must carry TEXTURE_BINDING. At 1×
        // the render texture is sampled directly, so it gets that usage; at 2×+
        // the MSAA color is render-only and the separate resolve target is sampled.
        let render_usage = if multisampled {
            wgpu::TextureUsages::RENDER_ATTACHMENT
        } else {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        };
        // The two HDR color attachments (display-space color + linear bloom
        // source) are created identically; only their role differs.
        let make_render = |label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format: SCENE_HDR_FORMAT,
                    usage: render_usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color_render_view = make_render("review_scene_color_render");
        let bloom_render_view = make_render("review_scene_bloom_render");

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
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        let make_resolve = |label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SCENE_HDR_FORMAT,
                    // RENDER_ATTACHMENT so it can be the MSAA resolve target;
                    // TEXTURE_BINDING so a later pass can sample it.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (color_resolve_view, bloom_resolve_view) = if multisampled {
            let color_resolve = make_resolve("review_scene_color_resolved");
            let bloom_resolve = make_resolve("review_scene_bloom_resolved");

            // The resolve targets are only ever written by the MSAA resolve, never
            // by a clear/discard. wgpu's lazy zero-init doesn't fire for a resolve
            // destination, so D3D12's debug layer flags them as "used
            // uninitialized" on the first resolve. The resolve fully overwrites
            // each texture every frame (so this is benign), but a one-time clear
            // here satisfies the validation requirement that a render-target
            // resource be initialized before first use. The render colors + depth
            // need no such clear — they are cleared every frame by the offscreen
            // pass's `LoadOp::Clear`.
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("review_scene_resolve_init"),
            });
            for view in [&color_resolve, &bloom_resolve] {
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("review_scene_resolve_init_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
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

            (Some(color_resolve), Some(bloom_resolve))
        } else {
            (None, None)
        };

        Self {
            width,
            height,
            sample_count,
            color_render_view,
            color_resolve_view,
            bloom_render_view,
            bloom_resolve_view,
            depth_view,
        }
    }

    /// The single-sample view the composite pass samples: the resolve target when
    /// MSAA is on, otherwise the (already single-sample) render texture.
    pub(crate) fn sampled_view(&self) -> &wgpu::TextureView {
        self.color_resolve_view
            .as_ref()
            .unwrap_or(&self.color_render_view)
    }

    /// The single-sample linear-HDR bloom source bloom passes read: the bloom
    /// resolve when MSAA is on, otherwise the (already single-sample) bloom render
    /// texture.
    pub(crate) fn sampled_bloom_view(&self) -> &wgpu::TextureView {
        self.bloom_resolve_view
            .as_ref()
            .unwrap_or(&self.bloom_render_view)
    }
}
