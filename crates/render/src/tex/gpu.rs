//! The Tex viewport's render path.
//!
//! Deliberately minimal and separate from the scene path: two fullscreen-triangle
//! programs (`tex_checker`, `tex_image` in `review.glsl`) with a small path-keyed GPU
//! texture cache. It owns no scene state. Channel isolation is a uniform the fragment
//! shader swizzles on — switching RGB/R/G/B/A is a uniform write, not a re-upload —
//! and the source texture is uploaded once (mipped, raw `Rgba8`) and reused, so only
//! the first view of a texture costs work.
//!
//! Both draws are [`SwapchainJob`]s rather than immediate draws: [`crate::Renderer`]
//! runs before the frame's one swapchain pass opens (`mac-port-plan.md` §3.2). The
//! *background* is not drawn at all for a solid fill — it is that pass's clear colour
//! ([`TexBackground::clear_color`]) — so only the checker needs a draw of its own.
//!
//! Ownership: the decoded pixels arrive by `Arc` from the app-owned texture pool
//! (invariant 2); the UI emits only the selected image + channel + placement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};

use super::{TexBackground, TexImage};
use crate::rhi::{
    Bindings, Blend, Filter, Frame, GpuResult, Pipeline, PipelineDesc, Sampler, SwapchainJob,
    Texture, Wrap, shader,
};
use crate::shaders::generated;
use crate::texture::DecodedImage;

/// Most distinct textures kept resident on the GPU at once — an LRU bound so a long
/// session inspecting many textures can't grow VRAM without limit. Opening a 17th
/// texture frees the least-recently-viewed one.
const TEX_CACHE_CAP: usize = 16;

/// The transparency-checker colours — `from_gray(170)` / `from_gray(110)` in the UI
/// theme (the Tex viewport's "Checker" background).
const CHECKER_LIGHT: [f32; 4] = [170.0 / 255.0, 170.0 / 255.0, 170.0 / 255.0, 1.0];
const CHECKER_DARK: [f32; 4] = [110.0 / 255.0, 110.0 / 255.0, 110.0 / 255.0, 1.0];

/// Vertices in a fullscreen triangle; `vs_fullscreen_bare` builds its corners from
/// `gl_VertexIndex` alone, so neither Tex draw binds a vertex buffer.
const FULLSCREEN_VERTICES: usize = 3;

/// The `tex_params` uniform block, mirroring `review.glsl` (invariant 11): the image
/// placement + channel + sRGB flag (for `fs_tex_image`) and the checker cell +
/// colours (for `fs_tex_checker`). One block serves both programs; each reads the
/// fields it needs.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct TexUniforms {
    img_min: [f32; 2],
    img_size: [f32; 2],
    /// 0 = RGB, 1..4 = R/G/B/A. `i32` because a uniform block cannot declare `uint`,
    /// so the shader (and shdc's generated struct) call it `int`.
    channel: i32,
    target_srgb: i32,
    checker_cell: f32,
    pad: i32,
    bg_light: [f32; 4],
    bg_dark: [f32; 4],
}

// Byte-size lock against the shader's block (invariant 11). The block is sized from
// `size_of::<T>()` and an upload is rejected only when it is *larger*, so a field
// added on one side alone uploads happily while the shader reads every later field
// shifted — wrong pixels, not an error. The two `vec2`s and the four scalars each
// pack into one 16-byte row.
const _: () = assert!(size_of::<TexUniforms>() == 64);
const _: () = assert!(size_of::<TexUniforms>() == size_of::<generated::TexParams>());
crate::shaders::assert_same_layout!(TexUniforms => generated::TexParams, {
    img_min => img_min,
    img_size => img_size,
    channel => channel,
    target_srgb => target_srgb,
    checker_cell => checker_cell,
    pad => pad,
    bg_light => bg_light,
    bg_dark => bg_dark,
});

/// An uploaded Tex texture: the mip-mapped GPU texture, the decoded image it was
/// built from (an identity check — a disk reload swaps the `Arc`, forcing a
/// re-upload), and a recency stamp for LRU eviction.
struct CachedTex {
    image: Arc<DecodedImage>,
    texture: Texture,
    last_used: u64,
}

/// The Tex viewport's GPU resources, built once (lazily) on the first
/// [`crate::Renderer::render_texture`]. Caches viewed textures by path (an LRU bounded
/// to [`TEX_CACHE_CAP`]) so re-selecting one is instant.
pub(crate) struct TexGpu {
    /// Image draw (`fs_tex_image`): channel-isolated, alpha-blended over the
    /// background.
    image_pipeline: Pipeline,
    /// Background checker (`fs_tex_checker`): opaque fullscreen fill.
    checker_pipeline: Pipeline,
    /// Linear minify / point magnify, clamped: crisp texels zoomed in, smooth over
    /// the mip chain zoomed out, and no wrap at the image border.
    sampler: Sampler,
    cache: HashMap<PathBuf, CachedTex>,
    /// Monotonic access counter stamped onto a `CachedTex` on each touch; the
    /// smallest is the LRU.
    tick: u64,
}

impl std::fmt::Debug for TexGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TexGpu")
            .field("cached", &self.cache.len())
            .finish_non_exhaustive()
    }
}

