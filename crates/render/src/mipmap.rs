//! GPU mipmap generation for the material texture slots.
//!
//! `wgpu` has no built-in mip generation, so freshly-uploaded material textures
//! (which arrive as a single full-resolution level) get their mip chain built here
//! by repeated box-downsample: each level is produced by a fullscreen blit that
//! bilinearly samples the level above. Color slots are `Rgba8UnormSrgb`, so the
//! sample decodes to linear, averages in linear space, and the sRGB render target
//! re-encodes on write — the correct way to downsample sRGB-authored color. Linear
//! data slots (`Rgba8Unorm`) average directly.
//!
//! `MipGenerator` owns the size-independent blit shader + sampler and a
//! per-format pipeline cache (one pipeline each for the sRGB and linear slot
//! formats). It is held by the [`crate::material::MaterialTable`] and invoked when
//! a slot's image is uploaded; mip-filtered + anisotropic sampling then resolves
//! the noise that single-level minification produced.

use std::collections::HashMap;

const MIPMAP_SHADER: &str = include_str!("mipmap.wgsl");

/// Builds full mip chains for 2D material textures via fullscreen downsample blits.
pub(crate) struct MipGenerator {
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    /// One blit pipeline per render-target format (the sRGB + linear slot formats),
    /// built on first use and reused thereafter.
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl MipGenerator {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_mipmap_shader"),
            source: wgpu::ShaderSource::Wgsl(MIPMAP_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_mipmap_bind_group_layout"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_mipmap_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Clamp-to-edge linear sampler: each downsample reads only the level above,
        // and edge clamping avoids wrapping the border in during the box filter.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_mipmap_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            shader,
            bind_group_layout,
            pipeline_layout,
            sampler,
            pipelines: HashMap::new(),
        }
    }

    /// Fetch (building on first use) the blit pipeline targeting `format`.
    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let shader = &self.shader;
        let layout = &self.pipeline_layout;
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("review_mipmap_pipeline"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_fullscreen"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_downsample"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview: None,
                cache: None,
            })
        })
    }

    /// Generate mip levels `1..mip_level_count` of `texture` (whose base level is
    /// already filled) by successive downsample blits. The texture must carry
    /// `RENDER_ATTACHMENT` usage and at least `mip_level_count` levels. A no-op when
    /// `mip_level_count <= 1`.
    pub(crate) fn generate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        mip_level_count: u32,
    ) {
        if mip_level_count <= 1 {
            return;
        }
        // Snapshot the pipeline first to end the &mut borrow before the loop builds
        // per-level views/bind groups off &self.
        let pipeline = self.pipeline(device, format).clone();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("review_mipmap_encoder"),
        });

        for level in 1..mip_level_count {
            let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("review_mipmap_src_view"),
                base_mip_level: level - 1,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("review_mipmap_dst_view"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("review_mipmap_bind_group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("review_mipmap_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }
}

/// Number of mip levels in a full chain for a `width`×`height` texture:
/// `floor(log2(max(w, h))) + 1`, i.e. down to the 1×1 level.
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mipmap_shader_validates() {
        let module =
            naga::front::wgsl::parse_str(super::MIPMAP_SHADER).expect("mipmap.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("mipmap.wgsl should validate");
    }

    #[test]
    fn mip_level_count_covers_full_chain() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 2), 2);
        assert_eq!(mip_level_count(256, 256), 9);
        // Non-square: driven by the larger dimension.
        assert_eq!(mip_level_count(1024, 16), 11);
        assert_eq!(mip_level_count(640, 480), 10);
    }
}
