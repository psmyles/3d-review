//! Screen-space ambient occlusion (CLAUDE.md render roadmap, Phase 5): an
//! occlusion pass over a single-sample view-space normal+depth G-buffer, then a
//! bilateral blur to remove per-pixel-rotation noise while respecting geometry
//! edges. The composite (`post.rs`) applies the blurred AO only to ambient light.
//!
//! `SsaoPass` owns the (size-independent) pipelines, sampler and uniform (with the
//! baked hemisphere kernel); the full-resolution AO ping/blur textures + the bind
//! groups that point at them live in `SceneResources` (rebuilt on resize alongside
//! the scene targets), mirroring how `BloomPass` / `PostPass` pair with their
//! `SceneResources` bind groups.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

const SSAO_SHADER: &str = include_str!("ssao.wgsl");

/// Number of hemisphere samples per pixel. Matches `KERNEL_SIZE` in `ssao.wgsl`.
const KERNEL_SIZE: usize = 16;

/// The AO texture format: a single 8-bit occlusion channel (0 = fully occluded,
/// 1 = open). Universally renderable + filterable, so SSAO needs no extra gate
/// beyond the G-buffer's `Rgba16Float` (which the scene/IBL already require).
pub(crate) const SSAO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Whether the adapter can run SSAO: it samples the `Rgba16Float` G-buffer
/// (filterable) and renders the `R8Unorm` AO target. Both are WebGPU-guaranteed on
/// any adapter that already renders the HDR scene, so this is effectively always
/// true — it exists to honour the capability-gate discipline (invariant 4) and to
/// let the UI disable the toggle rather than crash on an exotic adapter.
pub fn ssao_supported(adapter: &wgpu::Adapter) -> bool {
    let gbuffer = adapter
        .get_texture_format_features(crate::targets::SCENE_HDR_FORMAT)
        .flags;
    let ao = adapter.get_texture_format_features(SSAO_FORMAT).flags;
    gbuffer.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        && ao.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        && adapter
            .get_texture_format_features(SSAO_FORMAT)
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

/// SSAO uniform. `#[repr(C)]` + `Pod` to match the WGSL `SsaoUniforms` layout
/// (invariant 11).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SsaoUniforms {
    proj: [f32; 16],
    /// x = radius (view units), y = bias (view units), z = intensity, w = count.
    params: [f32; 4],
    /// x = is_ortho (1.0 / 0.0); y,z,w unused.
    config: [f32; 4],
    kernel: [[f32; 4]; KERNEL_SIZE],
}

pub(crate) struct SsaoPass {
    pub(crate) ssao_pipeline: wgpu::RenderPipeline,
    pub(crate) blur_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    kernel: [[f32; 4]; KERNEL_SIZE],
}

impl SsaoPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_ssao_shader"),
            source: wgpu::ShaderSource::Wgsl(SSAO_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_ssao_bind_group_layout"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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

        // Point-sample the G-buffer + AO so view normals / depths aren't blended
        // across edges; clamp so samples near the border don't wrap.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_ssao_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_ssao_uniform"),
            size: std::mem::size_of::<SsaoUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_ssao_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let ssao_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_ssao",
            "review_ssao_pipeline",
        );
        let blur_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_blur",
            "review_ssao_blur_pipeline",
        );

        Self {
            ssao_pipeline,
            blur_pipeline,
            bind_group_layout,
            sampler,
            uniform,
            kernel: build_kernel(),
        }
    }

    /// Bind group feeding the single-sample G-buffer into the occlusion pass.
    pub(crate) fn occlusion_bind_group(
        &self,
        device: &wgpu::Device,
        gbuffer_view: &wgpu::TextureView,
        dummy_ao_view: &wgpu::TextureView,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(gbuffer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(dummy_ao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// Bind group feeding the single-sample G-buffer and raw AO into the bilateral
    /// blur pass.
    pub(crate) fn blur_bind_group(
        &self,
        device: &wgpu::Device,
        gbuffer_view: &wgpu::TextureView,
        raw_ao_view: &wgpu::TextureView,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(gbuffer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(raw_ao_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// Write the per-frame SSAO uniform: the projection (for reconstruction +
    /// sample projection), the live radius/bias/intensity, and the ortho flag.
    /// `radius`/`bias` are already in view units (the caller scales the settings'
    /// scene-radius fractions by the live scene radius). Cheap; called every frame
    /// so the panel sliders are live.
    pub(crate) fn update(
        &self,
        queue: &wgpu::Queue,
        projection: Mat4,
        is_ortho: bool,
        radius: f32,
        bias: f32,
        intensity: f32,
    ) {
        let uniforms = SsaoUniforms {
            proj: projection.to_cols_array(),
            params: [
                radius.max(1e-4),
                bias.max(0.0),
                intensity.max(0.0),
                KERNEL_SIZE as f32,
            ],
            config: [if is_ortho { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            kernel: self.kernel,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
    }
}

/// A deterministic hemisphere kernel (tangent space, +Z), with samples packed
/// toward the origin so nearby occluders dominate. Built once; identical every
/// run (a fixed-seed LCG, so there is no per-launch flicker in the AO pattern).
fn build_kernel() -> [[f32; 4]; KERNEL_SIZE] {
    let mut state: u32 = 0x1234_5678;
    let mut rng = || {
        // Numerical Recipes LCG → float in [0, 1).
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / (1u32 << 24) as f32
    };

    let mut kernel = [[0.0_f32; 4]; KERNEL_SIZE];
    for (i, slot) in kernel.iter_mut().enumerate() {
        // Direction in the +Z hemisphere.
        let dir = Vec3::new(rng() * 2.0 - 1.0, rng() * 2.0 - 1.0, rng()).normalize_or_zero();
        // Accelerate the distribution toward the origin (more close samples).
        let t = i as f32 / KERNEL_SIZE as f32;
        let scale = 0.1 + 0.9 * t * t;
        let sample = dir * (scale * rng().max(0.05));
        *slot = [sample.x, sample.y, sample.z, 0.0];
    }
    kernel
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
                format: SSAO_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}