impl TexGpu {
    /// Build the Tex viewport's GPU resources. Called once, on the first Tex frame.
    pub(crate) fn new() -> GpuResult<Self> {
        let image_shader = shader::make(
            generated::tex_image_shader_desc,
            shader::bytecode!("tex_image"),
            c"tex image",
        )?;
        let image_pipeline = Pipeline::new(&PipelineDesc {
            blend: Blend::StraightAlpha,
            ..PipelineDesc::swapchain(image_shader, c"tex image")
        })?;
        let checker_shader = shader::make(
            generated::tex_checker_shader_desc,
            shader::bytecode!("tex_checker"),
            c"tex checker",
        )?;
        let checker_pipeline =
            Pipeline::new(&PipelineDesc::swapchain(checker_shader, c"tex checker"))?;
        Ok(Self {
            image_pipeline,
            checker_pipeline,
            sampler: Sampler::new(
                Filter::Linear,
                Filter::Nearest,
                Wrap::ClampToEdge,
                c"tex view",
            )?,
            cache: HashMap::new(),
            tick: 0,
        })
    }

    /// Record the Tex viewport into `frame`: the background fill, then the viewed
    /// image (when present) channel-isolated and alpha-blended over it. The egui
    /// chrome is painted on top afterwards, in the same pass.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        image: Option<TexImage>,
        background: TexBackground,
    ) -> GpuResult<()> {
        // A solid background *is* the pass's clear — there is nothing to draw behind
        // the image. The checker clears to its dark cell and draws the pattern over
        // it, so a frame that somehow skipped the draw reads as a dark backdrop
        // rather than as whatever the last frame left.
        frame.set_clear(background.clear_color());

        if let TexBackground::Checker { cell_px } = background {
            frame.queue(SwapchainJob::new(
                &self.checker_pipeline,
                FULLSCREEN_VERTICES,
                generated::UB_TEX_PARAMS,
                &TexUniforms {
                    checker_cell: cell_px.max(1.0),
                    bg_light: CHECKER_LIGHT,
                    bg_dark: CHECKER_DARK,
                    ..TexUniforms::default()
                },
            ));
        }

        let Some(image) = image else {
            return Ok(());
        };
        self.ensure_texture(&image.path, &image.image)?;
        let Some(cached) = self.cache.get(&image.path) else {
            return Ok(());
        };
        let mut bindings = Bindings::new();
        bindings.texture(generated::VIEW_TEX, &cached.texture);
        bindings.sampler(generated::SMP_SAMP, &self.sampler);
        frame.queue(
            SwapchainJob::new(
                &self.image_pipeline,
                FULLSCREEN_VERTICES,
                generated::UB_TEX_PARAMS,
                &TexUniforms {
                    img_min: image.min_px,
                    img_size: [image.size_px[0].max(1.0), image.size_px[1].max(1.0)],
                    channel: image.channel as i32,
                    // The backbuffer is plain UNORM (D20): write the source bytes
                    // verbatim, no encode.
                    target_srgb: 0,
                    ..TexUniforms::default()
                },
            )
            .with_bindings(&bindings),
        );
        Ok(())
    }

    /// Drop every uploaded texture (invariant 3). The pipelines and sampler stay —
    /// they are a few hundred bytes, and rebuilding them would cost a shader-object
    /// creation on re-entry, while the cache is the VRAM: up to [`TEX_CACHE_CAP`]
    /// mipped uploads, ~22 MB apiece at 4K.
    pub(crate) fn release_cache(&mut self) {
        self.cache.clear();
    }

    /// Upload `image` into a mip-mapped GPU texture (keyed by `path`) if it isn't
    /// already cached for this exact decoded image, marking `path` most-recently-used.
    /// A disk reload swaps the `Arc`, so the identity check rebuilds it; an unchanged
    /// image is a touch-only no-op. A genuine insert past [`TEX_CACHE_CAP`] evicts the
    /// least-recently-used entry.
    fn ensure_texture(&mut self, path: &Path, image: &Arc<DecodedImage>) -> GpuResult<()> {
        self.tick += 1;
        let now = self.tick;
        if let Some(cached) = self.cache.get_mut(path)
            && Arc::ptr_eq(&cached.image, image)
        {
            cached.last_used = now;
            return Ok(());
        }
        // Raw upload (`Rgba8`, no sRGB decode) so the displayed texel equals the
        // stored texel; a degenerate buffer falls back to a white texel.
        let texture = Texture::rgba8_mipped_or_white(
            image.width,
            image.height,
            &image.rgba,
            false,
            c"tex image",
        )?;
        // Cap only on genuine inserts: replacing a path's stale upload (a disk
        // reload) doesn't grow the set, so it must not evict another entry.
        if !self.cache.contains_key(path)
            && self.cache.len() >= TEX_CACHE_CAP
            && let Some(lru) = self
                .cache
                .iter()
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(lru_path, _)| lru_path.clone())
        {
            self.cache.remove(&lru);
        }
        self.cache.insert(
            path.to_path_buf(),
            CachedTex {
                image: Arc::clone(image),
                texture,
                last_used: now,
            },
        );
        Ok(())
    }
}
