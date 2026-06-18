//! Final model-wireframe overlay. Unlike the debug line pipeline in `scene.rs`,
//! this pass runs after post processing and expands mesh edges into camera-facing
//! triangle ribbons, so the overlay can have configurable thickness and avoid
//! SSAO / bloom / tone-map effects.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::scene::{EGUI_DEPTH_FORMAT, EGUI_MSAA_SAMPLE_COUNT, SceneVertex};
use crate::{CameraProjection, OrbitCamera};

const WIREFRAME_SHADER: &str = include_str!("wireframe.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WireframeUniforms {
    projection: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    /// x = 1 / framebuffer width, y = 1 / framebuffer height,
    /// z = active thickness, w = view-Z occlusion bias.
    params: [f32; 4],
    /// x = use world units, y = occlusion enabled, z/w unused.
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct WireframeSegment {
    start: [f32; 4],
    end: [f32; 4],
    color: [f32; 4],
}

impl WireframeSegment {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

pub(crate) struct WireframeOverlayPass {
    pub(crate) pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl WireframeOverlayPass {
    pub(crate) fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_wireframe_overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(WIREFRAME_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_wireframe_overlay_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_wireframe_overlay_uniform_buffer"),
            size: std::mem::size_of::<WireframeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_wireframe_overlay_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("review_wireframe_overlay_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[WireframeSegment::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: EGUI_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: EGUI_MSAA_SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            uniform_buffer,
        }
    }

    pub(crate) fn create_bind_group(
        &self,
        device: &wgpu::Device,
        gbuffer_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_wireframe_overlay_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(gbuffer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_uniform(
        &self,
        queue: &wgpu::Queue,
        camera: OrbitCamera,
        projection_mode: CameraProjection,
        width: u32,
        height: u32,
        screen_thickness: f32,
        world_thickness: f32,
        use_world_units: bool,
        occlusion_enabled: bool,
    ) {
        let active_thickness = if use_world_units {
            world_thickness.max(0.0001)
        } else {
            screen_thickness.max(1.0)
        };
        let occlusion_bias = (camera.scene_radius.max(1e-3) * 0.001).max(1e-5);
        let uniforms = WireframeUniforms {
            projection: camera.projection_matrix(projection_mode).to_cols_array_2d(),
            view: camera.view_matrix().to_cols_array_2d(),
            params: [
                1.0 / width.max(1) as f32,
                1.0 / height.max(1) as f32,
                active_thickness,
                occlusion_bias,
            ],
            flags: [
                if use_world_units { 1 } else { 0 },
                if occlusion_enabled { 1 } else { 0 },
                0,
                0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}

pub(crate) fn create_wireframe_segment_buffer(
    device: &wgpu::Device,
    line_vertices: &[SceneVertex],
) -> (wgpu::Buffer, u32) {
    let placeholder_segment = [WireframeSegment {
        start: [0.0, 0.0, 0.0, 1.0],
        end: [0.0, 0.0, 0.0, 1.0],
        color: [0.0, 0.0, 0.0, 0.0],
    }];
    let segments: Vec<_> = line_vertices
        .chunks_exact(2)
        .map(|edge| WireframeSegment {
            start: [
                edge[0].position[0],
                edge[0].position[1],
                edge[0].position[2],
                1.0,
            ],
            end: [
                edge[1].position[0],
                edge[1].position[1],
                edge[1].position[2],
                1.0,
            ],
            color: edge[0].color,
        })
        .collect();

    let contents = if segments.is_empty() {
        bytemuck::cast_slice(&placeholder_segment)
    } else {
        bytemuck::cast_slice(&segments)
    };

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_wireframe_overlay_segment_buffer"),
        contents,
        usage: wgpu::BufferUsages::VERTEX,
    });

    (vertex_buffer, segments.len() as u32)
}
