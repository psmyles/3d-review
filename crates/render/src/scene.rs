use bytemuck::{Pod, Zeroable};
use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use review_model::ModelData;
use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_mesh, scene_lines, vertex_normal_lines,
    wireframe_lines,
};
use crate::{CameraProjection, CheckerTexture, OrbitCamera, SceneDebugOptions, ShadingMode};

pub const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
pub const SCENE_SAMPLE_COUNT: u32 = 4;

/// The scene shader (shaded / unlit / wireframe / uv-checker paths). Kept in a
/// sibling `.wgsl` file but the `SceneUniforms` / `SceneVertex` layouts there
/// must track the `#[repr(C)]` structs below (invariant 11).
const SHADER: &str = include_str!("scene.wgsl");

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
            .or_insert_with(|| SceneResources::new(device, queue, self.output_format));

        if resources.output_format != self.output_format {
            *resources = SceneResources::new(device, queue, self.output_format);
        }

        if resources.model_revision != self.model_revision {
            // A new model rebuilds the steady-state mesh and resets every derived
            // line view to "not built" — they are (re)built on demand below only
            // for the views currently switched on (invariant 3).
            resources.update_model(device, &self.model, self.model_revision, self.debug_options);
        } else if resources.mesh_uv_channel != self.debug_options.uv_channel {
            // Switching UV channel only rebuilds the mesh vertex buffer's UVs;
            // the rest of the derived geometry is channel-independent.
            resources.update_mesh_channel(device, &self.model, self.debug_options.uv_channel);
        }

        // Build-on-demand / free-on-off for the derived line views: a view's
        // buffer exists only while its toggle is on, and is rebuilt live when its
        // baked length/color drifts from the current options (invariant 3).
        resources.sync_line_views(device, &self.model, self.debug_options);

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

        // group 1 (checker texture + sampler) stays bound for every draw using
        // the shared pipeline layout; only the mesh actually samples it.
        let checker_bind_group = match self.debug_options.uv_checker_texture {
            CheckerTexture::Greyscale => &resources.checker_bind_group_greyscale,
            CheckerTexture::Color => &resources.checker_bind_group_color,
        };
        render_pass.set_bind_group(1, checker_bind_group, &[]);

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

        if self.debug_options.show_bounding_box && resources.bounding_box_vertex_count > 0 {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            render_pass.set_vertex_buffer(0, resources.bounding_box_vertex_buffer.slice(..));
            render_pass.draw(0..resources.bounding_box_vertex_count, 0..1);
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
    mesh_uv_channel: u32,
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    checker_bind_group_greyscale: wgpu::BindGroup,
    checker_bind_group_color: wgpu::BindGroup,
    mesh_vertex_buffer: wgpu::Buffer,
    mesh_index_buffer: wgpu::Buffer,
    mesh_index_count: u32,
    line_vertex_buffer: wgpu::Buffer,
    line_vertex_count: u32,
    // Derived line views. Each `*_baked` is `Some(params)` while that view is
    // built and `None` while it is off (its buffer holds only a placeholder).
    // Comparing against the current options drives build / rebuild / free in
    // `sync_line_views` (invariant 3).
    wireframe_line_vertex_buffer: wgpu::Buffer,
    wireframe_line_vertex_count: u32,
    /// Color baked into the wireframe buffer, or `None` when the view is off.
    wireframe_baked: Option<[f32; 4]>,
    bounding_box_vertex_buffer: wgpu::Buffer,
    bounding_box_vertex_count: u32,
    /// Color baked into the bounding-box buffer, or `None` when the view is off.
    bounding_box_baked: Option<[f32; 4]>,
    face_normal_vertex_buffer: wgpu::Buffer,
    face_normal_vertex_count: u32,
    /// `(length_scale, color)` baked into the face-normal buffer, or `None`.
    face_baked: Option<NormalParams>,
    vertex_normal_vertex_buffer: wgpu::Buffer,
    vertex_normal_vertex_count: u32,
    /// `(length_scale, color)` baked into the vertex-normal buffer, or `None`.
    vertex_baked: Option<NormalParams>,
}

