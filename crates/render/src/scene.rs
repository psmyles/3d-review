use bytemuck::{Pod, Zeroable};
use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use glam::Vec3;
use review_model::ModelData;
use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::{CameraProjection, OrbitCamera, SceneDebugOptions, ShadingMode};

pub const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
pub const SCENE_SAMPLE_COUNT: u32 = 4;

const SHADER: &str = r#"
struct SceneUniforms {
    view_projection: mat4x4<f32>,
    render_options: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: SceneUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.normal = input.normal;
    output.uv = input.uv;
    return output;
}

fn checker_mask(uv: vec2<f32>) -> f32 {
    let tiles = floor(uv * 8.0);
    return fract((tiles.x + tiles.y) * 0.5) * 2.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv_checker_enabled = uniforms.render_options.y > 0.5;
    let shading_mode = uniforms.render_options.x;
    let normal_length_sq = dot(input.normal, input.normal);
    var base_color = input.color.rgb;
    if (uv_checker_enabled) {
        let checker = checker_mask(input.uv);
        let dark = vec3<f32>(0.11, 0.11, 0.12);
        let light = vec3<f32>(0.83, 0.84, 0.87);
        let accent = vec3<f32>(0.22, 0.57, 0.93);
        let mix_t = smoothstep(0.0, 1.0, checker);
        base_color = mix(dark, light, mix_t);
        if (fract(input.uv.x * 8.0) < 0.04 || fract(input.uv.y * 8.0) < 0.04) {
            base_color = accent;
        }
    }
    if (normal_length_sq < 1e-6) {
        return vec4<f32>(base_color, input.color.a);
    }

    if (shading_mode < 1.5) {
        return vec4<f32>(base_color, input.color.a);
    }

    let n = normalize(input.normal);
    let light_dir = normalize(vec3<f32>(0.35, 0.82, 0.44));
    let diffuse = max(dot(n, light_dir), 0.0);
    let hemi_t = clamp(n.y * 0.5 + 0.5, 0.0, 1.0);
    let sky = vec3<f32>(0.58, 0.64, 0.72);
    let ground = vec3<f32>(0.10, 0.11, 0.13);
    let hemi = mix(ground, sky, hemi_t);
    let lighting = hemi * 0.55 + vec3<f32>(1.0, 1.0, 1.0) * (0.20 + diffuse * 0.75);
    return vec4<f32>(base_color * lighting, input.color.a);
}
"#;

#[derive(Debug, Clone)]
pub struct SceneCallback {
    camera: OrbitCamera,
    projection_mode: CameraProjection,
    output_format: wgpu::TextureFormat,
    model: Arc<ModelData>,
    model_revision: u64,
    debug_options: SceneDebugOptions,
}

impl SceneCallback {
    pub fn new(
        camera: OrbitCamera,
        projection_mode: CameraProjection,
        output_format: wgpu::TextureFormat,
        model: Arc<ModelData>,
        model_revision: u64,
        debug_options: SceneDebugOptions,
    ) -> Self {
        Self {
            camera,
            projection_mode,
            output_format,
            model,
            model_revision,
            debug_options,
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
            resources.update_model(device, &self.model, self.model_revision, self.debug_options);
        }

        resources.update_camera(queue, self.camera, self.projection_mode, self.debug_options);
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

        if resources.mesh_index_count > 0
            && !matches!(self.debug_options.shading_mode, ShadingMode::Wireframe)
        {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.mesh_pipeline);
            render_pass.set_vertex_buffer(0, resources.mesh_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                resources.mesh_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..resources.mesh_index_count, 0, 0..1);
        }

        if self.debug_options.show_grid {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            render_pass.set_vertex_buffer(0, resources.line_vertex_buffer.slice(..));
            render_pass.draw(0..resources.line_vertex_count, 0..1);
        }

        if matches!(
            self.debug_options.shading_mode,
            ShadingMode::Wireframe | ShadingMode::ShadedWireframe
        ) && resources.wireframe_line_vertex_count > 0
        {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            render_pass.set_vertex_buffer(0, resources.wireframe_line_vertex_buffer.slice(..));
            render_pass.draw(0..resources.wireframe_line_vertex_count, 0..1);
        }

        if self.debug_options.face_normals && resources.face_normal_vertex_count > 0 {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            render_pass.set_vertex_buffer(0, resources.face_normal_vertex_buffer.slice(..));
            render_pass.draw(0..resources.face_normal_vertex_count, 0..1);
        }

