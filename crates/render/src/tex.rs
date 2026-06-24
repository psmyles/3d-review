//! The Tex viewport render path: an egui paint callback that draws one pooled
//! texture (channel-isolated, pan/zoom) into egui's framebuffer.
//!
//! This is the wgpu counterpart of the 3D / UV scene callbacks, but deliberately
//! minimal: it owns a tiny fullscreen-triangle pipeline ([`tex.wgsl`]) and a small
//! GPU texture cache, and it does **not** route through the scene's linear-HDR /
//! tone-map / MRT pipeline. Channel isolation is a uniform the fragment shader
//! swizzles on — switching RGB/R/G/B/A is a buffer write, not a re-upload — and
//! the source texture is uploaded once (mipped) and reused, so the first view of a
//! texture is the only processing cost (CLAUDE.md: one-time on import, instant
//! after).
//!
//! Ownership: the decoded pixels arrive by `Arc` from the app-owned texture pool
//! (invariant 2); the UI carries no GPU state, it only emits the selected image +
//! channel + placement as plain values.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::mipmap::{MipGenerator, mip_level_count};
use crate::texture::DecodedImage;

/// The Tex viewport shader (fullscreen triangle + channel swizzle). Kept beside
/// the [`TexUniforms`] layout it must match (invariant 11).
const TEX_SHADER: &str = include_str!("tex.wgsl");

/// Linear (no sRGB decode on sample) format the Tex texture is uploaded in, so
/// `textureSample` returns the raw stored bytes for every channel alike — the
/// faithfulness contract (see `tex.wgsl`).
const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Composite-pass uniform mirroring `tex.wgsl`'s `TexUniforms` (invariant 11):
/// the image rectangle in framebuffer pixels, the channel selector, and whether
/// the framebuffer encodes sRGB on write.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TexUniforms {
    img_min: [f32; 2],
    img_size: [f32; 2],
    channel: u32,
    target_srgb: u32,
    pad: [u32; 2],
}

/// Most distinct textures kept resident on the GPU at once. Re-selecting a cached
/// texture is instant, but each upload holds VRAM until evicted, so the cache is an
/// LRU bounded to this many entries: opening a 17th texture frees the
/// least-recently-viewed one. Sized for comfortable flipping through a working set
/// while keeping memory bounded on large asset libraries.
const TEX_CACHE_CAP: usize = 16;

/// An uploaded Tex texture: the mip-mapped GPU texture's bind group (group 1), the
/// decoded image it was built from for an identity check (a disk reload produces a
/// new `Arc`, which forces a re-upload), and a recency stamp for LRU eviction.
struct CachedTex {
    image: Arc<DecodedImage>,
    bind_group: wgpu::BindGroup,
    /// `TexResources::tick` value at the last access; the smallest is the LRU.
    last_used: u64,
}

/// The GPU resource cache for the Tex viewport, held in egui's `CallbackResources`
/// (one instance, reused across frames). Caches viewed textures by path so
/// re-selecting one is instant; the set is an LRU bounded to [`TEX_CACHE_CAP`]
/// entries, so a long session inspecting many textures can't grow VRAM without
/// bound.
struct TexResources {
    output_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    mip_generator: MipGenerator,
    cache: HashMap<PathBuf, CachedTex>,
    /// Monotonic access counter stamped onto a `CachedTex` on each touch; drives
    /// LRU eviction.
    tick: u64,
}

impl TexResources {
    fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_tex_shader"),
            source: wgpu::ShaderSource::Wgsl(TEX_SHADER.into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_tex_uniform_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_tex_texture_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Crisp texels when zoomed in (nearest magnify), smooth when zoomed out
        // (linear minify over the mip chain). Clamp so the border never wraps.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_tex_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_tex_uniform_buffer"),
            size: std::mem::size_of::<TexUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_tex_uniform_bind_group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_tex_pipeline_layout"),
            bind_group_layouts: &[&uniform_layout, &tex_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("review_tex_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            // Draws into egui's framebuffer, so it must match egui's depth state /
            // MSAA (like the scene composite). Depth is ignored: never written,
            // always passes.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::scene::EGUI_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: crate::scene::EGUI_MSAA_SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                // Straight-alpha blend so an image's transparency (RGB view)
                // composites over the egui background fill drawn behind it.
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        Self {
            output_format,
            pipeline,
            tex_layout,
            sampler,
            uniform_buffer,
            uniform_bind_group,
            mip_generator: MipGenerator::new(device),
            cache: HashMap::new(),
            tick: 0,
        }
    }

    /// Upload `image` into a mip-mapped GPU texture + bind group (keyed by `path`)
    /// if it isn't already cached for this exact decoded image, and mark `path` as
    /// most-recently-used. A disk reload swaps the `Arc`, so the identity check
    /// rebuilds it; an unchanged image is a touch-only no-op. When inserting a new
    /// path would exceed [`TEX_CACHE_CAP`], the least-recently-used entry is evicted.
    fn ensure_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        image: &Arc<DecodedImage>,
    ) {
        self.tick += 1;
        let now = self.tick;
        if let Some(cached) = self.cache.get_mut(path) {
            if Arc::ptr_eq(&cached.image, image) {
                cached.last_used = now;
                return;
            }
        }
        let view = upload_mipped(device, queue, &mut self.mip_generator, image);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_tex_bind_group"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        // Cap only on genuine inserts: replacing a path's stale upload (disk reload)
        // doesn't grow the set, so it must not evict another entry.
        if !self.cache.contains_key(path) && self.cache.len() >= TEX_CACHE_CAP {
            if let Some(lru) = self
                .cache
                .iter()
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(lru_path, _)| lru_path.clone())
            {
                self.cache.remove(&lru);
            }
        }
        self.cache.insert(
            path.to_path_buf(),
            CachedTex {
                image: Arc::clone(image),
                bind_group,
                last_used: now,
            },
        );
    }
}