/// Baked parameters for a normal-line view: `(length_scale, color)`. Compared by
/// value each frame to decide whether the view's buffer is up to date.
type NormalParams = (f32, [f32; 4]);

/// The three derived line views, used to address one for freeing.
#[derive(Debug, Clone, Copy)]
enum LineView {
    Wireframe,
    BoundingBox,
    FaceNormals,
    VertexNormals,
}

impl SceneResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat) -> Self {
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

        let checker_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("review_scene_checker_layout"),
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

        let checker_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("review_scene_checker_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let checker_bind_group_greyscale = create_checker_bind_group(
            device,
            queue,
            &checker_layout,
            &checker_sampler,
            include_bytes!("../../../assets/textures/T_UV_Checker_BW.png"),
            "review_scene_checker_greyscale",
        );
        let checker_bind_group_color = create_checker_bind_group(
            device,
            queue,
            &checker_layout,
            &checker_sampler,
            include_bytes!("../../../assets/textures/T_UV_Checker_CLR.png"),
            "review_scene_checker_color",
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_scene_pipeline_layout"),
            bind_group_layouts: &[&uniform_layout, &checker_layout],
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
        let (bounding_box_vertex_buffer, bounding_box_vertex_count) =
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
            mesh_uv_channel: 0,
            mesh_pipeline,
            line_pipeline,
            uniform_buffer,
            uniform_bind_group,
            checker_bind_group_greyscale,
            checker_bind_group_color,
            mesh_vertex_buffer,
            mesh_index_buffer,
            mesh_index_count,
            line_vertex_buffer,
            line_vertex_count: line_vertices.len() as u32,
            wireframe_line_vertex_buffer,
            wireframe_line_vertex_count,
            wireframe_baked: None,
            bounding_box_vertex_buffer,
            bounding_box_vertex_count,
            bounding_box_baked: None,
            face_normal_vertex_buffer,
            face_normal_vertex_count,
            face_baked: None,
            vertex_normal_vertex_buffer,
            vertex_normal_vertex_count,
            vertex_baked: None,
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
                debug_options.uv_checker_tiling.max(1) as f32,
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Rebuild the steady-state mesh for a new model and free every derived line
    /// view (the views are rebuilt on demand by [`sync_line_views`] for whichever
    /// toggles are on). Per invariant 3 the steady-state shaded view then holds
    /// zero derived buffers.
    fn update_model(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        model_revision: u64,
        debug_options: SceneDebugOptions,
    ) {
        let (mesh_vertices, mesh_indices) = model_mesh(model, debug_options.uv_channel);
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &mesh_vertices, &mesh_indices);

        self.mesh_vertex_buffer = mesh_vertex_buffer;
        self.mesh_index_buffer = mesh_index_buffer;
        self.mesh_index_count = mesh_index_count;
        self.model_revision = model_revision;
        self.mesh_uv_channel = debug_options.uv_channel;

        // Drop the previous model's derived geometry; `sync_line_views` rebuilds
        // whatever is currently switched on.
        self.free_line_view(device, LineView::Wireframe);
        self.free_line_view(device, LineView::BoundingBox);
        self.free_line_view(device, LineView::FaceNormals);
        self.free_line_view(device, LineView::VertexNormals);
    }

