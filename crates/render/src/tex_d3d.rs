//! The Tex viewport's Direct3D 11 render path.
//!
//! Deliberately minimal and separate from the scene path: a fullscreen-triangle
//! pipeline (`tex.hlsl`) with a small path-keyed GPU texture cache, drawn straight to
//! the backbuffer **before** the egui chrome (egui-directx11 has no paint callback).
//! It owns no scene state. Channel isolation is a uniform the pixel shader swizzles
//! on — switching RGB/R/G/B/A is a buffer write, not a re-upload — and the source
//! texture is uploaded once (mipped, raw `Rgba8Unorm`) and reused, so only the first
//! view of a texture costs work.
//!
//! Ownership: the decoded pixels arrive by `Arc` from the app-owned texture pool
//! (invariant 2); the UI emits only the selected image + channel + placement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::rhi::{
    BlendMode, Cull, DepthBias, DepthCompare, DepthState, DynamicConstantBuffer, Gpu, Pipeline,
    PipelineDesc, Sampler, Texture, Topology,
};
use crate::texture::DecodedImage;

/// Compiled DXBC — see `build.rs`.
const TEX_VS: &[u8] = include_bytes!("hlsl/tex.vs.dxbc");
const TEX_IMAGE_PS: &[u8] = include_bytes!("hlsl/tex.image.ps.dxbc");
const TEX_CHECKER_PS: &[u8] = include_bytes!("hlsl/tex.checker.ps.dxbc");

/// Most distinct textures kept resident on the GPU at once — an LRU bound so a long
/// session inspecting many textures can't grow VRAM without limit (matches the wgpu
/// `TEX_CACHE_CAP`). Opening a 17th texture frees the least-recently-viewed one.
const TEX_CACHE_CAP: usize = 16;

/// The grey solid background fill (gamma-space, written verbatim to the UNORM
/// backbuffer) — `Color32::from_gray(128)` in the UI theme.
const GREY_FILL: [f32; 4] = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0];
/// The transparency-checker colors — `from_gray(170)` / `from_gray(110)` in the
/// theme (the Tex viewport's "Checker" background).
const CHECKER_LIGHT: [f32; 4] = [170.0 / 255.0, 170.0 / 255.0, 170.0 / 255.0, 1.0];
const CHECKER_DARK: [f32; 4] = [110.0 / 255.0, 110.0 / 255.0, 110.0 / 255.0, 1.0];

/// The Tex viewport's background fill (chosen in the status bar), drawn behind the
/// image so transparency reads against a known backdrop. A render-side mirror of the
/// UI's `TextureBackground`; `app` maps one to the other.
#[derive(Debug, Clone, Copy)]
pub enum TexBackground {
    Black,
    White,
    Grey,
    /// The 2-color transparency checker; `cell_px` is one cell's size in physical
    /// pixels (the UI's cell size scaled by the points-per-pixel factor).
    Checker {
        cell_px: f32,
    },
}

/// One image to draw in the Tex viewport: the decoded pixels (shared by `Arc` from
/// the app-owned pool, keyed by `path` for the GPU cache + a disk-reload identity
/// check), the channel to isolate (0 = RGB, 1..4 = R/G/B/A), and the image rectangle
/// in physical framebuffer pixels (`app` resolves it from the canvas + pan/zoom).
pub struct TexImage {
    pub path: PathBuf,
    pub image: Arc<DecodedImage>,
    pub channel: u32,
    pub min_px: [f32; 2],
    pub size_px: [f32; 2],
}

/// Composite uniform mirroring `tex.hlsl`'s `TexUniforms` (invariant 11): the image
/// placement + channel + sRGB flag (for `fs_image`) and the checker cell + colors
/// (for `fs_checker`). One buffer serves both passes; each reads the fields it needs.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct TexUniforms {
    img_min: [f32; 2],
    img_size: [f32; 2],
    channel: u32,
    target_srgb: u32,
    checker_cell: f32,
    pad: u32,
    bg_light: [f32; 4],
    bg_dark: [f32; 4],
}

/// An uploaded Tex texture: the mip-mapped GPU texture, the decoded image it was
/// built from (an identity check — a disk reload swaps the `Arc`, forcing a
/// re-upload), and a recency stamp for LRU eviction.
struct CachedTex {
    image: Arc<DecodedImage>,
    texture: Texture,
    last_used: u64,
}

/// The Direct3D 11 GPU resources for the Tex viewport, built once (lazily) on the
/// first [`crate::Renderer::render_texture`]. Caches viewed textures by path (an LRU
/// bounded to [`TEX_CACHE_CAP`]) so re-selecting one is instant.
pub(crate) struct TexGpu {
    /// Image draw (`fs_image`): channel-isolated, alpha-blended over the background.
    image_pipeline: Pipeline,
    /// Background checker (`fs_checker`): opaque fullscreen fill.
    checker_pipeline: Pipeline,
    uniforms: DynamicConstantBuffer,
    sampler: Sampler,
    cache: HashMap<PathBuf, CachedTex>,
    /// Monotonic access counter stamped onto a `CachedTex` on each touch; the
    /// smallest is the LRU.
    tick: u64,
}

impl std::fmt::Debug for TexGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TexGpu").finish_non_exhaustive()
    }
}