        if self.debug_options.vertex_normals && resources.vertex_normal_vertex_count > 0 {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            render_pass.set_vertex_buffer(0, resources.vertex_normal_vertex_buffer.slice(..));
            render_pass.draw(0..resources.vertex_normal_vertex_count, 0..1);
        }
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
    wireframe_line_vertex_buffer: wgpu::Buffer,
    wireframe_line_vertex_count: u32,
    face_normal_vertex_buffer: wgpu::Buffer,
    face_normal_vertex_count: u32,
    vertex_normal_vertex_buffer: wgpu::Buffer,
    vertex_normal_vertex_count: u32,
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
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
        let (wireframe_line_vertex_buffer, wireframe_line_vertex_count) =
            create_line_buffer(device, &[]);
        let (face_normal_vertex_buffer, face_normal_vertex_count) = create_line_buffer(device, &[]);
        let (vertex_normal_vertex_buffer, vertex_normal_vertex_count) =
            create_line_buffer(device, &[]);
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
            wireframe_line_vertex_buffer,
            wireframe_line_vertex_count,
            face_normal_vertex_buffer,
            face_normal_vertex_count,
            vertex_normal_vertex_buffer,
            vertex_normal_vertex_count,
        }
    }

    fn update_camera(
        &self,
        queue: &wgpu::Queue,
        camera: OrbitCamera,
        projection_mode: CameraProjection,
        debug_options: SceneDebugOptions,
    ) {
        let uniforms = SceneUniforms {
            view_projection: camera.view_projection(projection_mode).to_cols_array_2d(),
            render_options: [
                shading_mode_value(debug_options.shading_mode),
                if debug_options.uv_checker { 1.0 } else { 0.0 },
                0.0,
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn update_model(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        model_revision: u64,
        debug_options: SceneDebugOptions,
    ) {
        let (mesh_vertices, mesh_indices) = model_mesh(model);
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &mesh_vertices, &mesh_indices);
        let wireframe_lines = wireframe_lines(model);
        let face_normal_lines = face_normal_lines(model, debug_options);
        let vertex_normal_lines = vertex_normal_lines(model, debug_options);
        let (wireframe_line_vertex_buffer, wireframe_line_vertex_count) =
            create_line_buffer(device, &wireframe_lines);
        let (face_normal_vertex_buffer, face_normal_vertex_count) =
            create_line_buffer(device, &face_normal_lines);
        let (vertex_normal_vertex_buffer, vertex_normal_vertex_count) =
            create_line_buffer(device, &vertex_normal_lines);

        self.mesh_vertex_buffer = mesh_vertex_buffer;
        self.mesh_index_buffer = mesh_index_buffer;
        self.mesh_index_count = mesh_index_count;
        self.wireframe_line_vertex_buffer = wireframe_line_vertex_buffer;
        self.wireframe_line_vertex_count = wireframe_line_vertex_count;
        self.face_normal_vertex_buffer = face_normal_vertex_buffer;
        self.face_normal_vertex_count = face_normal_vertex_count;
        self.vertex_normal_vertex_buffer = vertex_normal_vertex_buffer;
        self.vertex_normal_vertex_count = vertex_normal_vertex_count;
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
        multisample: wgpu::MultisampleState {
            count: SCENE_SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
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
            normal: vertex.normal.to_array(),
            uv: vertex.uv.to_array(),
            color: vertex.color.to_array(),
        })
        .collect();
    (vertices, model.indices.clone())
}

fn wireframe_lines(model: &ModelData) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    for triangle in model.indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let positions = [
            model.vertices.get(a).map(|vertex| vertex.position),
            model.vertices.get(b).map(|vertex| vertex.position),
            model.vertices.get(c).map(|vertex| vertex.position),
        ];
        let [Some(a), Some(b), Some(c)] = positions else {
            continue;
        };

        push_line(
            &mut vertices,
            a.to_array(),
            b.to_array(),
            [0.96, 0.98, 1.0, 0.82],
        );
        push_line(
            &mut vertices,
            b.to_array(),
            c.to_array(),
            [0.96, 0.98, 1.0, 0.82],
        );
        push_line(
            &mut vertices,
            c.to_array(),
            a.to_array(),
            [0.96, 0.98, 1.0, 0.82],
        );
    }

    vertices
}