    /// Build-on-demand / free-on-off for the three derived line views. A view's
    /// buffer is (re)built when its toggle is on and its baked params drift from
    /// the current options, and freed back to a placeholder when its toggle is
    /// off. Unchanged views are left untouched (no per-frame rebuild).
    fn sync_line_views(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        debug_options: SceneDebugOptions,
    ) {
        let wireframe_on = matches!(
            debug_options.shading_mode,
            ShadingMode::Wireframe | ShadingMode::ShadedWireframe
        );
        let want_wireframe = wireframe_on.then_some(debug_options.wireframe_color);
        if self.wireframe_baked != want_wireframe {
            let (buffer, count) = match want_wireframe {
                Some(color) => create_line_buffer(device, &wireframe_lines(model, color)),
                None => create_line_buffer(device, &[]),
            };
            self.wireframe_line_vertex_buffer = buffer;
            self.wireframe_line_vertex_count = count;
            self.wireframe_baked = want_wireframe;
        }

        let want_bounding_box = debug_options
            .show_bounding_box
            .then_some(debug_options.bounding_box_color);
        if self.bounding_box_baked != want_bounding_box {
            let (buffer, count) = match want_bounding_box {
                Some(color) => create_line_buffer(device, &bounding_box_lines(model, color)),
                None => create_line_buffer(device, &[]),
            };
            self.bounding_box_vertex_buffer = buffer;
            self.bounding_box_vertex_count = count;
            self.bounding_box_baked = want_bounding_box;
        }

        let want_face = debug_options.face_normals.then_some((
            debug_options.face_normal_length,
            debug_options.face_normal_color,
        ));
        if self.face_baked != want_face {
            let (buffer, count) = match want_face {
                Some((length, color)) => {
                    create_line_buffer(device, &face_normal_lines(model, length, color))
                }
                None => create_line_buffer(device, &[]),
            };
            self.face_normal_vertex_buffer = buffer;
            self.face_normal_vertex_count = count;
            self.face_baked = want_face;
        }

        let want_vertex = debug_options.vertex_normals.then_some((
            debug_options.vertex_normal_length,
            debug_options.vertex_normal_color,
        ));
        if self.vertex_baked != want_vertex {
            let (buffer, count) = match want_vertex {
                Some((length, color)) => {
                    create_line_buffer(device, &vertex_normal_lines(model, length, color))
                }
                None => create_line_buffer(device, &[]),
            };
            self.vertex_normal_vertex_buffer = buffer;
            self.vertex_normal_vertex_count = count;
            self.vertex_baked = want_vertex;
        }
    }

    /// Replace a derived view's buffer with an empty placeholder and mark it
    /// not-built, freeing the previous (potentially large) allocation.
    fn free_line_view(&mut self, device: &wgpu::Device, view: LineView) {
        let (buffer, count) = create_line_buffer(device, &[]);
        match view {
            LineView::Wireframe => {
                self.wireframe_line_vertex_buffer = buffer;
                self.wireframe_line_vertex_count = count;
                self.wireframe_baked = None;
            }
            LineView::BoundingBox => {
                self.bounding_box_vertex_buffer = buffer;
                self.bounding_box_vertex_count = count;
                self.bounding_box_baked = None;
            }
            LineView::FaceNormals => {
                self.face_normal_vertex_buffer = buffer;
                self.face_normal_vertex_count = count;
                self.face_baked = None;
            }
            LineView::VertexNormals => {
                self.vertex_normal_vertex_buffer = buffer;
                self.vertex_normal_vertex_count = count;
                self.vertex_baked = None;
            }
        }
    }

    fn update_mesh_channel(&mut self, device: &wgpu::Device, model: &ModelData, uv_channel: u32) {
        let (mesh_vertices, mesh_indices) = model_mesh(model, uv_channel);
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &mesh_vertices, &mesh_indices);
        self.mesh_vertex_buffer = mesh_vertex_buffer;
        self.mesh_index_buffer = mesh_index_buffer;
        self.mesh_index_count = mesh_index_count;
        self.mesh_uv_channel = uv_channel;
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

fn create_checker_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    png_bytes: &[u8],
    label: &str,
) -> wgpu::BindGroup {
    // The PNGs are baked in at build time (invariant: assets via include_bytes!),
    // so a decode failure is a packaging bug — fall back to a 1x1 white texel
    // rather than panicking inside the render callback.
    let rgba = image::load_from_memory(png_bytes)
        .map(|image| image.to_rgba8())
        .unwrap_or_else(|_| image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])));
    let (width, height) = rgba.dimensions();

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
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

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneUniforms {
    view_projection: [[f32; 4]; 4],
    render_options: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SceneVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) uv: [f32; 2],
    pub(crate) color: [f32; 4],
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
