//! The bloom passes (CLAUDE.md render roadmap, Phase 4): a bright-pass extract
//! from the scene's linear-HDR bloom source, then a separable Gaussian blur, all
//! at half framebuffer resolution. The blurred result is added back over the
//! display-space scene by the composite pass (`post.rs`).
//!
//! `BloomPass` owns the (size-independent) pipelines, sampler and per-step uniform
//! buffers; the half-resolution ping-pong textures + the bind groups that point at
//! them live in `SceneResources` (rebuilt on resize, alongside the scene targets),
//! mirroring how `PostPass` pairs with `SceneResources::post_bind_group`.

use bytemuck::{Pod, Zeroable};

use crate::targets::SCENE_HDR_FORMAT;

const BLOOM_SHADER: &str = include_str!("bloom.wgsl");

/// Number of full separable-blur iterations (each = one horizontal + one vertical
/// pass). Two widens the glow without a multi-mip chain.
pub(crate) const BLOOM_BLUR_ITERATIONS: u32 = 2;

/// Soft-knee width for the bright-pass, as a fraction of the threshold: the band
/// over which bloom fades in rather than switching on hard. Internal (the UI
/// exposes only threshold + intensity).
const BLOOM_KNEE_FRACTION: f32 = 0.5;

/// Bloom uniform shared by the bright-pass and blur entry points. `#[repr(C)]` +
/// `Pod` to match the WGSL `BloomUniforms` layout (invariant 11). The bright-pass
/// reads `threshold`/`knee`; the blur reads `offset`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BloomUniforms {
    threshold: f32,
    knee: f32,
    offset: [f32; 2],
}

pub(crate) struct BloomPass {
    pub(crate) brightpass_pipeline: wgpu::RenderPipeline,
    pub(crate) blur_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    brightpass_uniform: wgpu::Buffer,
    blur_h_uniform: wgpu::Buffer,
    blur_v_uniform: wgpu::Buffer,
}

impl BloomPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_bloom_shader"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_bloom_bind_group_layout"),
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
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_bloom_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let make_uniform = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<BloomUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let brightpass_uniform = make_uniform("review_bloom_brightpass_uniform");
        let blur_h_uniform = make_uniform("review_bloom_blur_h_uniform");
        let blur_v_uniform = make_uniform("review_bloom_blur_v_uniform");

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_bloom_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let brightpass_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_brightpass",
            "review_bloom_brightpass_pipeline",
        );
        let blur_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_blur",
            "review_bloom_blur_pipeline",
        );

        Self {
            brightpass_pipeline,
            blur_pipeline,
            bind_group_layout,
            sampler,
            brightpass_uniform,
            blur_h_uniform,
            blur_v_uniform,
        }
    }

    /// Bind group feeding `input_view` (the scene's resolved bloom source) into the
    /// bright-pass.
    pub(crate) fn brightpass_bind_group(
        &self,
        device: &wgpu::Device,
        input_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        self.bind_group(
            device,
            input_view,
            &self.brightpass_uniform,
            "review_bloom_brightpass_bg",
        )
    }

    /// Bind group for the horizontal blur pass (reads `input_view`, uses the
    /// horizontal sampling offset).
    pub(crate) fn blur_h_bind_group(
        &self,
        device: &wgpu::Device,
        input_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        self.bind_group(
            device,
            input_view,
            &self.blur_h_uniform,
            "review_bloom_blur_h_bg",
        )
    }

    /// Bind group for the vertical blur pass.
    pub(crate) fn blur_v_bind_group(
        &self,
        device: &wgpu::Device,
        input_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        self.bind_group(
            device,
            input_view,
            &self.blur_v_uniform,
            "review_bloom_blur_v_bg",
        )
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        input_view: &wgpu::TextureView,
        uniform: &wgpu::Buffer,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// Write the bright-pass threshold (the knee is derived from it). Cheap;
    /// called each frame so the panel's threshold slider is live.
    pub(crate) fn update_threshold(&self, queue: &wgpu::Queue, threshold: f32) {
        let threshold = threshold.max(0.0);
        let uniforms = BloomUniforms {
            threshold,
            knee: (threshold * BLOOM_KNEE_FRACTION).max(1e-3),
            offset: [0.0, 0.0],
        };
        queue.write_buffer(&self.brightpass_uniform, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Write the per-axis blur sampling offsets for the current half-resolution
    /// (one texel along each axis). Called when the bloom textures are (re)sized.
    pub(crate) fn update_blur_offsets(
        &self,
        queue: &wgpu::Queue,
        half_width: u32,
        half_height: u32,
    ) {
        let texel_x = 1.0 / half_width.max(1) as f32;
        let texel_y = 1.0 / half_height.max(1) as f32;
        let h = BloomUniforms {
            threshold: 0.0,
            knee: 0.0,
            offset: [texel_x, 0.0],
        };
        let v = BloomUniforms {
            threshold: 0.0,
            knee: 0.0,
            offset: [0.0, texel_y],
        };
        queue.write_buffer(&self.blur_h_uniform, 0, bytemuck::bytes_of(&h));
        queue.write_buffer(&self.blur_v_uniform, 0, bytemuck::bytes_of(&v));
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
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
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: SCENE_HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}