fn face_normal_lines(model: &ModelData, debug_options: SceneDebugOptions) -> Vec<SceneVertex> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return Vec::new();
    }

    let face_count = model
        .tri_to_face
        .iter()
        .copied()
        .max()
        .map(|max_face| max_face as usize + 1)
        .unwrap_or(triangle_count);
    let mut accum_centers = vec![Vec3::ZERO; face_count];
    let mut accum_normals = vec![Vec3::ZERO; face_count];
    let mut counts = vec![0_u32; face_count];
    let normal_length = debug_normal_length(model, debug_options.face_normal_length);

    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
        let face_index = model
            .tri_to_face
            .get(triangle_index)
            .copied()
            .unwrap_or(triangle_index as u32) as usize;
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let positions = [
            model.vertices.get(a).map(|vertex| vertex.position),
            model.vertices.get(b).map(|vertex| vertex.position),
            model.vertices.get(c).map(|vertex| vertex.position),
        ];
        let [Some(a), Some(b), Some(c)] = positions else {
            continue;
        };

        let ab = b - a;
        let ac = c - a;
        let normal = ab.cross(ac);
        if normal.length_squared() <= f32::EPSILON {
            continue;
        }

        if let (Some(center_accum), Some(normal_accum), Some(count)) = (
            accum_centers.get_mut(face_index),
            accum_normals.get_mut(face_index),
            counts.get_mut(face_index),
        ) {
            *center_accum += (a + b + c) / 3.0;
            *normal_accum += normal.normalize();
            *count += 1;
        }
    }

    let mut vertices = Vec::with_capacity(face_count * 2);
    for face_index in 0..face_count {
        let count = counts[face_index];
        if count == 0 {
            continue;
        }

        let center = accum_centers[face_index] / count as f32;
        let normal = accum_normals[face_index];
        if normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let end = center + normal.normalize() * normal_length;
        push_line(
            &mut vertices,
            center.to_array(),
            end.to_array(),
            debug_options.face_normal_color,
        );
    }

    vertices
}

fn vertex_normal_lines(model: &ModelData, debug_options: SceneDebugOptions) -> Vec<SceneVertex> {
    let normal_length = debug_normal_length(model, debug_options.vertex_normal_length);
    let mut vertices = Vec::with_capacity(model.vertices.len() * 2);

    for vertex in &model.vertices {
        if vertex.normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let start = vertex.position;
        let end = start + vertex.normal.normalize() * normal_length;
        push_line(
            &mut vertices,
            start.to_array(),
            end.to_array(),
            debug_options.vertex_normal_color,
        );
    }

    vertices
}

fn debug_normal_length(model: &ModelData, scale: f32) -> f32 {
    let size = model
        .bounds
        .map(|bounds| bounds.size())
        .unwrap_or(Vec3::splat(1.0));
    let max_extent = size.max_element().max(1.0);
    max_extent * scale.max(0.01)
}

fn create_mesh_buffers(
    device: &wgpu::Device,
    vertices: &[SceneVertex],
    indices: &[u32],
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let placeholder_vertex = [SceneVertex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
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

fn create_line_buffer(device: &wgpu::Device, vertices: &[SceneVertex]) -> (wgpu::Buffer, u32) {
    let placeholder_vertex = [SceneVertex {
        position: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color: [0.0, 0.0, 0.0, 0.0],
    }];
    let contents = if vertices.is_empty() {
        bytemuck::cast_slice(&placeholder_vertex)
    } else {
        bytemuck::cast_slice(vertices)
    };

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("review_scene_debug_line_vertex_buffer"),
        contents,
        usage: wgpu::BufferUsages::VERTEX,
    });

    (vertex_buffer, vertices.len() as u32)
}

fn push_line(vertices: &mut Vec<SceneVertex>, start: [f32; 3], end: [f32; 3], color: [f32; 4]) {
    vertices.push(SceneVertex {
        position: start,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color,
    });
    vertices.push(SceneVertex {
        position: end,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color,
    });
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniforms {
    view_projection: [[f32; 4]; 4],
    render_options: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

impl SceneVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

fn shading_mode_value(mode: ShadingMode) -> f32 {
    match mode {
        ShadingMode::Wireframe => 0.0,
        ShadingMode::Unlit => 1.0,
        ShadingMode::Shaded => 2.0,
        ShadingMode::ShadedWireframe => 3.0,
    }
}
