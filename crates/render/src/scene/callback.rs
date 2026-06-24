//! The egui paint callback. [`SceneCallback`] carries the per-frame inputs (what
//! to draw) and drives a full scene render: `prepare` reconciles the GPU resources
//! and encodes the offscreen passes, `paint` composites the result. The 3D/UV
//! draw list (`record_scene`) and the offscreen pass setup (`encode_scene`) live
//! here too, since they read `SceneCallback`'s own state.

use std::sync::Arc;

use egui::epaint::PaintCallbackInfo;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use review_model::ModelData;

use crate::material::MaterialState;
use crate::selection::SelectionView;
use crate::{
    AntiAliasing, CameraProjection, CheckerTexture, EnvironmentSettings, GtaoSettings, OrbitCamera,
    SceneDebugOptions, ShadingMode, TonemapSettings, UvCamera, UvShadingMode,
};

use super::SCENE_CLEAR_COLOR;
use super::SceneResources;

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
    /// Scene MSAA level. Drives the offscreen target / scene pipeline sample count.
    anti_aliasing: AntiAliasing,
    /// Image-based lighting / environment selection. Drives the precomputed IBL
    /// maps, the PBR shaded path, and the optional skybox.
    environment: EnvironmentSettings,
    /// Screen-space ambient occlusion settings. Drives the GTAO + blur passes and
    /// the composite multiply.
    gtao: GtaoSettings,
    /// Tone-mapping settings. Drives the tone-map stage of the composite shader.
    tonemap: TonemapSettings,
    /// Editable per-material PBR parameters (seeded from import defaults, edited
    /// via UI intents). Uploaded into the renderer's material table when
    /// `material_revision` changes; the mesh is drawn one range per material.
    materials: Vec<MaterialState>,
    /// Bumped by `app` on every material edit (and on model load) so the table is
    /// re-uploaded without rebuilding the mesh. Separate from `model_revision`.
    material_revision: u64,
    /// Outliner selection + solo flag + highlight color. Drives the viewport
    /// outline and the solo (isolate) draw filter (Phase 2).
    selection: SelectionView,
    /// Mesh nodes the Outliner has hidden (node indices). Their triangles are
    /// filtered out of the viewport draw + GTAO G-buffer (Phase 2). Empty means
    /// everything is visible.
    hidden_meshes: Vec<u32>,
    /// `Some` renders the 2D UV viewport instead of the 3D scene.
    uv_view: Option<UvView>,
}

impl SceneCallback {
    // Fourteen distinct, independent inputs (camera + projection + target + model +
    // revision + the five UI option bundles + the editable material table + its
    // revision + the Outliner selection + the hidden-mesh set); there is no
    // redundant pair to fold away, and a params struct would only move the same
    // values behind one name.
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
        gtao: GtaoSettings,
        tonemap: TonemapSettings,
        materials: &[MaterialState],
        material_revision: u64,
        selection: SelectionView,
        hidden_meshes: &[u32],
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
            gtao,
            tonemap,
            materials: materials.to_vec(),
            material_revision,
            selection,
            hidden_meshes: hidden_meshes.to_vec(),
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
            // The UV viewport keeps the default scene AA (4× MSAA), so switching
            // to UV mode renders exactly as it did pre-Phase-2.
            anti_aliasing: AntiAliasing::default(),
            // IBL is irrelevant to the 2D UV viewport; the skybox/IBL paths are
            // never reached there (the UV path returns before them).
            environment: EnvironmentSettings::default(),
            // GTAO is a 3D-only effect; the flat UV viewport has no depth to occlude.
            gtao: GtaoSettings {
                enabled: false,
                ..GtaoSettings::default()
            },
            // The UV viewport keeps the default tone mapping so its shaded fills
            // read the same as in the 3D scene.
            tonemap: TonemapSettings::default(),
            // The UV path samples no material; the table keeps its fallback entry,
            // bound to satisfy the shared pipeline layout.
            materials: Vec::new(),
            material_revision: 0,
            // No Outliner selection in the 2D UV viewport.
            selection: SelectionView::default(),
            // The 2D UV viewport draws every mesh's UVs; visibility is 3D-only.
            hidden_meshes: Vec::new(),
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
        // Create the cheap core resources on the first frame (Phase B). The heavy
        // scene pipelines + GTAO pass are compiled afterwards, one stage per frame,
        // by `advance_build`.
        let fresh = callback_resources.get::<SceneResources>().is_none();
        let resources = callback_resources
            .entry::<SceneResources>()
            .or_insert_with(|| SceneResources::new_core(device, queue, self.output_format));

        let recreated = if resources.output_format != self.output_format {
            *resources = SceneResources::new_core(device, queue, self.output_format);
            true
        } else {
            false
        };

