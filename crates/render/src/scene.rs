use bytemuck::{Pod, Zeroable};
use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use review_model::ModelData;
use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::bloom::{BLOOM_BLUR_ITERATIONS, BloomPass};
use crate::geometry::{
    bounding_box_lines, face_normal_lines, model_mesh, scene_lines, uv_fill_triangles,
    uv_grid_lines, uv_wireframe_lines, vertex_normal_lines, wireframe_lines,
};
use crate::ibl::{IblResources, PREFILTER_MAX_LOD};
use crate::post::PostPass;
use crate::ssao::{SSAO_FORMAT, SsaoPass};
use crate::targets::{SCENE_HDR_FORMAT, SceneTargets};
use crate::{
    ActiveMaterial, AntiAliasing, BloomSettings, CameraProjection, CheckerTexture,
    EnvironmentSettings, OrbitCamera, SceneDebugOptions, ShadingMode, SsaoSettings, UvCamera,
    UvShadingMode, VertexColorMode,
};

pub const SCENE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
/// MSAA sample count of egui's own framebuffer (the surface the composite pass
/// draws into). Fixed: it antialiases the egui chrome and is what the painter is
/// created with in `app`. Distinct from the scene's MSAA, which is dynamic
/// ([`AntiAliasing::msaa`]) and resolved to a single-sample texture before this
/// pass ever runs — so the two sample counts are deliberately independent.
pub const EGUI_MSAA_SAMPLE_COUNT: u32 = 4;

/// Background the offscreen scene target is cleared to each frame. Black in both
/// gamma and linear, so it matches the previous direct-to-egui clear regardless
/// of the target's color space. (The post pass overwrites the whole framebuffer,
/// so egui's own clear color no longer shows through in the scene region.)
const SCENE_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// The scene shader (shaded / unlit / wireframe / uv-checker paths). Kept in a
/// sibling `.wgsl` file but the `SceneUniforms` / `SceneVertex` layouts there
/// must track the `#[repr(C)]` structs below (invariant 11).
const SHADER: &str = include_str!("scene.wgsl");

/// The 2D UV viewport view: which UV channel to draw, how to shade it, and the
/// camera framing it. `Some` switches [`SceneCallback`] to the UV path (grid +
/// optional island fill + UV wireframe).
#[derive(Debug, Clone, Copy)]
struct UvView {
    camera: UvCamera,
    channel: u32,
    shading_mode: UvShadingMode,
}

#[derive(Debug, Clone)]
pub struct SceneCallback {
    camera: OrbitCamera,
    projection_mode: CameraProjection,
    output_format: wgpu::TextureFormat,
    model: Arc<ModelData>,
    model_revision: u64,
    debug_options: SceneDebugOptions,
    /// Scene MSAA level + FXAA toggle. Drives the offscreen target / scene
    /// pipeline sample count and the composite shader.
    anti_aliasing: AntiAliasing,
    /// Image-based lighting / environment selection. Drives the precomputed IBL
    /// maps, the PBR shaded path, and the optional skybox.
    environment: EnvironmentSettings,
    /// Bloom (HDR glow) settings. Drives the bloom passes + the composite add.
    bloom: BloomSettings,
    /// Screen-space ambient occlusion settings. Drives the SSAO + blur passes and
    /// the composite multiply.
    ssao: SsaoSettings,
    /// `Some` renders the 2D UV viewport instead of the 3D scene.
    uv_view: Option<UvView>,
}

