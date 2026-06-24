//! The scene's GPU resource cache implementation. The [`SceneResources`] struct
//! lives in the parent module (so the callback can read its fields); this module
//! owns its construction and the sync/update/encode methods that reconcile the
//! pipelines, targets, buffers and bind groups with the per-frame inputs
//! (build-on-change / free-on-off, invariant 3).

use wgpu::util::DeviceExt;

use review_model::ModelData;

use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_mesh, scene_lines, selection_geometry,
    uv_fill_triangles, uv_grid_lines, uv_wireframe_lines, vertex_normal_lines, visible_geometry,
    wireframe_lines,
};
use crate::gtao::GtaoPass;
use crate::ibl::{IblResources, PREFILTER_MAX_LOD};
use crate::material::{MaterialState, MaterialTable, material_layout};
use crate::post::PostPass;
use crate::selection::SelectionView;
use crate::targets::SceneTargets;
use crate::{
    ActiveMaterial, AntiAliasing, CameraProjection, EnvironmentSettings, OrbitCamera,
    SceneDebugOptions, ShadingMode, UvCamera, UvShadingMode,
};

use super::buffers::{
    build_gtao_targets, build_gtao_textures, create_checker_bind_group, create_index_buffer,
    create_line_buffer, create_mesh_buffers, fullscreen_pass,
};
use super::gpu_types::{SHADER, SceneUniforms, shading_mode_value, vertex_color_value};
use super::pipelines::{ScenePipelines, build_line_pipeline};
use super::{BuildStage, SceneResources};

/// The three derived line views, used to address one for freeing.
#[derive(Debug, Clone, Copy)]
enum LineView {
    Wireframe,
    BoundingBox,
    FaceNormals,
    VertexNormals,
}