        // Walk the deferred build forward one stage per frame, driven across frames
        // by `app`'s startup warmup. Skipped on the frame that just built the core
        // (fresh / format change), so the first presented (grid-only) frame pays
        // only the core cost.
        if !fresh && !recreated {
            resources.advance_build(device, queue);
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
            // The 2D UV viewport has no Outliner selection or per-mesh visibility;
            // drop any selection / visibility buffers built for the 3D scene
            // (invariant 3).
            resources.free_selection(device);
            resources.free_visibility(device);
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
            resources.sync_line_views(device, &self.model, self.debug_options, &self.hidden_meshes);

            // Rebuild the IBL maps if the environment changed (one-time, not per
            // frame); only the 3D path uses them.
            resources.sync_environment(device, queue, self.environment);

            // Bring the editable material table in line with the current values:
            // rebuilt when the material count changes (new model), re-uploaded when
            // an edit bumps the revision, otherwise left untouched.
            resources.sync_materials(device, queue, &self.materials, self.material_revision);

            // Build (or free) the selected-triangle index buffer (solo isolate +
            // highlight-flash fill source) when the Outliner selection changes
            // (invariant 3).
            resources.sync_selection(
                device,
                &self.model,
                self.model_revision,
                self.selection,
                &self.hidden_meshes,
            );

            // Build (or free) the per-mesh visibility draw list when the Outliner's
            // hidden-mesh set changes (invariant 3): the filtered index buffer
            // exists only while some mesh is hidden.
            resources.sync_visibility(
                device,
                &self.model,
                self.model_revision,
                &self.hidden_meshes,
            );

            // The highlight flash rides in the uniform: gamma-space color in rgb,
            // the flash fade in alpha (0 while nothing is selected/flashing). The
            // geometry is keyed only on *what* is selected, so this per-frame change
            // never rebuilds a buffer.
            let selection_color = if self.selection.selection.is_active() {
                let [r, g, b, _] = self.selection.highlight_color;
                [r, g, b, self.selection.fade.clamp(0.0, 1.0)]
            } else {
                [0.0; 4]
            };

            resources.update_camera(
                queue,
                self.camera,
                self.projection_mode,
                self.debug_options,
                self.environment,
                selection_color,
            );
        }

        // Render the scene (3D or UV) into the offscreen HDR targets now, on egui's
        // encoder, so it runs before egui's main pass. `paint` then composites the
        // resolved result into egui's framebuffer behind the chrome. Sync the
        // targets / scene pipelines to the framebuffer size + MSAA level first.
        let [width, height] = screen_descriptor.size_in_pixels;
        resources.sync_anti_aliasing(device, queue, width, height, self.anti_aliasing);