/// Upload a decoded image into a mip-mapped [`TEX_FORMAT`] texture and return its
/// view. The image's bytes are uploaded verbatim (no sRGB decode); the mip chain
/// is built by [`MipGenerator`] so minification has levels to filter. A degenerate
/// / mismatched buffer falls back to a single white texel so a decode hiccup never
/// panics the render path.
fn upload_mipped(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mip_generator: &mut MipGenerator,
    image: &DecodedImage,
) -> wgpu::TextureView {
    let width = image.width.max(1);
    let height = image.height.max(1);
    let (width, height, rgba): (u32, u32, &[u8]) =
        if image.rgba.len() < (width as usize * height as usize * 4) {
            (1, 1, &[255, 255, 255, 255])
        } else {
            (width, height, &image.rgba)
        };

    let mip_levels = mip_level_count(width, height);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("review_tex_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEX_FORMAT,
        // RENDER_ATTACHMENT lets the mip generator blit each downsampled level.
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    mip_generator.generate(device, queue, &texture, TEX_FORMAT, mip_levels);
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The egui paint callback for the Tex viewport. Carries the viewed image (shared
/// by `Arc` from the app-owned pool), where to draw it (in egui points), and which
/// channel to isolate. `prepare` ensures the GPU texture exists + writes the
/// placement/channel uniform; `paint` issues the single fullscreen draw.
#[derive(Debug, Clone)]
pub struct TexCallback {
    path: PathBuf,
    image: Arc<DecodedImage>,
    output_format: wgpu::TextureFormat,
    /// 0 = RGB, 1 = R, 2 = G, 3 = B, 4 = A (matches `tex.wgsl`'s `channel`).
    channel: u32,
    /// Image rectangle top-left in egui points (converted to framebuffer pixels in
    /// `prepare` via the screen descriptor's points-per-pixel scale).
    min_points: [f32; 2],
    /// Image rectangle size in egui points.
    size_points: [f32; 2],
}

impl TexCallback {
    /// Build a Tex callback for `image` (loaded from `path`), to be drawn at
    /// `min_points` with `size_points` (egui points) showing `channel`
    /// (0 = RGB, 1..4 = R/G/B/A). `output_format` is egui's framebuffer format.
    pub fn new(
        path: PathBuf,
        image: Arc<DecodedImage>,
        output_format: wgpu::TextureFormat,
        channel: u32,
        min_points: [f32; 2],
        size_points: [f32; 2],
    ) -> Self {
        Self {
            path,
            image,
            output_format,
            channel,
            min_points,
            size_points,
        }
    }
}

impl CallbackTrait for TexCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<TexResources>()
            .or_insert_with(|| TexResources::new(device, self.output_format));
        if resources.output_format != self.output_format {
            *resources = TexResources::new(device, self.output_format);
        }

        resources.ensure_texture(device, queue, &self.path, &self.image);

        let ppp = screen_descriptor.pixels_per_point;
        let uniforms = TexUniforms {
            img_min: [self.min_points[0] * ppp, self.min_points[1] * ppp],
            img_size: [
                (self.size_points[0] * ppp).max(1.0),
                (self.size_points[1] * ppp).max(1.0),
            ],
            channel: self.channel,
            target_srgb: u32::from(self.output_format.is_srgb()),
            pad: [0, 0],
        };
        queue.write_buffer(&resources.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<TexResources>() else {
            return;
        };
        let Some(cached) = resources.cache.get(&self.path) else {
            return;
        };
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
        render_pass.set_bind_group(1, &cached.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod shader_tests {
    /// The Tex shader must parse + validate (matches the scene shader's guard):
    /// naga catches type / control-flow / binding mistakes without a GPU.
    #[test]
    fn tex_shader_validates() {
        let module =
            naga::front::wgsl::parse_str(super::TEX_SHADER).expect("tex.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("tex.wgsl should validate");
    }
}
