use bytemuck::{Pod, Zeroable};
use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use review_model::ModelData;
use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::OrbitCamera;

pub const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

const SHADER: &str = r#"
struct SceneUniforms {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: SceneUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#;

#[derive(Debug, Clone)]
pub struct SceneCallback {
    camera: OrbitCamera,
    output_format: wgpu::TextureFormat,
    model: Arc<ModelData>,
    model_revision: u64,
}

impl SceneCallback {
    pub fn new(
        camera: OrbitCamera,
        output_format: wgpu::TextureFormat,
        model: Arc<ModelData>,
        model_revision: u64,
    ) -> Self {
        Self {
            camera,
            output_format,
            model,
            model_revision,
        }
    }
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<SceneResources>()
            .or_insert_with(|| SceneResources::new(device, self.output_format));

        if resources.output_format != self.output_format {
            *resources = SceneResources::new(device, self.output_format);
        }

        if resources.model_revision != self.model_revision {
            resources.update_model(device, &self.model, self.model_revision);
        }

        resources.update_camera(queue, self.camera);
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<SceneResources>() else {
            return;
        };

        if resources.mesh_index_count > 0 {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.mesh_pipeline);
            render_pass.set_vertex_buffer(0, resources.mesh_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                resources.mesh_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..resources.mesh_index_count, 0, 0..1);
        }

        render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
        render_pass.set_pipeline(&resources.line_pipeline);
        render_pass.set_vertex_buffer(0, resources.line_vertex_buffer.slice(..));
        render_pass.draw(0..resources.line_vertex_count, 0..1);
    }
}

struct SceneResources {
    output_format: wgpu::TextureFormat,
    model_revision: u64,
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    mesh_vertex_buffer: wgpu::Buffer,
    mesh_index_buffer: wgpu::Buffer,
    mesh_index_count: u32,
    line_vertex_buffer: wgpu::Buffer,
    line_vertex_count: u32,
}

impl SceneResources {
    fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_scene_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("review_scene_uniform_buffer"),
            size: std::mem::size_of::<SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_scene_uniform_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("review_scene_uniform_bind_group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_scene_pipeline_layout"),
            bind_group_layouts: &[&uniform_layout],
            push_constant_ranges: &[],
        });

        let mesh_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            output_format,
            wgpu::PrimitiveTopology::TriangleList,
            true,
            "review_scene_mesh_pipeline",
        );
        let line_pipeline = create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            output_format,
            wgpu::PrimitiveTopology::LineList,
            false,
            "review_scene_line_pipeline",
        );

        let line_vertices = scene_lines();
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &[], &[]);
        let line_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("review_scene_line_vertex_buffer"),
            contents: bytemuck::cast_slice(&line_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            output_format,
            model_revision: u64::MAX,
            mesh_pipeline,
            line_pipeline,
            uniform_buffer,
            uniform_bind_group,
            mesh_vertex_buffer,
            mesh_index_buffer,
            mesh_index_count,
            line_vertex_buffer,
            line_vertex_count: line_vertices.len() as u32,
        }
    }

    fn update_camera(&self, queue: &wgpu::Queue, camera: OrbitCamera) {
        let uniforms = SceneUniforms {
            view_projection: camera.view_projection().to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn update_model(&mut self, device: &wgpu::Device, model: &ModelData, model_revision: u64) {
        let (mesh_vertices, mesh_indices) = model_mesh(model);
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &mesh_vertices, &mesh_indices);

        self.mesh_vertex_buffer = mesh_vertex_buffer;
        self.mesh_index_buffer = mesh_index_buffer;
        self.mesh_index_count = mesh_index_count;
        self.model_revision = model_revision;
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    output_format: wgpu::TextureFormat,
    topology: wgpu::PrimitiveTopology,
    depth_write_enabled: bool,
    label: &'static str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[SceneVertex::layout()],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SCENE_DEPTH_FORMAT,
            depth_write_enabled,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    })
}

fn scene_lines() -> Vec<SceneVertex> {
    let mut vertices = Vec::new();
    let grid_extent = 12;

    for line in -grid_extent..=grid_extent {
        let strong = line % 4 == 0;
        let color = if strong {
            [0.42, 0.49, 0.54, 0.46]
        } else {
            [0.33, 0.38, 0.42, 0.28]
        };
        push_line(
            &mut vertices,
            [line as f32, 0.0, -grid_extent as f32],
            [line as f32, 0.0, grid_extent as f32],
            color,
        );
        push_line(
            &mut vertices,
            [-grid_extent as f32, 0.0, line as f32],
            [grid_extent as f32, 0.0, line as f32],
            color,
        );
    }

    push_line(
        &mut vertices,
        [-grid_extent as f32, 0.002, 0.0],
        [grid_extent as f32, 0.002, 0.0],
        [0.94, 0.23, 0.28, 1.0],
    );
    push_line(
        &mut vertices,
        [0.0, 0.004, -grid_extent as f32],
        [0.0, 0.004, grid_extent as f32],
        [0.18, 0.53, 1.0, 1.0],
    );
    vertices
}

fn model_mesh(model: &ModelData) -> (Vec<SceneVertex>, Vec<u32>) {
    let vertices = model
        .vertices
        .iter()
        .map(|vertex| SceneVertex {
            position: vertex.position.to_array(),
            color: vertex.color.to_array(),
        })
        .collect();
    (vertices, model.indices.clone())
}

fn create_mesh_buffers(
    device: &wgpu::Device,
    vertices: &[SceneVertex],
    indices: &[u32],
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let placeholder_vertex = [SceneVertex {
        position: [0.0, 0.0, 0.0],
        color: [0.0, 0.0, 0.0, 0.0],
    }];
    let placeholder_index = [0_u32];

    let vertex_contents = if vertices.is_empty() {
        bytemuck::cast_slice(&placeholder_vertex)
    } else {
        bytemuck::cast_slice(vertices)
    };
    let index_contents = if indices.is_empty() {
        bytemuck::cast_slice(&placeholder_index)
    } else {
        bytemuck::cast_slice(indices)
    };

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_mesh_vertex_buffer"),
        contents: vertex_contents,
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_mesh_index_buffer"),
        contents: index_contents,
        usage: wgpu::BufferUsages::INDEX,
    });

    (vertex_buffer, index_buffer, indices.len() as u32)
}

fn push_line(vertices: &mut Vec<SceneVertex>, start: [f32; 3], end: [f32; 3], color: [f32; 4]) {
    vertices.push(SceneVertex {
        position: start,
        color,
    });
    vertices.push(SceneVertex {
        position: end,
        color,
    });
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniforms {
    view_projection: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneVertex {
    position: [f32; 3],
    color: [f32; 4],
}

impl SceneVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}