        // GTAO is a 3D-only effect (forced off in UV mode), and is only active once
        // its deferred pass is built (Phase B) — until then it reports inactive, so
        // the composite doesn't darken by AO (an empty startup scene has nothing to
        // occlude anyway). Feed the composite the GTAO flag and run its passes after
        // the scene so the blurred AO is ready when `paint` composites it.
        let gtao_active = self.uv_view.is_none() && self.gtao.enabled && resources.gtao.is_some();
        if gtao_active {
            // gtao_active implies the deferred GTAO pass is built.
            if let Some(gtao) = &resources.gtao {
                // The settings' radius/bias are fractions of the framed model's
                // bounding-sphere radius, so the AO look is scale-invariant; scale
                // them into view units by the live scene radius here.
                let scene_radius = self.camera.scene_radius.max(1e-3);
                let (slices, steps) = self.gtao.quality.slices_steps();
                gtao.update(
                    queue,
                    self.camera.projection_matrix(self.projection_mode),
                    matches!(self.projection_mode, CameraProjection::Orthographic),
                    self.gtao.radius * scene_radius,
                    self.gtao.intensity,
                    self.gtao.thickness,
                    slices,
                    steps,
                );
            }
        }
        resources.post.update_uniform(
            queue,
            gtao_active,
            self.tonemap.enabled,
            self.tonemap.operator.shader_index(),
        );
        self.encode_scene(resources, egui_encoder);
        if gtao_active {
            resources.encode_gtao_gbuffer(egui_encoder);
            resources.encode_gtao(egui_encoder);
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
                // Location 0: linear HDR scene radiance the composite tone-maps.
                Some(wgpu::RenderPassColorAttachment {
                    view: &resources.targets.color_render_view,
                    resolve_target: resources.targets.color_resolve_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SCENE_CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                // Location 1: ambient radiance GTAO is allowed to attenuate.
                Some(wgpu::RenderPassColorAttachment {
                    view: &resources.targets.ambient_render_view,
                    resolve_target: resources.targets.ambient_resolve_view.as_ref(),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &resources.targets.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
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
        // The deferred scene pipelines (Phase B): `None` during the first warmup
        // frames, so any draw that needs one (mesh / skybox / UV-fill / selection
        // flash) is skipped until they land. The grid + line overlays use the core
        // `line_pipeline`, so they always draw.
        let scene_pipelines = resources.scene_pipelines.as_ref();

        // UV viewport: draw the 0..1 grid, then the island fill (solid-shaded /
        // per-island modes only — empty otherwise), then the model's UV edges on
        // top. group 1 must still be bound to satisfy the shared pipeline layout
        // even though none of these draws sample it.
        if self.uv_view.is_some() {
            render_pass.set_bind_group(1, &resources.checker_bind_group_greyscale, &[]);
            // group 2 (IBL) must be bound to satisfy the shared pipeline layout
            // even though the UV path never samples it.
            render_pass.set_bind_group(2, &resources.ibl.bind_group, &[]);
            // group 3 (material) must likewise be bound; the UV draws don't sample
            // it, so the all-fallback bind group is fine.
            render_pass.set_bind_group(3, resources.material_table.fallback_bind_group(), &[]);
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            if resources.uv_grid_vertex_count > 0 {
                render_pass.set_vertex_buffer(0, resources.uv_grid_vertex_buffer.slice(..));
                render_pass.draw(0..resources.uv_grid_vertex_count, 0..1);
            }
            if let (Some(scene_pipelines), true) =
                (scene_pipelines, resources.uv_fill_vertex_count > 0)
            {
                render_pass.set_pipeline(&scene_pipelines.uv_fill);
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
        // group 3 (material) is bound for every draw too; the mesh loop below
        // rebinds it per material range, but the skybox / grid / overlays don't
        // sample it, so the all-fallback bind group satisfies the layout for them.
        render_pass.set_bind_group(3, resources.material_table.fallback_bind_group(), &[]);

        // Skybox background first, behind all geometry (depth-test always, no
        // write), when the environment is shown as the background.
        if let (true, Some(scene_pipelines)) = (self.environment.show_background, scene_pipelines) {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&scene_pipelines.skybox);
            render_pass.draw(0..3, 0..1);
        }

        // Mesh draw list, in precedence order: solo (isolate the selection) wins;
        // otherwise per-mesh visibility (the filtered list, present only while some
        // mesh is hidden); otherwise the whole mesh. All three share the mesh vertex
        // buffer, so only the index source + ranges differ.
        let solo = self.selection.solo && self.selection.selection.is_active();
        let draw_mesh = resources.mesh_index_count > 0
            && !matches!(self.debug_options.shading_mode, ShadingMode::Wireframe);
        if let (Some(scene_pipelines), true) = (scene_pipelines, draw_mesh) {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            // Backface Rendering off (default) culls back faces; on draws the mesh
            // double-sided. The two pipelines are prebuilt, so this is a pick.
            let mesh_pipeline = if self.debug_options.render_backfaces {
                &scene_pipelines.mesh_double_sided
            } else {
                &scene_pipelines.mesh
            };
            render_pass.set_pipeline(mesh_pipeline);
            render_pass.set_vertex_buffer(0, resources.mesh_vertex_buffer.slice(..));
            let (index_buffer, ranges) = if solo {
                (
                    &resources.selection_index_buffer,
                    &resources.selection_ranges,
                )
            } else if resources.visible_active {
                (&resources.visible_index_buffer, &resources.visible_ranges)
            } else {
                (&resources.mesh_index_buffer, &resources.material_ranges)
            };
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            // One draw per material range, binding each material's own group-3 bind
            // group (its uniform slice + its texture slots).
            for range in ranges {
                render_pass.set_bind_group(
                    3,
                    resources.material_table.bind_group_for(range.material),
                    &[],
                );
                let end = range.first_index + range.index_count;
                render_pass.draw_indexed(range.first_index..end, 0, 0..1);
            }
        }

        if self.debug_options.show_grid {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&resources.line_pipeline);
            render_pass.set_vertex_buffer(0, resources.line_vertex_buffer.slice(..));
            render_pass.draw(0..resources.line_vertex_count, 0..1);
        }

        // Model wireframe: a plain line list drawn in the scene pass so the
        // `line_pipeline`'s depth test (Reversed-Z `GreaterEqual` against the
        // mesh depth) occludes edges on hidden faces. Native line rasterization +
        // the scene MSAA keep it clean and stable (no thickness control). Drawn
        // after the mesh/grid so it sits on top wherever it is not occluded.
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

        // Selection highlight flash: a flat bright-color fill redrawing the selected
        // triangles over the mesh, fading out over ~0.5s after a selection change.
        // Drawn last so it sits on top. Reuses the selected-triangle index buffer
        // (the solo list) over the shared mesh vertex buffer; `fs_selection` tints
        // them with the uniform highlight color × the flash fade alpha. Depth-tested
        // (Reversed-Z `GreaterEqual`, no write) so the fill is occluded where the
        // selection hides behind other geometry but wins over the coplanar surface
        // it tints. Skipped once the flash has fully faded, so the steady state (and
        // a still-active-but-faded selection) pays nothing.
        let flash = self.selection.selection.is_active()
            && self.selection.fade > 0.0
            && resources.selection_index_count > 0;
        if let (Some(scene_pipelines), true) = (scene_pipelines, flash) {
            render_pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
            render_pass.set_pipeline(&scene_pipelines.selection_fill);
            render_pass.set_vertex_buffer(0, resources.mesh_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                resources.selection_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..resources.selection_index_count, 0, 0..1);
        }
    }
}