impl SceneCallback {
    // Ten distinct, independent inputs (camera + projection + target + model +
    // revision + the five UI option bundles); there is no redundant pair to fold
    // away, and a params struct would only move the same values behind one name.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        camera: OrbitCamera,
        projection_mode: CameraProjection,
        output_format: wgpu::TextureFormat,
        model: Arc<ModelData>,
        model_revision: u64,
        debug_options: SceneDebugOptions,
        anti_aliasing: AntiAliasing,
        environment: EnvironmentSettings,
        bloom: BloomSettings,
        ssao: SsaoSettings,
    ) -> Self {
        Self {
            camera,
            projection_mode,
            output_format,
            model,
            model_revision,
            debug_options,
            anti_aliasing,
            environment,
            bloom,
            ssao,
            uv_view: None,
        }
    }

    /// Build a callback that renders the 2D UV viewport for `channel` framed by
    /// `camera` and shaded per `shading_mode`, instead of the 3D scene.
    pub fn new_uv(
        output_format: wgpu::TextureFormat,
        model: Arc<ModelData>,
        model_revision: u64,
        camera: UvCamera,
        channel: u32,
        shading_mode: UvShadingMode,
    ) -> Self {
        Self {
            camera: OrbitCamera::default(),
            projection_mode: CameraProjection::default(),
            output_format,
            model,
            model_revision,
            debug_options: SceneDebugOptions::default(),
            // The UV viewport keeps the default scene AA (4× MSAA, no FXAA), so
            // switching to UV mode renders exactly as it did pre-Phase-2.
            anti_aliasing: AntiAliasing::default(),
            // IBL is irrelevant to the 2D UV viewport; the skybox/IBL paths are
            // never reached there (the UV path returns before them).
            environment: EnvironmentSettings::default(),
            // Bloom is a 3D-only effect; the UV viewport never glows. Disable it so
            // the composite skips the bloom add in UV mode.
            bloom: BloomSettings {
                enabled: false,
                ..BloomSettings::default()
            },
            // SSAO is a 3D-only effect; the flat UV viewport has no depth to occlude.
            ssao: SsaoSettings {
                enabled: false,
                ..SsaoSettings::default()
            },
            uv_view: Some(UvView {
                camera,
                channel,
                shading_mode,
            }),
        }
    }
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<SceneResources>()
            .or_insert_with(|| SceneResources::new(device, queue, self.output_format));

        if resources.output_format != self.output_format {
            *resources = SceneResources::new(device, queue, self.output_format);
        }

        if let Some(uv) = self.uv_view {
            // UV viewport path: build only the UV wireframe (on demand, per
            // channel / model) and frame it with the 2D camera. The 3D mesh /
            // line views are left untouched and rebuilt when the 3D scene returns.
            resources.sync_uv_view(
                device,
                &self.model,
                self.model_revision,
                uv.channel,
                uv.shading_mode,
            );
            resources.update_camera_uv(queue, uv.camera);
        } else {
            // Back in the 3D scene: free the (potentially large) UV wireframe so
            // the steady-state view holds no derived UV buffer (invariant 3).
            resources.free_uv_view(device);

            if resources.model_revision != self.model_revision {
                // A new model rebuilds the steady-state mesh and resets every
                // derived line view to "not built" — they are (re)built on demand
                // below only for the views currently switched on (invariant 3).
                resources.update_model(
                    device,
                    &self.model,
                    self.model_revision,
                    self.debug_options,
                );
            } else if resources.mesh_uv_channel != self.debug_options.uv_channel {
                // Switching UV channel only rebuilds the mesh vertex buffer's UVs;
                // the rest of the derived geometry is channel-independent.
                resources.update_mesh_channel(device, &self.model, self.debug_options.uv_channel);
            }

            // Build-on-demand / free-on-off for the derived line views: a view's
            // buffer exists only while its toggle is on, and is rebuilt live when
            // its baked length/color drifts from the current options (invariant 3).
            resources.sync_line_views(device, &self.model, self.debug_options);

            // Rebuild the IBL maps if the environment changed (one-time, not per
            // frame); only the 3D path uses them.
            resources.sync_environment(device, queue, self.environment);

            resources.update_camera(
                queue,
                self.camera,
                self.projection_mode,
                self.debug_options,
                self.environment,
            );
        }

        // Render the scene (3D or UV) into the offscreen HDR targets now, on egui's
        // encoder, so it runs before egui's main pass. `paint` then composites the
        // resolved result into egui's framebuffer behind the chrome. Sync the
        // targets / scene pipelines to the framebuffer size + MSAA level first.
        let [width, height] = screen_descriptor.size_in_pixels;
        resources.sync_anti_aliasing(device, queue, width, height, self.anti_aliasing);

        // Bloom + SSAO are 3D-only effects (both forced off in UV mode). Feed the
        // composite the FXAA / bloom / SSAO flags, and run their passes after the
        // scene so the blurred glow + AO are ready when `paint` composites them.
        let bloom_active = self.uv_view.is_none() && self.bloom.enabled;
        let ssao_active = self.uv_view.is_none() && self.ssao.enabled;
        resources
            .bloom
            .update_threshold(queue, self.bloom.threshold);
        if ssao_active {
            // The settings' radius/bias are fractions of the framed model's
            // bounding-sphere radius, so the AO look is scale-invariant; scale them
            // into view units by the live scene radius here.
            let scene_radius = self.camera.scene_radius.max(1e-3);
            resources.ssao.update(
                queue,
                self.camera.projection_matrix(self.projection_mode),
                matches!(self.projection_mode, CameraProjection::Orthographic),
                self.ssao.radius * scene_radius,
                self.ssao.bias * scene_radius,
                self.ssao.intensity,
            );
        }
        resources.post.update_uniform(
            queue,
            width.max(1),
            height.max(1),
            self.anti_aliasing.effective_fxaa(),
            bloom_active,
            self.bloom.intensity,
            ssao_active,
        );

        self.encode_scene(resources, egui_encoder);
        if ssao_active {
            resources.encode_ssao(egui_encoder);
        }
        if bloom_active {
            resources.encode_bloom(egui_encoder);
        }

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

        // Composite the offscreen scene (already drawn + resolved in `prepare`)
        // into egui's framebuffer with a single fullscreen blit, behind the chrome.
        render_pass.set_pipeline(&resources.post.pipeline);
        render_pass.set_bind_group(0, &resources.post_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

impl SceneCallback {
    /// Record the scene into the offscreen HDR target: a single MSAA color pass
    /// (resolved to a sampleable single-sample texture) with depth. The draw list
    /// is identical to what used to run directly in egui's pass — only the target
    /// changed — so the composited image is unchanged (Phase 1).
    fn encode_scene(&self, resources: &SceneResources, encoder: &mut wgpu::CommandEncoder) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("review_scene_offscreen_pass"),
            color_attachments: &[
                // Location 0: display-space color the composite shows.
                Some(wgpu::RenderPassColorAttachment {
                    view: &resources.targets.color_render_view,
                    resolve_target: resources.targets.color_resolve_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SCENE_CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                // Location 1: linear-HDR bloom source (cleared to black so empty
                // background contributes no glow).
                Some(wgpu::RenderPassColorAttachment {
                    view: &resources.targets.bloom_render_view,
                    resolve_target: resources.targets.bloom_resolve_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SCENE_CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                // Location 2: SSAO G-buffer (view normal + view Z), cleared to all
                // zero so the empty background reads as unoccluded.
                Some(wgpu::RenderPassColorAttachment {
                    view: &resources.targets.gbuffer_render_view,
                    resolve_target: resources.targets.gbuffer_resolve_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &resources.targets.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        self.record_scene(resources, &mut render_pass);
    }

    /// Draw the scene geometry into `render_pass` (the offscreen target). Branches
    /// on `uv_view` for the 2D UV viewport vs the 3D scene.
    fn record_scene<'pass>(
        &self,
        resources: &'pass SceneResources,
        render_pass: &mut wgpu::RenderPass<'pass>,
    ) {
        // UV viewport: draw the 0..1 grid, then the island fill (solid-shaded /
        // per-island modes only — empty otherwise), then the model's UV edges on
        // top. group 1 must still be bound to satisfy the shared pipeline layout
        // even though none of these draws sample it.
        if self.uv_view.is_some() {
            render_pass.set_bind_group(1, &resources.checker_bind_group_greyscale, &[]);
            // group 2 (IBL) must be bound to satisfy the shared pipeline layout
            // even though the UV path never samples it.
            render_pass.set_bind_group(2, &resources.ibl.bind_group, &[]);
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            if resources.uv_grid_vertex_count > 0 {
                render_pass.set_vertex_buffer(0, resources.uv_grid_vertex_buffer.slice(..));
                render_pass.draw(0..resources.uv_grid_vertex_count, 0..1);
            }
            if resources.uv_fill_vertex_count > 0 {
                render_pass.set_pipeline(&resources.uv_fill_pipeline);
                render_pass.set_vertex_buffer(0, resources.uv_fill_vertex_buffer.slice(..));
                render_pass.draw(0..resources.uv_fill_vertex_count, 0..1);
            }
            if resources.uv_wireframe_vertex_count > 0 {
                render_pass.set_pipeline(&resources.line_pipeline);
                render_pass.set_vertex_buffer(0, resources.uv_wireframe_vertex_buffer.slice(..));
                render_pass.draw(0..resources.uv_wireframe_vertex_count, 0..1);
            }
            return;
        }

        // group 1 (checker texture + sampler) stays bound for every draw using
        // the shared pipeline layout; only the mesh actually samples it.
        let checker_bind_group = match self.debug_options.uv_checker_texture {
            CheckerTexture::Greyscale => &resources.checker_bind_group_greyscale,
            CheckerTexture::Color => &resources.checker_bind_group_color,
        };
        render_pass.set_bind_group(1, checker_bind_group, &[]);
        // group 2 (the IBL maps) is likewise bound for every draw — the shared
        // pipeline layout includes it; only the shaded path and skybox sample it.
        render_pass.set_bind_group(2, &resources.ibl.bind_group, &[]);

        // Skybox background first, behind all geometry (depth-test always, no
        // write), when the environment is shown as the background.
        if self.environment.show_background {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.skybox_pipeline);
            render_pass.draw(0..3, 0..1);
        }

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

        if (self.debug_options.wireframe_overlay
            || matches!(self.debug_options.shading_mode, ShadingMode::Wireframe))
            && resources.wireframe_line_vertex_count > 0
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
    /// MSAA sample count the scene pipelines + targets are currently built for.
    /// Rebuilt when the chosen [`AntiAliasing::msaa`] level changes.
    scene_sample_count: u32,
    /// Retained so the scene pipelines can be rebuilt when the MSAA level changes
    /// (pipeline sample count is baked at creation).
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    /// Layout of the IBL bind group (group 2), embedded in `pipeline_layout`.
    /// Stable across environment switches so the pipelines stay valid; passed to
    /// every [`IblResources::new`] to rebuild the bind group.
    ibl_layout: wgpu::BindGroupLayout,
    /// Precomputed image-based-lighting maps + their bind group. Rebuilt by
    /// `sync_environment` only when the chosen environment changes.
    ibl: IblResources,
    /// Offscreen HDR color + depth the scene renders into, recreated on resize or
    /// MSAA change.
    targets: SceneTargets,
    /// The fullscreen composite pass (offscreen scene → egui's framebuffer).
    post: PostPass,
    /// Bind group feeding the resolved scene color + blurred bloom to `post`;
    /// rebuilt with `targets`.
    post_bind_group: wgpu::BindGroup,
    /// The bloom bright-pass + blur pass (Phase 4). Size-independent; the half-res
    /// ping-pong textures + bind groups below are rebuilt with `targets`.
    bloom: BloomPass,
    /// Half-resolution HDR ping-pong textures the bloom blur bounces between.
    /// `a` holds the bright-pass output and the final (post-blur) result `post`
    /// samples; `b` is the intermediate. Recreated on resize.
    bloom_tex_a: wgpu::TextureView,
    bloom_tex_b: wgpu::TextureView,
    /// Bloom bind groups: bright-pass reads the scene bloom source, the blur reads
    /// `a` (→ `b`) then `b` (→ `a`). Rebuilt when the bloom textures are.
    bloom_brightpass_bind_group: wgpu::BindGroup,
    bloom_blur_h_bind_group: wgpu::BindGroup,
    bloom_blur_v_bind_group: wgpu::BindGroup,
    /// The SSAO occlusion + blur pass (Phase 5). Size-independent; the full-res AO
    /// textures + bind groups below are rebuilt with `targets`.
    ssao: SsaoPass,
    /// Full-resolution AO textures: `raw` holds the occlusion pass output, `blur`
    /// the denoised result the composite samples. Recreated on resize.
    ssao_raw_view: wgpu::TextureView,
    ssao_blur_view: wgpu::TextureView,
    /// SSAO bind groups: the occlusion pass reads the scene G-buffer, the blur
    /// reads `raw`. Rebuilt when the AO textures / G-buffer are.
    ssao_bind_group: wgpu::BindGroup,
    ssao_blur_bind_group: wgpu::BindGroup,
    model_revision: u64,
    mesh_uv_channel: u32,
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    /// Fullscreen pipeline that draws the environment cubemap as the background.
    skybox_pipeline: wgpu::RenderPipeline,
    /// Flat-color triangle pipeline for the UV island fill: no lighting (the fill
    /// vertices carry a zero normal), no depth write/bias — it sits under the UV
    /// wireframe and is composited by draw order in the 2D viewport.
    uv_fill_pipeline: wgpu::RenderPipeline,
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
    // UV viewport buffers. The 0..1 grid is static (built once); the UV
    // wireframe is built on demand for the active `(model_revision, channel)`
    // and freed when the 3D scene is shown again (invariant 3).
    uv_grid_vertex_buffer: wgpu::Buffer,
    uv_grid_vertex_count: u32,
    uv_wireframe_vertex_buffer: wgpu::Buffer,
    uv_wireframe_vertex_count: u32,
    /// `(model_revision, channel)` baked into the UV wireframe, or `None` when
    /// the view is off (its buffer holds only a placeholder).
    uv_baked: Option<(u64, u32)>,
    // UV island fill. Built on demand for the active `(model_revision, channel,
    // shading_mode)` when the mode draws a fill (Shaded / Islands), and freed
    // back to a placeholder in Wire mode or when the 3D scene is shown.
    uv_fill_vertex_buffer: wgpu::Buffer,
    uv_fill_vertex_count: u32,
    /// `(model_revision, channel, shading_mode)` baked into the UV fill, or
    /// `None` when no fill is drawn (its buffer holds only a placeholder).
    uv_fill_baked: Option<(u64, u32, UvShadingMode)>,
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

        // The IBL maps occupy bind group 2; its layout is part of the shared
        // pipeline layout, so every scene pipeline can sample the environment.
        let ibl_layout = IblResources::scene_layout(device);
        let ibl = IblResources::new(
            device,
            queue,
            &ibl_layout,
            EnvironmentSettings::default().map,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("review_scene_pipeline_layout"),
            bind_group_layouts: &[&uniform_layout, &checker_layout, &ibl_layout],
            push_constant_ranges: &[],
        });

        // Scene pipelines start at the default MSAA level and are rebuilt by
        // `sync_anti_aliasing` when the level changes (the sample count is baked
        // into a pipeline at creation). `build_scene_pipelines` carries the
        // depth-bias reasoning.
        let scene_sample_count = AntiAliasing::default().msaa.sample_count();
        let (mesh_pipeline, line_pipeline, uv_fill_pipeline, skybox_pipeline) =
            build_scene_pipelines(device, &pipeline_layout, &shader, scene_sample_count);

        // Offscreen targets + the composite pass. Targets start at 1x1 and are
        // recreated at the real framebuffer size on the first `prepare`
        // (`sync_anti_aliasing`); the post pass draws into egui's `output_format`.
        let targets = SceneTargets::new(device, queue, 1, 1, scene_sample_count);
        let post = PostPass::new(device, output_format);
        let bloom = BloomPass::new(device);
        let ssao = SsaoPass::new(device);
        let (
            bloom_tex_a,
            bloom_tex_b,
            bloom_brightpass_bind_group,
            bloom_blur_h_bind_group,
            bloom_blur_v_bind_group,
        ) = build_bloom_targets(device, queue, &targets, &bloom);
        let (ssao_raw_view, ssao_blur_view, ssao_bind_group, ssao_blur_bind_group) =
            build_ssao_targets(device, queue, &targets, &ssao);
        let post_bind_group = post.create_bind_group(
            device,
            targets.sampled_view(),
            &bloom_tex_a,
            &ssao_blur_view,
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

        Self {
            output_format,
            scene_sample_count,
            shader,
            pipeline_layout,
            ibl_layout,
            ibl,
            targets,
            post,
            post_bind_group,
            bloom,
            bloom_tex_a,
            bloom_tex_b,
            bloom_brightpass_bind_group,
            bloom_blur_h_bind_group,
            bloom_blur_v_bind_group,
            ssao,
            ssao_raw_view,
            ssao_blur_view,
            ssao_bind_group,
            ssao_blur_bind_group,
            model_revision: u64::MAX,
            mesh_uv_channel: 0,
            mesh_pipeline,
            line_pipeline,
            skybox_pipeline,
            uv_fill_pipeline,
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

    /// Bring the offscreen targets, scene pipelines and composite uniform in line
    /// with the framebuffer size + the chosen antialiasing. Targets are recreated
    /// when the size or MSAA level changes; the scene pipelines are rebuilt only
    /// when the MSAA level changes (their sample count is baked at creation); the
    /// FXAA flag + texel size are written every frame (cheap). Steady-state frames
    /// (unchanged size/level) allocate nothing.
    fn sync_anti_aliasing(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        anti_aliasing: AntiAliasing,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        // The master AA toggle collapses to single-sample + no FXAA when off; the
        // panel's stored level/flag are honoured only while it is on.
        let sample_count = anti_aliasing.effective_sample_count();

        if self.scene_sample_count != sample_count {
            let (mesh_pipeline, line_pipeline, uv_fill_pipeline, skybox_pipeline) =
                build_scene_pipelines(device, &self.pipeline_layout, &self.shader, sample_count);
            self.mesh_pipeline = mesh_pipeline;
            self.line_pipeline = line_pipeline;
            self.uv_fill_pipeline = uv_fill_pipeline;
            self.skybox_pipeline = skybox_pipeline;
            self.scene_sample_count = sample_count;
        }

        let targets_stale = self.targets.width != width
            || self.targets.height != height
            || self.targets.sample_count != sample_count;
        if targets_stale {
            self.targets = SceneTargets::new(device, queue, width, height, sample_count);
            // Every bind group reading the targets (bloom ping-pong, SSAO AO
            // textures, and the composite) is now stale; rebuild them all.
            let (
                bloom_tex_a,
                bloom_tex_b,
                bloom_brightpass_bind_group,
                bloom_blur_h_bind_group,
                bloom_blur_v_bind_group,
            ) = build_bloom_targets(device, queue, &self.targets, &self.bloom);
            self.bloom_tex_a = bloom_tex_a;
            self.bloom_tex_b = bloom_tex_b;
            self.bloom_brightpass_bind_group = bloom_brightpass_bind_group;
            self.bloom_blur_h_bind_group = bloom_blur_h_bind_group;
            self.bloom_blur_v_bind_group = bloom_blur_v_bind_group;

            let (ssao_raw_view, ssao_blur_view, ssao_bind_group, ssao_blur_bind_group) =
                build_ssao_targets(device, queue, &self.targets, &self.ssao);
            self.ssao_raw_view = ssao_raw_view;
            self.ssao_blur_view = ssao_blur_view;
            self.ssao_bind_group = ssao_bind_group;
            self.ssao_blur_bind_group = ssao_blur_bind_group;

            // The composite reads the resolved color, the blurred bloom, and the
            // blurred AO — all just recreated.
            self.post_bind_group = self.post.create_bind_group(
                device,
                self.targets.sampled_view(),
                &self.bloom_tex_a,
                &self.ssao_blur_view,
            );
        }
    }

    /// Run the bloom passes on egui's encoder, after the scene pass: bright-pass
    /// the scene's resolved linear-HDR bloom source into the half-res `a`, then
    /// ping-pong a separable Gaussian blur between `a` and `b`. Ends in `a`, which
    /// the composite (`post`) samples. Only called when bloom is active.
    fn encode_bloom(&self, encoder: &mut wgpu::CommandEncoder) {
        bloom_fullscreen_pass(
            encoder,
            &self.bloom.brightpass_pipeline,
            &self.bloom_brightpass_bind_group,
            &self.bloom_tex_a,
            "review_bloom_brightpass_pass",
        );
        for _ in 0..BLOOM_BLUR_ITERATIONS {
            // Horizontal: a → b, then vertical: b → a.
            bloom_fullscreen_pass(
                encoder,
                &self.bloom.blur_pipeline,
                &self.bloom_blur_h_bind_group,
                &self.bloom_tex_b,
                "review_bloom_blur_h_pass",
            );
            bloom_fullscreen_pass(
                encoder,
                &self.bloom.blur_pipeline,
                &self.bloom_blur_v_bind_group,
                &self.bloom_tex_a,
                "review_bloom_blur_v_pass",
            );
        }
    }

    /// Run the SSAO passes on egui's encoder, after the scene pass: the occlusion
    /// pass reads the scene's resolved G-buffer into `raw`, then the box blur
    /// denoises `raw` → `blur`, which the composite (`post`) samples. Reuses the
    /// bloom fullscreen-pass helper (single color attachment, no depth). Only
    /// called when SSAO is active.
    fn encode_ssao(&self, encoder: &mut wgpu::CommandEncoder) {
        bloom_fullscreen_pass(
            encoder,
            &self.ssao.ssao_pipeline,
            &self.ssao_bind_group,
            &self.ssao_raw_view,
            "review_ssao_pass",
        );
        bloom_fullscreen_pass(
            encoder,
            &self.ssao.blur_pipeline,
            &self.ssao_blur_bind_group,
            &self.ssao_blur_view,
            "review_ssao_blur_pass",
        );
    }

    fn update_camera(
        &self,
        queue: &wgpu::Queue,
        camera: OrbitCamera,
        projection_mode: CameraProjection,
        debug_options: SceneDebugOptions,
        environment: EnvironmentSettings,
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
            view: camera.view_matrix().to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Rebuild the IBL maps when the chosen environment changes. One-time GPU
    /// work (a burst of small precompute passes); a no-op when the map is
    /// unchanged, so steady-state frames pay nothing.
    fn sync_environment(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        environment: EnvironmentSettings,
    ) {
        if self.ibl.environment != environment.map {
            self.ibl = IblResources::new(device, queue, &self.ibl_layout, environment.map);
        }
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
        let wireframe_on = debug_options.wireframe_overlay
            || matches!(debug_options.shading_mode, ShadingMode::Wireframe);
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

    /// Build-on-demand for the UV viewport's derived buffers. The wireframe is
    /// rebuilt only when the active model or channel changes (`uv_baked`); the
    /// island fill is rebuilt when the model / channel / shading mode changes
    /// (`uv_fill_baked`) and freed in Wire mode — so panning/zooming rebuilds
    /// nothing (invariant 3).
    fn sync_uv_view(
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
    fn free_uv_view(&mut self, device: &wgpu::Device) {
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
    fn update_camera_uv(&self, queue: &wgpu::Queue, camera: UvCamera) {
        let uniforms = SceneUniforms {
            view_projection: camera.view_projection().to_cols_array_2d(),
            inv_view_projection: [[0.0; 4]; 4],
            render_options: [0.0; 4],
            camera_position: [0.0; 4],
            // The UV path never reaches the IBL / skybox code, so these are unused.
            env_params: [0.0; 4],
            // No SSAO in the UV viewport (its fill/lines write a zero G-buffer), so
            // the view matrix is unused here.
            view: [[0.0; 4]; 4],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}

/// Depth behavior for a pipeline: whether it writes depth, and how much it biases
/// fragments. The mesh writes depth and pushes the surface back (so coplanar line
/// overlays win the depth test); the line pipeline does neither.
struct DepthConfig {
    write_enabled: bool,
    bias: wgpu::DepthBiasState,
}

/// Build the three scene pipelines (mesh / line / UV-fill) for a given MSAA
/// sample count. Rebuilt whenever the level changes, since the sample count is
/// baked into pipeline state.
///
/// The wireframe (and other line overlays) share vertex positions with the
/// shaded surface they trace, so they z-fight it: on curved faces edges sink
/// behind the surface and drop out, and MSAA partial occlusion leaves the
/// survivors uneven in opacity/thickness. We can't bias the lines directly — on
/// DX12 depth bias applies only to triangle primitives — so instead the mesh
/// pipeline pushes the shaded surface a hair *away* from the camera with a
/// slope-scaled depth bias. Lines then render at their true depth and win the
/// `LessEqual` test against the receded surface, while still being correctly
/// occluded by geometry genuinely in front of them (slope-scaled bias is in real
/// depth-buffer units, so it never over-pulls the far side through the front the
/// way a constant clip-space line offset did). The scene pipelines render into
/// the offscreen HDR target (`SCENE_HDR_FORMAT`), not egui's framebuffer; the
/// post pass composites the result back.
fn build_scene_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    sample_count: u32,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let mesh_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        DepthConfig {
            write_enabled: true,
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.0,
                clamp: 0.0,
            },
        },
        sample_count,
        "review_scene_mesh_pipeline",
    );
    let line_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::LineList,
        DepthConfig {
            write_enabled: false,
            bias: wgpu::DepthBiasState::default(),
        },
        sample_count,
        "review_scene_line_pipeline",
    );
    // The UV island fill draws flat-color triangles in the 2D viewport. It never
    // writes depth (everything sits at z=0) so the grid below and the wireframe
    // above composite purely by draw order.
    let uv_fill_pipeline = create_pipeline(
        device,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        DepthConfig {
            write_enabled: false,
            bias: wgpu::DepthBiasState::default(),
        },
        sample_count,
        "review_scene_uv_fill_pipeline",
    );
    let skybox_pipeline = create_skybox_pipeline(device, layout, shader, sample_count);
    (
        mesh_pipeline,
        line_pipeline,
        uv_fill_pipeline,
        skybox_pipeline,
    )
}

/// The skybox pipeline: a fullscreen triangle (no vertex buffer) sampling the
/// environment cubemap. It draws first in the offscreen pass over the cleared
/// frame, so it never writes depth and always passes the depth test; scene
/// geometry then composites on top. Renders into the HDR target at the scene's
/// MSAA level, so it is rebuilt alongside the other pipelines when MSAA changes.
fn create_skybox_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("review_scene_skybox_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_skybox"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SCENE_DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_skybox"),
            // MRT to match the other scene pipelines: location 0 = display color,
            // location 1 = linear-HDR bloom source, location 2 = SSAO G-buffer. The
            // skybox draws first over the cleared frame, so no target blends (opaque
            // replace); it writes a zero G-buffer (background = unoccluded).
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
    depth: DepthConfig,
    sample_count: u32,
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
            depth_write_enabled: depth.write_enabled,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: depth.bias,
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            // MRT (invariant 11: matches the `FragOutput` struct in `scene.wgsl`):
            // location 0 = display color, location 1 = linear-HDR bloom source,
            // location 2 = SSAO G-buffer. Locations 0 and 1 alpha-blend so overlay
            // draws (which write bloom alpha 0) leave the geometry's bloom value
            // intact. The G-buffer packs view Z in its `.w`, so it can NOT alpha-
            // blend (the `.w` is data, not coverage) — it replaces; overlays write a
            // zero G-buffer there, which SSAO reads as background.
            targets: &[
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SCENE_HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
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
        vertex_color: [0.0, 0.0, 0.0, 0.0],
        smoothness: 0.0,
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
        vertex_color: [0.0, 0.0, 0.0, 0.0],
        smoothness: 0.0,
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

/// Create one half-resolution HDR bloom ping-pong texture (render target + sampled
/// in later passes) and return its view.
fn create_bloom_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// (Re)create the half-resolution bloom textures + the three bloom bind groups,
/// and refresh the blur sampling offsets for the new size. Called on creation and
/// whenever the scene targets are recreated (resize / MSAA change). Returns the
/// new `(bloom_a, bloom_b, brightpass_bg, blur_h_bg, blur_v_bg)`. The composite
/// bind group is built by the caller (it also depends on the SSAO AO texture).
fn build_bloom_targets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    targets: &SceneTargets,
    bloom: &BloomPass,
) -> (
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::BindGroup,
    wgpu::BindGroup,
) {
    let half_width = (targets.width / 2).max(1);
    let half_height = (targets.height / 2).max(1);
    let bloom_tex_a = create_bloom_texture(device, half_width, half_height, "review_bloom_tex_a");
    let bloom_tex_b = create_bloom_texture(device, half_width, half_height, "review_bloom_tex_b");

    // The composite binds `bloom_tex_a` every frame but only samples it when bloom
    // is active. Clear both once at creation so D3D12's debug layer doesn't flag
    // the texture as read-before-initialized while bloom is off (the bloom passes
    // fully overwrite them with `LoadOp::Clear` whenever bloom is on).
    clear_views(
        device,
        queue,
        &[&bloom_tex_a, &bloom_tex_b],
        "review_bloom_init",
    );

    bloom.update_blur_offsets(queue, half_width, half_height);

    let brightpass_bg = bloom.brightpass_bind_group(device, targets.sampled_bloom_view());
    // The horizontal blur reads `a` (→ `b`); the vertical reads `b` (→ `a`).
    let blur_h_bg = bloom.blur_h_bind_group(device, &bloom_tex_a);
    let blur_v_bg = bloom.blur_v_bind_group(device, &bloom_tex_b);

    (
        bloom_tex_a,
        bloom_tex_b,
        brightpass_bg,
        blur_h_bg,
        blur_v_bg,
    )
}

/// Create one full-resolution single-channel AO texture (render target + sampled
/// in later passes) and return its view.
fn create_ssao_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    label: &str,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SSAO_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// (Re)create the full-resolution AO textures + the SSAO bind groups (occlusion
/// pass reads the scene G-buffer → `raw`; blur reads `raw` → `blur`). Called on
/// creation and whenever the scene targets are recreated. Returns the new
/// `(raw, blur, ssao_bg, blur_bg)`. As with bloom, the composite binds the blurred
/// AO every frame but only samples it when SSAO is on, so both AO textures are
/// cleared once here to satisfy D3D12's read-before-init validation.
fn build_ssao_targets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    targets: &SceneTargets,
    ssao: &SsaoPass,
) -> (
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::BindGroup,
) {
    let raw = create_ssao_texture(device, targets.width, targets.height, "review_ssao_raw");
    let blur = create_ssao_texture(device, targets.width, targets.height, "review_ssao_blur");
    clear_views(device, queue, &[&raw, &blur], "review_ssao_init");

    let ssao_bg = ssao.bind_group(device, targets.sampled_gbuffer_view(), "review_ssao_bg");
    let blur_bg = ssao.bind_group(device, &raw, "review_ssao_blur_bg");
    (raw, blur, ssao_bg, blur_bg)
}

/// Clear a set of color views once (`LoadOp::Clear` to black), in a single
/// throwaway encoder. Used to initialise render targets that a later pass binds
/// but may not write before first read (bloom / AO when their effect is off).
fn clear_views(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    views: &[&wgpu::TextureView],
    label: &str,
) {
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    for view in views {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    queue.submit(std::iter::once(encoder.finish()));
}

/// Run one fullscreen bloom pass: clear `target`, bind `pipeline` + `bind_group`,
/// draw the fullscreen triangle. The bloom textures are single-sample with no
/// depth, so the pass is a bare color attachment.
fn bloom_fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
    label: &str,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
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
    /// Inverse view-projection, for reconstructing world ray directions in the
    /// skybox pass.
    inv_view_projection: [[f32; 4]; 4],
    render_options: [f32; 4],
    /// World-space camera eye in `xyz` (`w` is padding). Used by the shaded path
    /// to build the view vector for the specular highlight / reflection.
    camera_position: [f32; 4],
    /// Image-based lighting: `x` = IBL enabled (>0.5), `y` = intensity, `z` =
    /// show background skybox (>0.5), `w` = prefiltered-cube max mip LOD.
    env_params: [f32; 4],
    /// View matrix (world → view), for writing the view-space normal + depth into
    /// the SSAO G-buffer.
    view: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SceneVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) uv: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) vertex_color: [f32; 4],
    pub(crate) smoothness: f32,
}

impl SceneVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4, 5 => Float32
    ];

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
    }
}

/// Encode the vertex-color view into `render_options.w` for the shader: `-1`
/// when the active material isn't vertex colors, otherwise the mode index (`0`
/// RGB, `1` Alpha, `2` RGB+A). One float keeps the uniform layout unchanged.
fn vertex_color_value(debug_options: SceneDebugOptions) -> f32 {
    if debug_options.active_material != ActiveMaterial::VertexColors {
        return -1.0;
    }
    match debug_options.vertex_color_mode {
        VertexColorMode::Rgb => 0.0,
        VertexColorMode::Alpha => 1.0,
        VertexColorMode::RgbAlpha => 2.0,
    }
}
