//! The post / composite pass that draws the offscreen scene color into egui's
//! framebuffer behind the chrome (CLAUDE.md render roadmap, Phase 1), with an
//! optional FXAA edge-blend (Phase 2).
//!
//! Owns the fullscreen-triangle pipeline, the sampler used to read the resolved
//! scene targets, and a small uniform carrying the inverse resolution + effect
//! flags. The bind group is rebuilt by the caller whenever targets are recreated
//! (resize / MSAA change), since it references their texture views; the uniform
//! is rewritten each frame.

use bytemuck::{Pod, Zeroable};

use crate::scene::{EGUI_DEPTH_FORMAT, EGUI_MSAA_SAMPLE_COUNT};

const POST_SHADER: &str = include_str!("post.wgsl");

/// Composite-pass uniform: the inverse framebuffer resolution (texel size, for
/// FXAA neighbor taps), whether FXAA is enabled, and the bloom enable + intensity
/// for the additive bloom composite. `#[repr(C)]` + `Pod` to match the WGSL
/// `PostUniforms` layout (invariant 11).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniforms {
    inv_resolution: [f32; 2],
    fxaa_enabled: u32,
    bloom_enabled: u32,
    bloom_intensity: f32,
    ssao_enabled: u32,
    /// Whether the tone curve runs (0 = linear pass-through to sRGB).
    tonemap_enabled: u32,
    /// Which tone-map operator the shader's `apply_tonemap` switch selects.
    tonemap_op: u32,
}

pub(crate) struct PostPass {
    pub(crate) pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The blurred bloom texture, sampled with the same filtering
                // sampler (it is half-res, so it is upsampled here).
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // The blurred SSAO occlusion (R8, full-res), applied to the
                // scene's ambient light when SSAO is enabled.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Linear HDR ambient radiance that SSAO is allowed to attenuate.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        // FXAA needs bilinear taps between texels; the passthrough path samples
        // the target 1:1 (no magnify/minify), so linear is harmless there too.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_post_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_post_uniform_buffer"),
            size: std::mem::size_of::<PostUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
                format: EGUI_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            // The composite pass draws into egui's framebuffer, so its sample
            // count tracks egui's fixed MSAA — NOT the scene's (dynamic) MSAA,
            // which is already resolved away into the single-sample target below.
            multisample: wgpu::MultisampleState {
                count: EGUI_MSAA_SAMPLE_COUNT,
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
            uniform_buffer,
        }
    }

    /// (Re)build the bind group pointing at the resolved scene color, blurred
    /// bloom, blurred AO and ambient-radiance views. Called on creation and
    /// whenever the targets are recreated (resize / MSAA change), since all views
    /// are then stale.
    pub(crate) fn create_bind_group(
        &self,
        device: &wgpu::Device,
        scene_color: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
        ssao: &wgpu::TextureView,
        ambient: &wgpu::TextureView,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(ssao),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(ambient),
                },
            ],
        })
    }

    /// Write the per-frame composite uniform: the texel size (for FXAA taps), the
    /// FXAA enable flag, the bloom enable + intensity, the SSAO enable flag, and
    /// the tone-map enable + operator. Cheap; called every frame from `prepare`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_uniform(
        &self,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        fxaa: bool,
        bloom_enabled: bool,
        bloom_intensity: f32,
        ssao_enabled: bool,
        tonemap_enabled: bool,
        tonemap_op: u32,
    ) {
        let uniforms = PostUniforms {
            inv_resolution: [1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32],
            fxaa_enabled: u32::from(fxaa),
            bloom_enabled: u32::from(bloom_enabled),
            bloom_intensity: bloom_intensity.max(0.0),
            ssao_enabled: u32::from(ssao_enabled),
            tonemap_enabled: u32::from(tonemap_enabled),
            tonemap_op,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}