impl TexGpu {
    /// Build the Tex viewport GPU resources. Called once on the first Tex frame.
    pub(crate) fn new(gpu: &Gpu) -> windows::core::Result<Self> {
        let device = gpu.device();
        // Both passes are a fullscreen triangle (no vertex buffer / input layout)
        // with depth disabled; the image alpha-blends over the background, the
        // checker overwrites opaquely.
        let fullscreen = |ps: &'static [u8], blend| PipelineDesc {
            vs: TEX_VS,
            ps,
            input: &[],
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: DepthState {
                test: false,
                write: false,
                compare: DepthCompare::Always,
            },
            blend,
            depth_bias: DepthBias::default(),
            sample_count: 1,
        };
        let image_pipeline =
            Pipeline::new(device, &fullscreen(TEX_IMAGE_PS, BlendMode::AlphaBlend))?;
        let checker_pipeline =
            Pipeline::new(device, &fullscreen(TEX_CHECKER_PS, BlendMode::Opaque))?;
        let uniforms = DynamicConstantBuffer::new::<TexUniforms>(device)?;
        let sampler = Sampler::tex_view(device)?;
        Ok(Self {
            image_pipeline,
            checker_pipeline,
            uniforms,
            sampler,
            cache: HashMap::new(),
            tick: 0,
        })
    }

    /// Draw the Tex viewport to the backbuffer: the background fill, then the viewed
    /// image (when present) channel-isolated + alpha-blended over it. The egui chrome
    /// is drawn on top afterwards by `app` (its canvas region is transparent).
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        image: Option<TexImage>,
        background: TexBackground,
    ) -> windows::core::Result<()> {
        let device = gpu.device();
        let ctx = gpu.context();

        // Solid backgrounds clear the whole backbuffer (the chrome bands then cover
        // everything outside the canvas); the checker draws a fullscreen pass.
        let solid = match background {
            TexBackground::Black => Some([0.0, 0.0, 0.0, 1.0]),
            TexBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            TexBackground::Grey => Some(GREY_FILL),
            TexBackground::Checker { .. } => None,
        };
        if let Some(color) = solid {
            gpu.clear_backbuffer(color);
        }
        gpu.begin_backbuffer_blit();

        if let TexBackground::Checker { cell_px } = background {
            let uniforms = TexUniforms {
                checker_cell: cell_px.max(1.0),
                bg_light: CHECKER_LIGHT,
                bg_dark: CHECKER_DARK,
                ..TexUniforms::default()
            };
            self.uniforms.update(ctx, &uniforms)?;
            self.checker_pipeline.bind(ctx);
            self.uniforms.bind_ps(ctx, 0);
            gpu.draw(3);
        }

        if let Some(image) = image {
            self.ensure_texture(device, ctx, &image.path, &image.image)?;
            if let Some(cached) = self.cache.get(&image.path) {
                let uniforms = TexUniforms {
                    img_min: image.min_px,
                    img_size: [image.size_px[0].max(1.0), image.size_px[1].max(1.0)],
                    channel: image.channel,
                    // Our backbuffer is UNORM (gamma): write the source bytes verbatim.
                    target_srgb: 0,
                    ..TexUniforms::default()
                };
                self.uniforms.update(ctx, &uniforms)?;
                self.image_pipeline.bind(ctx);
                self.uniforms.bind_ps(ctx, 0);
                cached.texture.bind_ps(ctx, 0);
                self.sampler.bind_ps(ctx, 0);
                gpu.draw(3);
                gpu.unbind_ps_srvs(1);
            }
        }

        Ok(())
    }

    /// Upload `image` into a mip-mapped GPU texture (keyed by `path`) if it isn't
    /// already cached for this exact decoded image, marking `path` most-recently-used.
    /// A disk reload swaps the `Arc`, so the identity check rebuilds it; an unchanged
    /// image is a touch-only no-op. A genuine insert past [`TEX_CACHE_CAP`] evicts the
    /// least-recently-used entry. Mirrors the wgpu `ensure_texture`.
    fn ensure_texture(
        &mut self,
        device: &ID3D11Device,
        ctx: &ID3D11DeviceContext,
        path: &Path,
        image: &Arc<DecodedImage>,
    ) -> windows::core::Result<()> {
        self.tick += 1;
        let now = self.tick;
        if let Some(cached) = self.cache.get_mut(path)
            && Arc::ptr_eq(&cached.image, image)
        {
            cached.last_used = now;
            return Ok(());
        }
        let texture = upload_texture(device, ctx, image)?;
        // Cap only on genuine inserts: replacing a path's stale upload (disk reload)
        // doesn't grow the set, so it must not evict another entry.
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

/// Upload a decoded image into a mip-mapped raw (`Rgba8Unorm`, no sRGB decode)
/// texture. A degenerate / mismatched buffer falls back to a single white texel so a
/// decode hiccup never panics the render path. Mirrors the wgpu `upload_mipped`.
fn upload_texture(
    device: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    image: &DecodedImage,
) -> windows::core::Result<Texture> {
    let width = image.width.max(1);
    let height = image.height.max(1);
    if image.rgba.len() < (width as usize * height as usize * 4) {
        Texture::rgba8_mipped(device, ctx, 1, 1, &[255, 255, 255, 255], false)
    } else {
        Texture::rgba8_mipped(device, ctx, width, height, &image.rgba, false)
    }
}
