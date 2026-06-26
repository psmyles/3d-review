//! Ground-Truth Ambient Occlusion (CLAUDE.md render roadmap, Phase 5): a
//! horizon-based occlusion pass over a single-sample view-space normal+depth
//! G-buffer, then a bilateral blur to remove per-pixel-rotation noise while
//! respecting geometry edges. The composite (`post.rs`) applies the blurred AO
//! only to ambient light.
//!
//! `GtaoPass` owns the (size-independent) pipelines, sampler and uniform; the
//! full-resolution AO ping/blur textures + the bind groups that point at them live
//! in `SceneResources` (rebuilt on resize alongside the scene targets), mirroring
//! how `PostPass` pairs with its `SceneResources` bind groups.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

const GTAO_SHADER: &str = include_str!("gtao.wgsl");

/// The AO texture format: a single 8-bit occlusion channel (0 = fully occluded,
/// 1 = open). Universally renderable + filterable, so GTAO needs no extra gate
/// beyond the G-buffer's `Rgba16Float` (which the scene/IBL already require).
pub(crate) const GTAO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Whether the adapter can run GTAO: it samples the `Rgba16Float` G-buffer
/// (filterable) and renders the `R8Unorm` AO target. Both are WebGPU-guaranteed on
/// any adapter that already renders the HDR scene, so this is effectively always
/// true — it exists to honour the capability-gate discipline (invariant 4) and to
/// let the UI disable the toggle rather than crash on an exotic adapter.
pub fn gtao_supported(adapter: &wgpu::Adapter) -> bool {
    let gbuffer = adapter
        .get_texture_format_features(crate::targets::SCENE_HDR_FORMAT)
        .flags;
    let ao = adapter.get_texture_format_features(GTAO_FORMAT).flags;
    gbuffer.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        && ao.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        && adapter
            .get_texture_format_features(GTAO_FORMAT)
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

/// GTAO uniform. `#[repr(C)]` + `Pod` to match the WGSL `GtaoUniforms` layout
/// (invariant 11).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GtaoUniforms {
    proj: [f32; 16],
    /// x = radius (view units), y = intensity, z = thickness, w = unused.
    params: [f32; 4],
    /// x = is_ortho (1.0 / 0.0); y = slice count; z = steps per slice; w unused.
    config: [f32; 4],
}

pub(crate) struct GtaoPass {
    pub(crate) gtao_pipeline: wgpu::RenderPipeline,
    pub(crate) blur_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

impl GtaoPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_gtao_shader"),
            source: wgpu::ShaderSource::Wgsl(GTAO_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_gtao_bind_group_layout"),
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
            label: Some("review_gtao_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_gtao_uniform"),
            size: std::mem::size_of::<GtaoUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_gtao_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let gtao_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_gtao",
            "review_gtao_pipeline",
        );
        let blur_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            "fs_blur",
            "review_gtao_blur_pipeline",
        );

        Self {
            gtao_pipeline,
            blur_pipeline,
            bind_group_layout,
            sampler,
            uniform,
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

    /// Write the per-frame GTAO uniform: the projection (for reconstruction +
    /// sample projection), the live radius/intensity/thickness, the slice/step
    /// counts, and the ortho flag. `radius` is already in view units (the caller
    /// scales the settings' scene-radius fraction by the live scene radius). Cheap;
    /// called every frame so the panel sliders are live.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &self,
        queue: &wgpu::Queue,
        projection: Mat4,
        is_ortho: bool,
        radius: f32,
        intensity: f32,
        thickness: f32,
        slices: u32,
        steps: u32,
    ) {
        let uniforms = GtaoUniforms {
            proj: projection.to_cols_array(),
            params: [
                radius.max(1e-4),
                intensity.max(0.0),
                thickness.clamp(0.0, 1.0),
                0.0,
            ],
            config: [
                if is_ortho { 1.0 } else { 0.0 },
                slices.max(1) as f32,
                steps.max(1) as f32,
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
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
                format: GTAO_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    /// The GTAO shader must parse + validate. The real check is GPU pipeline
    /// creation, but validating with naga here catches type / control-flow /
    /// binding mistakes without a GPU (matches the scene-shader guard).
    #[test]
    fn gtao_shader_validates() {
        let module =
            naga::front::wgsl::parse_str(super::GTAO_SHADER).expect("gtao.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("gtao.wgsl should validate");
    }
}