impl SceneResources {
    /// Build only the resources the grid-only first frame needs — the cheap
    /// "core" of [`SceneResources`] (Phase B). The heavier deferred scene
    /// pipelines and the GTAO effect pass are built afterwards, one stage
    /// per frame, by [`Self::advance_build`] (driven across frames by `app`'s
    /// startup warmup); until each lands, the matching draw / effect is skipped.
    /// IBL stays in the core (a cheap baked-map upload, not a precompute), so the
    /// shaded look is correct the instant a model appears.
    pub(super) fn new_core(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        // Startup timing: this runs once, on the first frame. With the deferred
        // build (Phase B) it is now only the *core* cost — shader + line pipeline +
        // post composite + the IBL baked-map upload + checker decode — not the full
        // scene-pipeline + GTAO set, which `advance_build` compiles over the
        // next frames. Logged via `tracing::info!` (set RUST_LOG=info).
        let build_start = std::time::Instant::now();

        let shader_start = std::time::Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("review_scene_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let shader_module_ms = shader_start.elapsed().as_secs_f64() * 1000.0;

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
            include_bytes!("../../../../assets/textures/T_UV_Checker_BW.png"),
            "review_scene_checker_greyscale",
        );
        let checker_bind_group_color = create_checker_bind_group(
            device,
            queue,
            &checker_layout,
            &checker_sampler,
            include_bytes!("../../../../assets/textures/T_UV_Checker_CLR.png"),
            "review_scene_checker_color",
        );

        // The IBL maps occupy bind group 2; its layout is part of the shared
        // pipeline layout, so every scene pipeline can sample the environment.
        let ibl_layout = IblResources::scene_layout(device);
        let ibl_start = std::time::Instant::now();
        let ibl = IblResources::from_baked(
            device,
            queue,
            &ibl_layout,
            EnvironmentSettings::default().map,
        );
        let ibl_ms = ibl_start.elapsed().as_secs_f64() * 1000.0;

        // The editable per-material uniforms occupy bind group 3; its layout joins
        // the shared pipeline layout so every scene pipeline can read a material.
        let material_layout = material_layout(device);
        let material_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let material_table =
            MaterialTable::new(device, queue, &material_layout, material_alignment);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_scene_pipeline_layout"),
            bind_group_layouts: &[
                &uniform_layout,
                &checker_layout,
                &ibl_layout,
                &material_layout,
            ],
            push_constant_ranges: &[],
        });

        // Core pipeline: only the line pipeline (the grid-only first frame draws
        // with it). The deferred scene pipelines (mesh / skybox / UV-fill /
        // selection-fill + GTAO G-buffer) start at the default MSAA level and are
        // built by `advance_build`, then rebuilt by `sync_anti_aliasing` on a level
        // change (the sample count is baked into a pipeline at creation).
        let scene_sample_count = AntiAliasing::default().msaa.sample_count();
        let line_start = std::time::Instant::now();
        let line_pipeline =
            build_line_pipeline(device, &pipeline_layout, &shader, scene_sample_count);
        let line_pipeline_ms = line_start.elapsed().as_secs_f64() * 1000.0;

        // Offscreen targets + the composite pass. The GTAO *textures* are built
        // here (the composite binds them from frame 1), but the GTAO effect *pass*
        // — the expensive shader/pipeline compile — is deferred to `advance_build`.
        // Targets start at 1x1 and are recreated at the real framebuffer size on the
        // first `prepare` (`sync_anti_aliasing`); the post pass draws into egui's
        // `output_format`.
        let targets = SceneTargets::new(device, queue, 1, 1, scene_sample_count);
        let post_start = std::time::Instant::now();
        let post = PostPass::new(device, output_format);
        let post_ms = post_start.elapsed().as_secs_f64() * 1000.0;
        let (gtao_gbuffer_view, gtao_depth_view, gtao_raw_view, gtao_blur_view) =
            build_gtao_textures(device, queue, &targets);
        let post_bind_group = post.create_bind_group(
            device,
            targets.sampled_view(),
            &gtao_blur_view,
            targets.sampled_ambient_view(),
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
        let uv_grid_vertices = uv_grid_lines();
        let (uv_grid_vertex_buffer, uv_grid_vertex_count) =
            create_line_buffer(device, &uv_grid_vertices);
        let (uv_wireframe_vertex_buffer, uv_wireframe_vertex_count) =
            create_line_buffer(device, &[]);
        let (uv_fill_vertex_buffer, uv_fill_vertex_count) = create_line_buffer(device, &[]);
        let (selection_index_buffer, selection_index_count) = create_index_buffer(device, &[]);
        let (visible_index_buffer, visible_index_count) = create_index_buffer(device, &[]);

        tracing::info!(
            total_ms = build_start.elapsed().as_secs_f64() * 1000.0,
            shader_module_ms,
            line_pipeline_ms,
            post_ms,
            ibl_ms,
            "SceneResources::new_core (core first-frame GPU build; scene pipelines + GTAO deferred)"
        );

        Self {
            output_format,
            scene_sample_count,
            shader,
            pipeline_layout,
            ibl_layout,
            ibl,
            material_layout,
            material_alignment,
            material_table,
            material_ranges: Vec::new(),
            // Sentinel distinct from any real revision so the first 3D frame uploads
            // the (initially fallback-only) table.
            material_revision: u64::MAX,
            selection_index_buffer,
            selection_ranges: Vec::new(),
            selection_index_count,
            selection_baked: None,
            visible_index_buffer,
            visible_ranges: Vec::new(),
            visible_index_count,
            visible_active: false,
            visibility_baked: None,
            targets,
            post,
            post_bind_group,
            // The GTAO effect pass + its bind groups are deferred to `advance_build`
            // (Phase B); only its textures are built in the core.
            gtao: None,
            gtao_gbuffer_view,
            gtao_depth_view,
            gtao_raw_view,
            gtao_blur_view,
            gtao_bind_group: None,
            gtao_blur_bind_group: None,
            model_revision: u64::MAX,
            mesh_uv_channel: 0,
            // The deferred scene pipelines (built by `advance_build`); the core
            // first frame draws with `line_pipeline` only.
            scene_pipelines: None,
            line_pipeline,
            build_stage: BuildStage::ScenePipelines,
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
            uv_grid_vertex_buffer,
            uv_grid_vertex_count,
            uv_wireframe_vertex_buffer,
            uv_wireframe_vertex_count,
            uv_baked: None,
            uv_fill_vertex_buffer,
            uv_fill_vertex_count,
            uv_fill_baked: None,
        }
    }

    /// Advance the deferred GPU-resource build one stage (Phase B), compiling the
    /// pipelines / passes that [`Self::new_core`] skipped so the grid-only first
    /// frame stays cheap. Called once per frame from the scene callback's `prepare`
    /// (after the frame that created the core), driven across frames by `app`'s
    /// startup warmup. Returns whether more stages remain; a no-op once `Done`.
    ///
    /// Each stage reuses the existing builders verbatim, so once warm the resources
    /// are identical to a one-shot build — only *when* they are built differs.
    pub(super) fn advance_build(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        match self.build_stage {
            BuildStage::ScenePipelines => {
                // The five MSAA-dependent scene pipelines + the GTAO G-buffer, at
                // the sample count `sync_anti_aliasing` settled the targets on. A
                // CLI/file-association mesh needs these to draw, so they build first.
                self.scene_pipelines = Some(ScenePipelines::build(
                    device,
                    &self.pipeline_layout,
                    &self.shader,
                    self.scene_sample_count,
                ));
                self.build_stage = BuildStage::Gtao;
                true
            }
            BuildStage::Gtao => {
                // GTAO pass + its bind groups (recreates the AO / G-buffer textures
                // at the current size, so rebuild `post_bind_group` afterwards — it
                // samples `gtao_blur_view`).
                let gtao = GtaoPass::new(device);
                let (gbuffer, depth, raw, blur, gtao_bg, blur_bg) =
                    build_gtao_targets(device, queue, &self.targets, &gtao);
                self.gtao_gbuffer_view = gbuffer;
                self.gtao_depth_view = depth;
                self.gtao_raw_view = raw;
                self.gtao_blur_view = blur;
                self.gtao_bind_group = Some(gtao_bg);
                self.gtao_blur_bind_group = Some(blur_bg);
                self.gtao = Some(gtao);

                // The composite samples the just-recreated gtao_blur texture —
                // repoint its bind group at it.
                self.post_bind_group = self.post.create_bind_group(
                    device,
                    self.targets.sampled_view(),
                    &self.gtao_blur_view,
                    self.targets.sampled_ambient_view(),
                );
                self.build_stage = BuildStage::Done;
                true
            }
            BuildStage::Done => false,
        }
    }

    /// Bring the offscreen targets, scene pipelines and composite uniform in line
    /// with the framebuffer size + the chosen antialiasing. Targets are recreated
    /// when the size or MSAA level changes; the scene pipelines are rebuilt only
    /// when the MSAA level changes (their sample count is baked at creation).
    /// Steady-state frames (unchanged size/level) allocate nothing.
    pub(super) fn sync_anti_aliasing(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        anti_aliasing: AntiAliasing,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        // The master AA toggle collapses to single-sample when off; the panel's
        // stored level is honoured only while it is on.
        let sample_count = anti_aliasing.effective_sample_count();

        if self.scene_sample_count != sample_count {
            // The line pipeline is core (always present); rebuild it for the new
            // level. The deferred scene pipelines rebuild their MSAA-dependent
            // subset only if already built — otherwise the `ScenePipelines` build
            // stage picks up the updated `scene_sample_count` set below.
            self.line_pipeline =
                build_line_pipeline(device, &self.pipeline_layout, &self.shader, sample_count);
            if let Some(scene_pipelines) = &mut self.scene_pipelines {
                scene_pipelines.rebuild_for_msaa(
                    device,
                    &self.pipeline_layout,
                    &self.shader,
                    sample_count,
                );
            }
            self.scene_sample_count = sample_count;
        }

        let targets_stale = self.targets.width != width
            || self.targets.height != height
            || self.targets.sample_count != sample_count;
        if targets_stale {
            self.targets = SceneTargets::new(device, queue, width, height, sample_count);

            // The GTAO *textures* always exist (the composite binds them); recreate
            // them at the new size. Their pass bind groups are rebuilt only once the
            // deferred GTAO pass is built (Phase B) — until then the composite treats
            // AO as inactive, so the cleared textures are never sampled meaningfully.
            let (gtao_gbuffer_view, gtao_depth_view, gtao_raw_view, gtao_blur_view) =
                build_gtao_textures(device, queue, &self.targets);
            self.gtao_gbuffer_view = gtao_gbuffer_view;
            self.gtao_depth_view = gtao_depth_view;
            self.gtao_raw_view = gtao_raw_view;
            self.gtao_blur_view = gtao_blur_view;
            if let Some(gtao) = &self.gtao {
                self.gtao_bind_group = Some(gtao.occlusion_bind_group(
                    device,
                    &self.gtao_gbuffer_view,
                    &self.gtao_blur_view,
                    "review_gtao_bg",
                ));
                self.gtao_blur_bind_group = Some(gtao.blur_bind_group(
                    device,
                    &self.gtao_gbuffer_view,
                    &self.gtao_raw_view,
                    "review_gtao_blur_bg",
                ));
            }

            // The composite reads the resolved color, the (cleared-or-blurred) AO,
            // and the ambient radiance — all just recreated.
            self.post_bind_group = self.post.create_bind_group(
                device,
                self.targets.sampled_view(),
                &self.gtao_blur_view,
                self.targets.sampled_ambient_view(),
            );
        }
    }

    /// Render a single-sample, mesh-only normal/depth buffer for GTAO. This avoids
    /// MSAA resolve averaging view-space normals/Z across geometry edges before
    /// the occlusion and bilateral blur passes read them.
    pub(super) fn encode_gtao_gbuffer(&self, encoder: &mut wgpu::CommandEncoder) {
        // The G-buffer pipeline is part of the deferred `ScenePipelines` (Phase B);
        // it lands before GTAO is ever active (the GTAO pass builds in a later
        // stage), so this guard is defensive.
        let Some(scene_pipelines) = &self.scene_pipelines else {
            return;
        };
        // Match the shaded mesh draw's visibility: a hidden mesh casts no AO. Solo
        // is left out here (as before), so only the per-mesh hide filters the AO.
        let (index_buffer, index_count) = if self.visible_active {
            (&self.visible_index_buffer, self.visible_index_count)
        } else {
            (&self.mesh_index_buffer, self.mesh_index_count)
        };
        if index_count == 0 {
            return;
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("review_gtao_gbuffer_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.gtao_gbuffer_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.gtao_depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(1, &self.checker_bind_group_greyscale, &[]);
        render_pass.set_bind_group(2, &self.ibl.bind_group, &[]);
        // group 3 (material) must be bound to satisfy the shared pipeline layout;
        // the G-buffer pass writes only normals/depth, so the all-fallback bind
        // group is fine. One draw over the whole index buffer (material irrelevant).
        render_pass.set_bind_group(3, self.material_table.fallback_bind_group(), &[]);
        render_pass.set_pipeline(&scene_pipelines.gtao_gbuffer);
        render_pass.set_vertex_buffer(0, self.mesh_vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..index_count, 0, 0..1);
    }

    /// Run the GTAO passes on egui's encoder, after the scene pass: the occlusion
    /// pass reads the single-sample G-buffer into `raw`, then the bilateral blur
    /// denoises `raw` → `blur`, which the composite (`post`) samples. Uses the
    /// shared fullscreen-pass helper (single color attachment, no depth). Only
    /// called when GTAO is active.
    pub(super) fn encode_gtao(&self, encoder: &mut wgpu::CommandEncoder) {
        // The deferred GTAO pass + its bind groups land together (Phase B); guarded
        // defensively (the composite treats AO as inactive until they exist).
        let (Some(gtao), Some(gtao_bg), Some(blur_bg)) = (
            &self.gtao,
            &self.gtao_bind_group,
            &self.gtao_blur_bind_group,
        ) else {
            return;
        };
        fullscreen_pass(
            encoder,
            &gtao.gtao_pipeline,
            gtao_bg,
            &self.gtao_raw_view,
            "review_gtao_pass",
        );
        fullscreen_pass(
            encoder,
            &gtao.blur_pipeline,
            blur_bg,
            &self.gtao_blur_view,
            "review_gtao_blur_pass",
        );
    }

    pub(super) fn update_camera(
        &self,
        queue: &wgpu::Queue,
        camera: OrbitCamera,
        projection_mode: CameraProjection,
        debug_options: SceneDebugOptions,
        environment: EnvironmentSettings,
        selection_color: [f32; 4],
    ) {
        let view_projection = camera.view_projection(projection_mode);
        let uniforms = SceneUniforms {
            view_projection: view_projection.to_cols_array_2d(),
            inv_view_projection: view_projection.inverse().to_cols_array_2d(),
            render_options: [
                shading_mode_value(debug_options.shading_mode),
                if debug_options.active_material == ActiveMaterial::UvChecker {
                    1.0
                } else {
                    0.0
                },
                debug_options.uv_checker_tiling.max(1) as f32,
                vertex_color_value(debug_options),
            ],
            camera_position: camera.eye_position().extend(0.0).to_array(),
            env_params: [
                if environment.ibl_enabled { 1.0 } else { 0.0 },
                environment.intensity.max(0.0),
                if environment.show_background {
                    1.0
                } else {
                    0.0
                },
                PREFILTER_MAX_LOD,
            ],
            projection_params: [
                if matches!(projection_mode, CameraProjection::Orthographic) {
                    1.0
                } else {
                    0.0
                },
                // Environment yaw (radians) for the IBL / skybox sample rotation.
                environment.rotation_degrees.to_radians(),
                0.0,
                0.0,
            ],
            view: camera.view_matrix().to_cols_array_2d(),
            selection_color,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Reload the IBL maps when the chosen environment changes. A pure upload of
    /// the baked maps (no GPU precompute); a no-op when the map is unchanged, so
    /// steady-state frames pay nothing.
    pub(super) fn sync_environment(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        environment: EnvironmentSettings,
    ) {
        if self.ibl.environment != environment.map {
            self.ibl = IblResources::from_baked(device, queue, &self.ibl_layout, environment.map);
        }
    }

    /// Rebuild the steady-state mesh for a new model and free every derived line
    /// view (the views are rebuilt on demand by [`sync_line_views`] for whichever
    /// toggles are on). Per invariant 3 the steady-state shaded view then holds
    /// zero derived buffers.
    pub(super) fn update_model(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        model_revision: u64,
        debug_options: SceneDebugOptions,
    ) {
        let (mesh_vertices, mesh_indices, mesh_ranges) =
            model_mesh(model, debug_options.uv_channel);
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &mesh_vertices, &mesh_indices);

        self.mesh_vertex_buffer = mesh_vertex_buffer;
        self.mesh_index_buffer = mesh_index_buffer;
        self.mesh_index_count = mesh_index_count;
        self.material_ranges = mesh_ranges;
        self.model_revision = model_revision;
        self.mesh_uv_channel = debug_options.uv_channel;

        // Drop the previous model's derived geometry; `sync_line_views` rebuilds
        // whatever is currently switched on.
        self.free_line_view(device, LineView::Wireframe);
        self.free_line_view(device, LineView::BoundingBox);
        self.free_line_view(device, LineView::FaceNormals);
        self.free_line_view(device, LineView::VertexNormals);

        // The visibility filter's node indices belong to the previous model; drop
        // it (the app clears its hidden set on load, so `sync_visibility` rebuilds
        // only if the new model has meshes hidden).
        self.free_visibility(device);
    }

    /// Build-on-demand / free-on-off for the three derived line views. A view's
    /// buffer is (re)built when its toggle is on and its baked params drift from
    /// the current options, and freed back to a placeholder when its toggle is
    /// off. Unchanged views are left untouched (no per-frame rebuild).
    pub(super) fn sync_line_views(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        debug_options: SceneDebugOptions,
        hidden_meshes: &[u32],
    ) {
        let wireframe_on = debug_options.wireframe_overlay
            || matches!(debug_options.shading_mode, ShadingMode::Wireframe);
        // The wireframe rebuilds when its color *or* the Outliner's hidden set
        // drifts, so edges of a hidden mesh disappear with the mesh itself.
        let want_wireframe =
            wireframe_on.then(|| (debug_options.wireframe_color, hidden_meshes.to_vec()));
        if self.wireframe_baked != want_wireframe {
            let (buffer, count) = match &want_wireframe {
                Some((color, hidden)) => {
                    create_line_buffer(device, &wireframe_lines(model, *color, hidden))
                }
                None => create_line_buffer(device, &[]),
            };
            self.wireframe_line_vertex_buffer = buffer;
            self.wireframe_line_vertex_count = count;
            self.wireframe_baked = want_wireframe;
        }

        let want_bounding_box = debug_options.show_bounding_box.then(|| {
            // The hidden set only affects the box in "visible only" mode, so leave
            // it out of the bake key otherwise — toggling a mesh's visibility then
            // never rebuilds the (identical) whole-model box.
            let visible_only = debug_options.bounding_box_visible_only;
            let hidden = if visible_only {
                hidden_meshes.to_vec()
            } else {
                Vec::new()
            };
            (debug_options.bounding_box_color, visible_only, hidden)
        });
        if self.bounding_box_baked != want_bounding_box {
            let (buffer, count) = match &want_bounding_box {
                // In "visible only" mode the box wraps just the unhidden geometry;
                // otherwise it wraps the whole model. Either way it's empty when no
                // bounds remain (every mesh hidden, or an empty model).
                Some((color, visible_only, hidden)) => {
                    let bounds = if *visible_only {
                        model.visible_bounds(hidden)
                    } else {
                        model.bounds
                    };
                    match bounds {
                        Some(bounds) => {
                            create_line_buffer(device, &bounding_box_lines(bounds, *color))
                        }
                        None => create_line_buffer(device, &[]),
                    }
                }
                None => create_line_buffer(device, &[]),
            };
            self.bounding_box_vertex_buffer = buffer;
            self.bounding_box_vertex_count = count;
            self.bounding_box_baked = want_bounding_box;
        }

        let want_face = debug_options.face_normals.then(|| {
            (
                debug_options.face_normal_length,
                debug_options.face_normal_color,
                hidden_meshes.to_vec(),
            )
        });
        if self.face_baked != want_face {
            let (buffer, count) = match &want_face {
                Some((length, color, hidden)) => {
                    create_line_buffer(device, &face_normal_lines(model, *length, *color, hidden))
                }
                None => create_line_buffer(device, &[]),
            };
            self.face_normal_vertex_buffer = buffer;
            self.face_normal_vertex_count = count;
            self.face_baked = want_face;
        }

        let want_vertex = debug_options.vertex_normals.then(|| {
            (
                debug_options.vertex_normal_length,
                debug_options.vertex_normal_color,
                hidden_meshes.to_vec(),
            )
        });
        if self.vertex_baked != want_vertex {
            let (buffer, count) = match &want_vertex {
                Some((length, color, hidden)) => {
                    create_line_buffer(device, &vertex_normal_lines(model, *length, *color, hidden))
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
        match view {
            LineView::Wireframe => {
                let (buffer, count) = create_line_buffer(device, &[]);
                self.wireframe_line_vertex_buffer = buffer;
                self.wireframe_line_vertex_count = count;
                self.wireframe_baked = None;
            }
            LineView::BoundingBox => {
                let (buffer, count) = create_line_buffer(device, &[]);
                self.bounding_box_vertex_buffer = buffer;
                self.bounding_box_vertex_count = count;
                self.bounding_box_baked = None;
            }
            LineView::FaceNormals => {
                let (buffer, count) = create_line_buffer(device, &[]);
                self.face_normal_vertex_buffer = buffer;
                self.face_normal_vertex_count = count;
                self.face_baked = None;
            }
            LineView::VertexNormals => {
                let (buffer, count) = create_line_buffer(device, &[]);
                self.vertex_normal_vertex_buffer = buffer;
                self.vertex_normal_vertex_count = count;
                self.vertex_baked = None;
            }
        }
    }

    pub(super) fn update_mesh_channel(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        uv_channel: u32,
    ) {
        let (mesh_vertices, mesh_indices, mesh_ranges) = model_mesh(model, uv_channel);
        let (mesh_vertex_buffer, mesh_index_buffer, mesh_index_count) =
            create_mesh_buffers(device, &mesh_vertices, &mesh_indices);
        self.mesh_vertex_buffer = mesh_vertex_buffer;
        self.mesh_index_buffer = mesh_index_buffer;
        self.mesh_index_count = mesh_index_count;
        // The index reordering depends only on `tri_material`, so the ranges are
        // unchanged by a UV-channel switch — but reassign them to stay in lockstep
        // with the freshly rebuilt index buffer.
        self.material_ranges = mesh_ranges;
        self.mesh_uv_channel = uv_channel;
    }

    /// Bring the material table in line with the current editable values. Rebuilds
    /// the table (buffer + bind group) when the material count changes (a new
    /// model), re-uploads when an edit bumped the revision, and otherwise does
    /// nothing — so steady-state frames pay nothing (the same cadence as
    /// `sync_environment`).
    pub(super) fn sync_materials(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        materials: &[MaterialState],
        material_revision: u64,
    ) {
        let count_changed = self.material_table.material_count() != materials.len();
        if count_changed || self.material_revision != material_revision {
            self.material_table.sync(
                device,
                queue,
                &self.material_layout,
                self.material_alignment,
                materials,
            );
            self.material_revision = material_revision;
        }
    }

    /// Build (or free) the selected-triangle index buffer (the solo isolate list +
    /// the highlight flash's fill source) when the Outliner selection changes. Like
    /// `sync_line_views`, the buffer exists only while a selection is active and is
    /// rebuilt when the model / selection drifts (invariant 3). The highlight color
    /// and flash fade ride in the uniform, so they don't trigger a rebuild — only a
    /// change of *what* is selected does. Steady-state frames do nothing.
    pub(super) fn sync_selection(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        model_revision: u64,
        view: SelectionView,
        hidden: &[u32],
    ) {
        let want = view
            .selection
            .is_active()
            .then(|| (model_revision, view.selection, hidden.to_vec()));
        if self.selection_baked == want {
            return;
        }
        match &want {
            // The selection resolved to no usable geometry (e.g. the model lacks the
            // parallel arrays, or every selected mesh is hidden) -> clear; otherwise
            // upload the visible selected indices.
            Some((_, selection, hidden)) => match selection_geometry(model, *selection, hidden) {
                Some((indices, ranges)) => {
                    let (index_buffer, index_count) = create_index_buffer(device, &indices);
                    self.selection_index_buffer = index_buffer;
                    self.selection_index_count = index_count;
                    self.selection_ranges = ranges;
                }
                None => self.clear_selection_buffers(device),
            },
            None => self.clear_selection_buffers(device),
        }
        self.selection_baked = want;
    }

    /// Drop the selection buffers back to placeholders and mark nothing selected.
    pub(super) fn free_selection(&mut self, device: &wgpu::Device) {
        if self.selection_baked.is_none() {
            return;
        }
        self.clear_selection_buffers(device);
        self.selection_baked = None;
    }

    /// Reset the selection index buffer to an empty placeholder (shared by
    /// `sync_selection`'s no-geometry path and `free_selection`).
    fn clear_selection_buffers(&mut self, device: &wgpu::Device) {
        let (index_buffer, index_count) = create_index_buffer(device, &[]);
        self.selection_index_buffer = index_buffer;
        self.selection_index_count = index_count;
        self.selection_ranges = Vec::new();
    }

    /// Build (or free) the per-mesh visibility draw list when the Outliner's hidden
    /// set changes. Like `sync_selection`, the filtered index buffer exists only
    /// while some mesh is hidden and is rebuilt when the model / hidden set drifts
    /// (invariant 3). `hidden` is the hidden mesh node indices; empty means
    /// everything is visible (the full mesh is drawn, no filtered buffer held).
    /// Steady-state frames do nothing.
    pub(super) fn sync_visibility(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        model_revision: u64,
        hidden: &[u32],
    ) {
        let want = (!hidden.is_empty()).then(|| (model_revision, hidden.to_vec()));
        if self.visibility_baked == want {
            return;
        }
        match &want {
            // Something is hidden: build the filtered draw list, or fall back to the
            // full mesh when the model carries no per-triangle node info.
            Some((_, hidden)) => match visible_geometry(model, hidden) {
                Some((indices, ranges)) => {
                    let (index_buffer, index_count) = create_index_buffer(device, &indices);
                    self.visible_index_buffer = index_buffer;
                    self.visible_index_count = index_count;
                    self.visible_ranges = ranges;
                    self.visible_active = true;
                }
                None => self.clear_visibility_buffers(device),
            },
            None => self.clear_visibility_buffers(device),
        }
        self.visibility_baked = want;
    }

    /// Drop the visibility buffer back to a placeholder and mark nothing hidden.
    pub(super) fn free_visibility(&mut self, device: &wgpu::Device) {
        if self.visibility_baked.is_none() && !self.visible_active {
            return;
        }
        self.clear_visibility_buffers(device);
        self.visibility_baked = None;
    }

    /// Reset the visibility index buffer to a placeholder and turn the filter off
    /// (shared by `sync_visibility`'s fall-back path and `free_visibility`).
    fn clear_visibility_buffers(&mut self, device: &wgpu::Device) {
        let (index_buffer, index_count) = create_index_buffer(device, &[]);
        self.visible_index_buffer = index_buffer;
        self.visible_index_count = index_count;
        self.visible_ranges = Vec::new();
        self.visible_active = false;
    }

    /// Build-on-demand for the UV viewport's derived buffers. The wireframe is
    /// rebuilt only when the active model or channel changes (`uv_baked`); the
    /// island fill is rebuilt when the model / channel / shading mode changes
    /// (`uv_fill_baked`) and freed in Wire mode — so panning/zooming rebuilds
    /// nothing (invariant 3).
    pub(super) fn sync_uv_view(
        &mut self,
        device: &wgpu::Device,
        model: &ModelData,
        model_revision: u64,
        channel: u32,
        shading_mode: UvShadingMode,
    ) {
        let want_wireframe = Some((model_revision, channel));
        if self.uv_baked != want_wireframe {
            let (buffer, count) = create_line_buffer(device, &uv_wireframe_lines(model, channel));
            self.uv_wireframe_vertex_buffer = buffer;
            self.uv_wireframe_vertex_count = count;
            self.uv_baked = want_wireframe;
        }

        // Wire mode draws no fill; Shaded / Islands build the triangle fill
        // (per-island coloring only in Islands mode).
        let want_fill = match shading_mode {
            UvShadingMode::Wire => None,
            UvShadingMode::Shaded | UvShadingMode::Islands => {
                Some((model_revision, channel, shading_mode))
            }
        };
        if self.uv_fill_baked != want_fill {
            let fill = match shading_mode {
                UvShadingMode::Wire => Vec::new(),
                UvShadingMode::Shaded => uv_fill_triangles(model, channel, false),
                UvShadingMode::Islands => uv_fill_triangles(model, channel, true),
            };
            let (buffer, count) = create_line_buffer(device, &fill);
            self.uv_fill_vertex_buffer = buffer;
            self.uv_fill_vertex_count = count;
            self.uv_fill_baked = want_fill;
        }
    }

    /// Free the UV wireframe + island fill back to placeholders when the UV view
    /// is off (the 3D scene is shown), so neither holds a derived allocation.
    pub(super) fn free_uv_view(&mut self, device: &wgpu::Device) {
        if self.uv_baked.is_none() && self.uv_fill_baked.is_none() {
            return;
        }
        let (wireframe_buffer, wireframe_count) = create_line_buffer(device, &[]);
        self.uv_wireframe_vertex_buffer = wireframe_buffer;
        self.uv_wireframe_vertex_count = wireframe_count;
        self.uv_baked = None;
        let (fill_buffer, fill_count) = create_line_buffer(device, &[]);
        self.uv_fill_vertex_buffer = fill_buffer;
        self.uv_fill_vertex_count = fill_count;
        self.uv_fill_baked = None;
    }

    /// Write the 2D UV camera's view-projection into the shared uniform buffer.
    /// The other uniform fields are unused by the line path (lines return their
    /// own vertex color), so they are left zeroed.
    pub(super) fn update_camera_uv(&self, queue: &wgpu::Queue, camera: UvCamera) {
        let uniforms = SceneUniforms {
            view_projection: camera.view_projection().to_cols_array_2d(),
            inv_view_projection: [[0.0; 4]; 4],
            render_options: [0.0; 4],
            camera_position: [0.0; 4],
            // The UV path never reaches the IBL / skybox code, so these are unused.
            env_params: [0.0; 4],
            projection_params: [1.0, 0.0, 0.0, 0.0],
            // No GTAO in the UV viewport, so the view matrix is unused here.
            view: [[0.0; 4]; 4],
            // No Outliner selection in the 2D UV viewport.
            selection_color: [0.0; 4],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}
