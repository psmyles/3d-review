//! The post / composite pass that draws the offscreen scene color into egui's
//! framebuffer behind the chrome (CLAUDE.md render roadmap, Phase 1).
//!
//! Owns the fullscreen-triangle pipeline plus the sampler used to read the
//! resolved scene target. The bind group is rebuilt by the caller whenever the
//! target is recreated (resize), since it references the resolved texture view.
//! For Phase 1 the shader is a passthrough (see `post.wgsl`); this is the seam
//! tone mapping / bloom / FXAA slot into later.

use crate::scene::{SCENE_DEPTH_FORMAT, SCENE_SAMPLE_COUNT};

const POST_SHADER: &str = include_str!("post.wgsl");

pub(crate) struct PostPass {
    pub(crate) pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl PostPass {
    /// `output_format` is egui's framebuffer format (the pass draws into egui's
    /// render pass, so it must match and carry the same MSAA / depth state).
    pub(crate) fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_post_shader"),
            source: wgpu::ShaderSource::Wgsl(POST_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_post_bind_group_layout"),
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

        // The scene target matches the framebuffer 1:1, so the fullscreen blit
        // never magnifies/minifies; nearest would do, but linear is harmless and
        // future-proofs a differently-sized source.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_post_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_post_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("review_post_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            // egui's render pass carries a depth attachment, so a pipeline used in
            // it must declare matching depth state. The blit ignores depth: never
            // writes, always passes (it draws first, over the cleared frame).
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SCENE_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SCENE_SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                // Opaque overwrite: the resolved scene already composited every
                // overlay over the background, so the blit replaces the frame.
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// (Re)build the bind group pointing at the resolved scene color view. Called
    /// on creation and whenever the targets are recreated (resize).
    pub(crate) fn create_bind_group(
        &self,
        device: &wgpu::Device,
        scene_color: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_post_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene_color),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}
