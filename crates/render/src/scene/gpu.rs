//! The scene renderer. It owns the GPU-side scene resources (pipelines, buffers,
//! offscreen targets, IBL maps, material table) built lazily on the first frame and
//! drives the per-frame render: the scene draws into offscreen linear-HDR MRT
//! targets, and a composite job tone-maps + sRGB-encodes them into the frame's
//! swapchain pass behind the egui chrome.
//!
//! The full shaded look is the skybox + the per-material PBR/IBL `fs_main` (the
//! material uniform block + its seven texture slots + the IBL maps + the UV checker),
//! drawn one indexed range per material, plus the live tone-map operator switch, the
//! debug line views and the selection flash.
//!
//! ## Bindings are per program, not per pass
//!
//! sokol takes bindings as a *value* re-applied after every `apply_pipeline`, and it
//! validates that a draw supplies exactly what its shader declared — no more and no
//! less. So there is no "bind the shared scene state once per pass" step any more:
//! each draw assembles the set its own program asks for ([`SceneGpu::mesh_bindings`]
//! for anything running `fs_main`, [`SceneGpu::line_bindings`] for the ones that
//! sample nothing, and the skybox's own), and applies exactly the uniform blocks
//! that program declares.
//!
//! *Exactly* is the operative word, and shdc decides it: a resource a program does
//! not actually read is stripped from its reflection, so binding it is as wrong as
//! leaving a declared one empty. Three consequences worth knowing, each of which is
//! a validation failure if got wrong:
//!
//! * `fs_main` never samples the environment cube (the skybox does), so the mesh
//!   binds three IBL maps, not four.
//! * `fs_line` reads none of the fragment-stage scene uniforms, so the line
//!   pipelines apply only the vertex block — while `fs_selection`, which reads the
//!   flash colour, takes both.
//! * `vs_main` declares all four deform tables, so every program sharing it must
//!   bind all four. That is what [`DeformDummies`] is for: a model without a skin
//!   has none of them.
//!
//! Being ported (`mac-port-plan.md` Phase 1 step 4): the ambient-occlusion passes
//! are not here yet, so the composite's GTAO factor is always off.

use bytemuck::Zeroable;
use review_model::ModelData;

use crate::geometry::{scene_lines, uv_grid_lines};
use crate::ibl::{IblMaps, PREFILTER_MAX_LOD};
use crate::material::{MaterialEntry, MaterialState, MaterialTable, effective_materials};
use crate::rhi::{
    Bindings, ColorTarget, DepthTarget, Format, Frame, GpuResult, IndexBuffer, Pipeline,
    PipelineDesc, Sampler, StorageBuffer, SwapchainJob, Texture, VertexBuffer, shader,
};
use crate::selection::SelectionView;
use crate::shaders::generated;
use crate::{
    ActiveMaterial, AntiAliasing, CameraProjection, CheckerTexture, EnvironmentSettings,
    OrbitCamera, SceneDebugOptions, SceneFrame, ShadingMode, TonemapSettings, UvCamera,
    UvShadingMode, ViewportBackground,
};

use super::gpu_types::{
    InfluenceEntry, MorphEntry, PaletteEntry, PostUniforms, SceneUniforms, buffer_view_value,
    shading_mode_value, skin_weight_value, vertex_color_value,
};
use super::pipelines::{ScenePipelineSet, build_scene_pipelines};
use super::resources::{DeformGpu, ModelSlot, SlotId};

/// The greyscale + colour UV-checker PNGs (baked in; invariant: assets via
/// `include_bytes!`), uploaded once as sRGB textures and picked per frame.
const CHECKER_GREYSCALE_PNG: &[u8] =
    include_bytes!("../../../../assets/textures/T_UV_Checker_BW.png");
const CHECKER_COLOR_PNG: &[u8] = include_bytes!("../../../../assets/textures/T_UV_Checker_CLR.png");

/// Vertices in a fullscreen triangle — the skybox and the composite, both of which
/// build their corners from `gl_VertexIndex`.
const FULLSCREEN_VERTICES: usize = 3;

/// A rectangle of the backbuffer for the composite to write into: the whole thing
/// for a single view, one half for each side of the Opt workspace's split.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BackbufferRect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl BackbufferRect {
    pub(crate) fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// One-element stand-ins for the four deform tables.
///
/// `vs_main` declares all four storage buffers, and sokol validates that a draw binds
/// every view its shader declared — so a static model, or a skinned one whose morph
/// table is empty, still has to bind *something*. These are that something: 4, 48, 28
/// and 4 bytes of zeroes, never read (the uniform's deform flag is off, and an empty
/// deform lane makes the shader skip them anyway).
struct DeformDummies {
    influences: StorageBuffer<InfluenceEntry>,
    palette: StorageBuffer<PaletteEntry>,
    morph: StorageBuffer<MorphEntry>,
    weights: StorageBuffer<f32>,
}

impl DeformDummies {
    fn new() -> GpuResult<Self> {
        Ok(Self {
            influences: StorageBuffer::immutable(
                &[InfluenceEntry::zeroed()],
                c"deform influences (dummy)",
            )?,
            palette: StorageBuffer::immutable(
                &[PaletteEntry::zeroed()],
                c"deform palette (dummy)",
            )?,
            morph: StorageBuffer::immutable(&[MorphEntry::zeroed()], c"morph deltas (dummy)")?,
            weights: StorageBuffer::immutable(&[0.0f32], c"morph weights (dummy)")?,
        })
    }

    fn bind(&self, bindings: &mut Bindings) {
        bindings.storage(generated::VIEW_DEFORM_INFLUENCES, &self.influences);
        bindings.storage(generated::VIEW_DEFORM_PALETTE, &self.palette);
        bindings.storage(generated::VIEW_MORPH_DELTAS, &self.morph);
        bindings.storage(generated::VIEW_MORPH_WEIGHTS, &self.weights);
    }
}

/// The scene GPU resources, built once (lazily) on the first render. The offscreen
/// targets are recreated on resize; the mesh buffers when the model changes; the IBL
/// maps when the environment changes.
pub(crate) struct SceneGpu {
    /// Every pipeline that draws into the scene pass, held as the one set
    /// [`build_scene_pipelines`] returns.
    pub(super) scene: ScenePipelineSet,
    /// The composite, drawn into the frame's swapchain pass as a deferred job.
    composite: Pipeline,
    /// Linear clamp — the composite's input sampler and the IBL sampler.
    sampler: Sampler,
    /// Repeat sampler for the UV checker.
    checker_sampler: Sampler,
    /// The two baked UV-checker textures, picked per frame.
    checker_greyscale: Texture,
    checker_color: Texture,
    /// The image-based-lighting maps, reloaded on environment change.
    ibl: IblMaps,
    /// The editable per-material table.
    materials: MaterialTable,
    /// Stand-ins for the deform tables a model doesn't have.
    dummies: DeformDummies,
    /// The static reference grid (built once; model-independent).
    grid: VertexBuffer,
    /// The static 0..1 UV reference grid (built once; model-independent).
    uv_grid: VertexBuffer,
    /// Offscreen linear-HDR targets: location 0 scene colour, location 1 ambient.
    color: ColorTarget,
    ambient: ColorTarget,
    depth: DepthTarget,
    /// The last *requested* AA level, before capability clamping — cached so the
    /// (cheap, but per-frame) adapter query only runs when the request changes.
    requested_sample_count: u32,
    /// Everything cached for the model currently being drawn.
    pub(super) active: ModelSlot,
    /// The *other* model's cache, held so the Opt workspace can show two meshes
    /// without rebuilding either one every frame. See [`SceneGpu::activate`].
    pub(super) idle: ModelSlot,
    /// Which model `active` currently holds.
    active_slot: SlotId,
}

impl std::fmt::Debug for SceneGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneGpu").finish_non_exhaustive()
    }
}

impl SceneGpu {
    /// Build the scene GPU resources at `size` and `sample_count` MSAA. Called once,
    /// on the first 3D frame, at the live AA level — so the first frame needs no
    /// rebuild.
    pub(crate) fn new(size: (u32, u32), sample_count: u32) -> GpuResult<Self> {
        let (width, height) = size;
        let sample_count = sample_count.max(1);
        let composite_shader = shader::make(
            generated::post_shader_desc,
            shader::bytecode!("post"),
            c"composite",
        )?;
        Ok(Self {
            scene: build_scene_pipelines(sample_count)?,
            composite: Pipeline::new(&PipelineDesc::swapchain(composite_shader, c"composite"))?,
            sampler: Sampler::linear_clamp()?,
            checker_sampler: Sampler::linear_repeat()?,
            checker_greyscale: decode_checker(CHECKER_GREYSCALE_PNG)?,
            checker_color: decode_checker(CHECKER_COLOR_PNG)?,
            ibl: IblMaps::from_baked(EnvironmentSettings::default().map)?,
            materials: MaterialTable::new()?,
            dummies: DeformDummies::new()?,
            grid: VertexBuffer::new(&scene_lines(), c"grid")?,
            uv_grid: VertexBuffer::new(&uv_grid_lines(), c"uv grid")?,
            color: ColorTarget::hdr(width, height, sample_count, c"scene colour")?,
            ambient: ColorTarget::hdr(width, height, sample_count, c"scene ambient")?,
            depth: DepthTarget::new(width, height, sample_count, c"scene depth")?,
            requested_sample_count: sample_count,
            active: ModelSlot::new(),
            idle: ModelSlot::new(),
            active_slot: SlotId::Source,
        })
    }

    /// Reconcile the offscreen targets and the scene pipelines with the size the
    /// scene renders at and the live MSAA level. Steady-state frames allocate
    /// nothing.
    ///
    /// `size` is not always the backbuffer's: the Opt split renders each half at half
    /// width so the composite maps its target onto its half of the backbuffer
    /// one-to-one instead of squashing a full-width image into it.
    fn sync_targets(
        &mut self,
        frame: &Frame<'_>,
        size: (u32, u32),
        sample_count: u32,
    ) -> GpuResult<()> {
        let (width, height) = (size.0.max(1), size.1.max(1));
        // Only re-ask the adapter when the *request* moved (invariant 4): an
        // unsupported level degrades to the nearest supported one rather than failing
        // target creation on every frame.
        let requested = sample_count.max(1);
        let sample_count = if requested == self.requested_sample_count {
            self.color.sample_count()
        } else {
            frame.clamp_msaa(requested)
        };
        let size_changed = self.color.size() != (width, height);
        let samples_changed = self.color.sample_count() != sample_count;
        if !size_changed && !samples_changed {
            self.requested_sample_count = requested;
            return Ok(());
        }

        // Pipelines before targets. Their sample count is baked at creation but they
        // own nothing the targets depend on, so a failure here leaves every resource
        // and every field describing the level still in force, and the next frame
        // simply tries again. Recreating the targets first would strand them at the
        // new level with pipelines built for the old one.
        if samples_changed {
            self.scene = build_scene_pipelines(sample_count)?;
        }
        self.color = ColorTarget::hdr(width, height, sample_count, c"scene colour")?;
        self.ambient = ColorTarget::hdr(width, height, sample_count, c"scene ambient")?;
        self.depth = DepthTarget::new(width, height, sample_count, c"scene depth")?;
        // Written only once everything above exists: caching the request past a
        // failure would report the level as satisfied and leave it silently dropped
        // until the user changed it again.
        self.requested_sample_count = requested;
        Ok(())
    }

    /// Render the scene: skybox + mesh (per-material PBR/IBL) + grid + overlays into
    /// the offscreen HDR MRT, then queue the composite (tone map + sRGB, over the
    /// viewport background) into the frame's swapchain pass.
    pub(crate) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        camera: OrbitCamera,
    ) -> GpuResult<()> {
        // Every workspace but Opt draws the source model, and Opt may have left the
        // processed slot active. Without this the source mesh would be synced *into*
        // the processed slot — rebuilding both, and discarding the processed cache on
        // every switch between workspaces.
        self.activate(SlotId::Source);

        let size = frame.size();
        self.sync_frame(frame, scene, material_states, material_revision, size)?;
        self.record_view(frame, scene, camera, BackbufferRect::full(size))
    }

    /// Draw the active slot's model into `dest`: the scene pass, then the composite.
    /// Assumes [`Self::sync_frame`] has already run for this model.
    pub(super) fn record_view(
        &mut self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        camera: OrbitCamera,
        dest: BackbufferRect,
    ) -> GpuResult<()> {
        let uniforms = scene_uniforms(
            camera,
            scene.projection,
            scene.environment,
            scene.selection,
            scene.debug,
            self.active.deform_enabled(),
        );
        self.record_scene_pass(frame, scene, &uniforms);
        self.queue_composite(
            frame,
            &post_uniforms(scene.background, scene.tonemap, flat_display(scene)),
            dest,
        );
        Ok(())
    }

    /// Make `slot` the active one. A no-op when it already is; otherwise a single
    /// `mem::swap`, which is why alternating between two models within a frame costs
    /// nothing.
    pub(super) fn activate(&mut self, slot: SlotId) {
        if self.active_slot == slot {
            return;
        }
        std::mem::swap(&mut self.active, &mut self.idle);
        self.active_slot = slot;
    }

    /// Drop the processed model's cached buffers (invariant 3), whichever slot
    /// currently holds them.
    pub(crate) fn release_processed(&mut self) {
        if self.active_slot == SlotId::Processed {
            self.active.release();
        } else {
            self.idle.release();
        }
    }

    /// Reconcile every GPU resource with this frame's inputs: the offscreen targets,
    /// the Unique-mode part key, the mesh buffers + effective material table, the
    /// pose, the derived line views, the selection / visibility draw lists, and the
    /// IBL maps.
    pub(super) fn sync_frame(
        &mut self,
        frame: &Frame<'_>,
        scene: &SceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        target_size: (u32, u32),
    ) -> GpuResult<()> {
        self.sync_targets(
            frame,
            target_size,
            scene.anti_aliasing.effective_sample_count(),
        )?;

        // Every caller of this is a 3D frame, so the UV viewport's buffers are off
        // (invariant 3); `render_uv` builds them instead and never comes through here.
        self.release_uv_views();

        // The Unique-mode part key, then the mesh + the effective material table
        // (both depend on the active material mode's grouping).
        self.sync_unique_parts(scene.model, scene.model_revision, scene.debug.material_mode);
        self.sync_mesh(
            scene.model,
            scene.model_revision,
            scene.debug.uv_channel,
            scene.debug.material_mode,
        )?;
        let effective = effective_materials(
            scene.debug.material_mode,
            material_states,
            self.active.unique_part_count,
        );
        self.materials
            .sync(&effective, material_revision, scene.debug.material_mode)?;

        // The pose (palette + shape weights) the vertex shader deforms with, uploaded
        // only when its revision moves; then build-on-demand / free-on-off for the
        // derived overlays (invariant 3).
        self.sync_pose(scene.pose, scene.pose_revision)?;
        self.sync_line_views(
            scene.model,
            scene.model_revision,
            scene.debug,
            scene.hidden_meshes,
            scene.scene_bounds,
        )?;
        self.sync_skeleton(
            scene.model,
            scene.model_revision,
            scene.debug,
            scene.selected_bones,
        )?;
        self.sync_skin_weights(
            scene.model,
            scene.model_revision,
            scene.debug,
            scene.selected_bones,
        )?;
        self.sync_selection(
            scene.model,
            scene.model_revision,
            scene.selection,
            scene.hidden_meshes,
            scene.debug.material_mode,
        )?;
        self.sync_visibility(
            scene.model,
            scene.model_revision,
            scene.hidden_meshes,
            scene.debug.material_mode,
        )?;

        // Reload the IBL maps when the chosen environment changes (a pure upload).
        if self.ibl.environment != scene.environment.map {
            self.ibl = IblMaps::from_baked(scene.environment.map)?;
        }
        Ok(())
    }

    /// The bindings a draw running `fs_main` needs: the checker, the four IBL maps,
    /// one material's seven slots, the three samplers, and the deform tables.
    fn mesh_bindings(&self, checker: &Texture, material: &MaterialEntry) -> Bindings {
        let mut bindings = Bindings::new();
        bindings.texture(generated::VIEW_CHECKER_TEXTURE, checker);
        bindings.sampler(generated::SMP_CHECKER_SAMPLER, &self.checker_sampler);
        self.ibl.bind_shading(&mut bindings);
        bindings.sampler(generated::SMP_IBL_SAMPLER, &self.sampler);
        material.bind_textures(&mut bindings);
        bindings.sampler(generated::SMP_MATERIAL_SAMPLER, self.materials.sampler());
        self.bind_deform(&mut bindings);
        bindings
    }

    /// The bindings a draw running `fs_line` or `fs_selection` needs: nothing but the
    /// deform tables `vs_main` declares. Binding a texture here would be a validation
    /// error, not a harmless extra.
    fn line_bindings(&self) -> Bindings {
        let mut bindings = Bindings::new();
        self.bind_deform(&mut bindings);
        bindings
    }

    /// Fill the four deform slots: the dummies first, then whatever real tables the
    /// active model has over the top.
    fn bind_deform(&self, bindings: &mut Bindings) {
        self.dummies.bind(bindings);
        if let Some(deform) = self.active_deform() {
            deform.bind(bindings);
        }
    }

    /// The active model's deform tables, if it has any.
    fn active_deform(&self) -> Option<&DeformGpu> {
        self.active
            .mesh
            .as_ref()
            .and_then(|mesh| mesh.deform.as_ref())
    }

    /// The offscreen 2-MRT scene pass: skybox, the mesh draw list, the grid, the
    /// derived line overlays, the pivot marker, the skeleton and the selection flash.
    fn record_scene_pass(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        uniforms: &SceneUniforms,
    ) {
        let debug = scene.debug;
        let selection = scene.selection;

        // Clear the scene colour + ambient to zero radiance *and* zero alpha: the
        // alpha is the composite's coverage mask, so the cleared background reads as
        // "no geometry" and the post pass paints the chosen viewport background there
        // (in display space, after tone mapping).
        frame.begin_offscreen_pass(
            &[&self.color, &self.ambient],
            Some(&self.depth),
            [0.0; 4],
            c"scene",
        );

        let checker = match debug.uv_checker_texture {
            CheckerTexture::Greyscale => &self.checker_greyscale,
            CheckerTexture::Color => &self.checker_color,
        };

        // Skybox background first, behind all geometry, when shown. It declares only
        // the environment cube and the IBL sampler, so that is all it binds.
        if scene.environment.show_background {
            let mut bindings = Bindings::new();
            self.ibl.bind_env(&mut bindings);
            bindings.sampler(generated::SMP_IBL_SAMPLER, &self.sampler);
            frame.apply_pipeline(&self.scene.skybox);
            frame.apply_bindings(&bindings);
            // `vs_skybox` builds its ray from the fragment block alone, so the vertex
            // block is not declared here and applying it would fail validation.
            frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
            frame.draw(0, FULLSCREEN_VERTICES);
        }

        // Mesh draw list, in precedence order: solo (isolate the selection) wins;
        // otherwise per-mesh visibility (the filtered list, present only while some
        // mesh is hidden); otherwise the whole mesh. All three share the mesh vertex
        // buffer, so only the index source + ranges differ. Wireframe shading draws no
        // filled surface.
        let solo = selection.solo && selection.selection.is_active();
        if let Some(mesh) = &self.active.mesh
            && !matches!(debug.shading_mode, ShadingMode::Wireframe)
        {
            let pipeline = if debug.render_backfaces {
                &self.scene.mesh_double_sided
            } else {
                &self.scene.mesh
            };
            // The skin-weight heat map is a drop-in replacement for the mesh's vertex
            // buffer: same length, same order, so the index buffer, the per-material
            // ranges and the solo / visibility lists below all stay valid. It keeps
            // the real normals (the shader Lambert-shades it) and is selected by
            // `projection_params.w`, so the material bound per range is simply
            // ignored.
            let vertices = self
                .active
                .views
                .weights_buf
                .as_ref()
                .unwrap_or(&mesh.vertices);
            // Solo draws only the selection (empty → nothing); visible draws the
            // filtered list (`None` while active means every mesh is hidden → nothing);
            // otherwise the whole mesh.
            let draw_list: Option<(&IndexBuffer, &[_])> = if solo {
                self.active
                    .selection_index
                    .as_ref()
                    .map(|index| (index, self.active.selection_ranges.as_slice()))
            } else if self.active.visible_active {
                self.active
                    .visible_index
                    .as_ref()
                    .map(|index| (index, self.active.visible_ranges.as_slice()))
            } else {
                Some((&mesh.indices, mesh.ranges.as_slice()))
            };
            if let Some((index_buffer, ranges)) = draw_list {
                frame.apply_pipeline(pipeline);
                frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
                frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
                for range in ranges {
                    let material = self.materials.entry(range.material);
                    let mut bindings = self.mesh_bindings(checker, material);
                    bindings.mesh_vertices(vertices);
                    bindings.mesh_indices(index_buffer);
                    frame.apply_bindings(&bindings);
                    frame.apply_uniforms(generated::UB_MATERIAL, material.uniform());
                    frame.draw(range.first_index as usize, range.index_count as usize);
                }
            }
        }

        // The static grid, then the derived line overlays (wireframe / bounding box /
        // face+vertex normals) on top of the mesh. All share the line pipeline
        // (depth-tested Reversed-Z `GreaterEqual`, no depth write — the mesh pushed
        // its surface back so coplanar edges win). Each buffer is `None` while its
        // view is off.
        let mut line_views: Vec<&VertexBuffer> = Vec::new();
        if debug.show_grid {
            line_views.push(&self.grid);
        }
        line_views.extend(
            [
                &self.active.views.wireframe_buf,
                &self.active.views.bounding_box_buf,
                &self.active.views.face_normal_buf,
                &self.active.views.vertex_normal_buf,
            ]
            .into_iter()
            .flatten(),
        );
        self.draw_lines(frame, &self.scene.line, &line_views, uniforms);

        // Pivot marker + the skeleton's outlines: the always-on-top line pipeline
        // (depth compare `Always`), so they read *through* the mesh instead of being
        // occluded inside it.
        let overlay_lines: Vec<&VertexBuffer> = [
            &self.active.views.pivot_buf,
            &self.active.views.skeleton_line_buf,
        ]
        .into_iter()
        .flatten()
        .collect();

        // Skeleton fills go first, under their own outlines: translucent octahedra,
        // with the per-bone selection tint already baked into the buffer.
        if let Some(fill) = &self.active.views.skeleton_fill_buf {
            let mut bindings = self.mesh_bindings(checker, self.materials.fallback());
            bindings.mesh_vertices(fill);
            frame.apply_pipeline(&self.scene.fill_overlay);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
            frame.apply_uniforms(generated::UB_MATERIAL, self.materials.fallback().uniform());
            frame.draw(0, fill.count());
        }
        self.draw_lines(frame, &self.scene.line_overlay, &overlay_lines, uniforms);

        // Selection highlight flash: a flat bright-colour fill redrawing the selected
        // triangles over the mesh, fading out after a selection change. Drawn last so
        // it sits on top; reuses the selection index buffer over the shared mesh
        // vertex buffer, and `fs_selection` tints it with the uniform highlight colour
        // × the flash fade. Skipped once faded, so the steady state pays nothing.
        let flash = selection.selection.is_active() && selection.fade > 0.0;
        if flash
            && let (Some(mesh), Some(index)) = (&self.active.mesh, &self.active.selection_index)
        {
            let mut bindings = self.line_bindings();
            bindings.mesh_vertices(&mesh.vertices);
            bindings.mesh_indices(index);
            frame.apply_pipeline(&self.scene.selection);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
            frame.draw(0, index.count());
        }

        frame.end_pass();
    }

    /// Draw a set of line buffers through one pipeline. They share everything but the
    /// vertex stream, so the pipeline and the scene uniforms are applied once.
    fn draw_lines(
        &self,
        frame: &mut Frame<'_>,
        pipeline: &Pipeline,
        buffers: &[&VertexBuffer],
        uniforms: &SceneUniforms,
    ) {
        if buffers.is_empty() {
            return;
        }
        frame.apply_pipeline(pipeline);
        // `fs_line` reads nothing from the fragment block, so shdc strips it and only
        // the vertex block is declared.
        frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
        for buffer in buffers {
            let mut bindings = self.line_bindings();
            bindings.mesh_vertices(buffer);
            frame.apply_bindings(&bindings);
            frame.draw(0, buffer.count());
        }
    }

    /// Queue the composite into the frame's swapchain pass: the AO-darkened ambient +
    /// tone map + sRGB encode over the viewport background, into `dest`.
    ///
    /// Deferred rather than drawn, because that pass has not opened yet and there is
    /// only one of it per frame (`mac-port-plan.md` §3.2). The GTAO slot is bound with
    /// the ambient target as a harmless placeholder — the shader ignores it while
    /// `gtao_enabled` is 0, but every declared view must still be bound.
    fn queue_composite(&self, frame: &mut Frame<'_>, post: &PostUniforms, dest: BackbufferRect) {
        let mut bindings = Bindings::new();
        bindings.target(generated::VIEW_SCENE_COLOR, &self.color);
        bindings.target(generated::VIEW_GTAO_TEXTURE, &self.ambient);
        bindings.target(generated::VIEW_AMBIENT_TEXTURE, &self.ambient);
        bindings.sampler(generated::SMP_SCENE_SAMPLER, &self.sampler);
        frame.queue(
            SwapchainJob::new(
                &self.composite,
                FULLSCREEN_VERTICES,
                generated::UB_POST_PARAMS,
                post,
            )
            .with_bindings(&bindings)
            .with_viewport(dest.x, dest.y, dest.width, dest.height),
        );
    }

    /// Render the 2D UV viewport (instead of the 3D scene): the 0..1 grid, the
    /// optional island fill (Shaded / Islands modes), then the model's UV edges on
    /// top — all framed by the 2D `uv_camera` and composited like the 3D scene
    /// (tone-mapped, no GTAO).
    // Independent per-frame inputs (frame + model + revision + camera + channel +
    // shading mode + background); none is redundant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_uv(
        &mut self,
        frame: &mut Frame<'_>,
        model: &ModelData,
        model_revision: u64,
        uv_camera: UvCamera,
        channel: u32,
        shading_mode: UvShadingMode,
        anti_aliasing: AntiAliasing,
        background: ViewportBackground,
    ) -> GpuResult<()> {
        // The UV viewport shows the source model, so its derived buffers belong in the
        // source slot (see the note in `render`).
        self.activate(SlotId::Source);
        let size = frame.size();
        self.sync_targets(frame, size, anti_aliasing.effective_sample_count())?;
        self.sync_uv_view(model, model_revision, channel, shading_mode)?;

        // The UV camera's orthographic view-projection; the rest of the uniform is
        // unused by the UV path (lines and fills return their own vertex colour).
        let uniforms = uv_scene_uniforms(uv_camera);

        // Clear to zero (radiance + coverage); the background is painted in the
        // composite, matching the 3D path.
        frame.begin_offscreen_pass(
            &[&self.color, &self.ambient],
            Some(&self.depth),
            [0.0; 4],
            c"uv scene",
        );

        // Reference grid first.
        self.draw_lines(frame, &self.scene.line, &[&self.uv_grid], &uniforms);

        // Island fill (Shaded / Islands), under the wireframe. It runs `fs_main`, so
        // it binds the full material set even though the zero-normal branch it takes
        // never uses the sampled values.
        if let Some(fill) = &self.active.views.uv_fill_buf {
            let material = self.materials.fallback();
            let mut bindings = self.mesh_bindings(&self.checker_greyscale, material);
            bindings.mesh_vertices(fill);
            frame.apply_pipeline(&self.scene.uv_fill);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, &uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, &uniforms);
            frame.apply_uniforms(generated::UB_MATERIAL, material.uniform());
            frame.draw(0, fill.count());
        }

        // The model's UV edges on top.
        if let Some(wireframe) = &self.active.views.uv_wireframe_buf {
            self.draw_lines(frame, &self.scene.line, &[wireframe], &uniforms);
        }
        frame.end_pass();

        // Composite: tone-mapped (the default operator, so shaded fills read like the
        // 3D scene), no GTAO (the flat UV viewport has no depth to occlude), over the
        // chosen viewport background.
        let post = post_uniforms(background, TonemapSettings::default(), false);
        self.queue_composite(frame, &post, BackbufferRect::full(size));
        Ok(())
    }
}

/// Decode a baked UV-checker PNG into an sRGB GPU texture. A decode failure is a
/// packaging bug — fall back to a 1×1 white texel rather than failing the build of
/// the scene resources.
fn decode_checker(png_bytes: &[u8]) -> GpuResult<Texture> {
    match image::load_from_memory(png_bytes) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Texture::immutable_2d(&rgba, width, height, Format::Rgba8Srgb, c"uv checker")
        }
        Err(error) => {
            // Degrading to flat white is deliberate, but not silently: a corrupt baked
            // checker is a packaging bug worth seeing under `--tracy`.
            crate::rhi::gpu_profiler::note(&format!(
                "baked UV-checker PNG failed to decode: {error}"
            ));
            Texture::immutable_2d(
                &[255, 255, 255, 255],
                1,
                1,
                Format::Rgba8Srgb,
                c"uv checker",
            )
        }
    }
}

/// Whether this frame is one of the flat data-inspection views, which emit final
/// display pixels from the scene shader.
///
/// They bypass lighting, the composite's tone map and GTAO entirely — a value shown
/// through a tone curve is no longer the value — so the composite blits straight
/// through and the occlusion passes are skipped.
fn flat_display(frame: &SceneFrame<'_>) -> bool {
    matches!(
        frame.debug.active_material,
        ActiveMaterial::Buffers | ActiveMaterial::SkinWeights
    )
}

/// Build the composite pass's [`PostUniforms`] from the live settings.
fn post_uniforms(
    background: ViewportBackground,
    tonemap: TonemapSettings,
    passthrough: bool,
) -> PostUniforms {
    let (top, bottom) = background.gradient_srgb();
    PostUniforms {
        // GTAO returns with its own stage (`mac-port-plan.md` Phase 1 step 4).
        gtao_enabled: 0,
        tonemap_enabled: u32::from(tonemap.enabled),
        tonemap_op: tonemap.operator.shader_index(),
        passthrough: u32::from(passthrough),
        bg_top: [top[0], top[1], top[2], 0.0],
        bg_bottom: [bottom[0], bottom[1], bottom[2], 0.0],
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera, projection, environment,
/// selection and debug options. The selection flash rides in `selection_color`
/// (gamma-space rgb + the flash fade in alpha, zero while nothing is
/// selected/flashing), read only by `fs_selection`.
pub(super) fn scene_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    environment: EnvironmentSettings,
    selection: SelectionView,
    debug: SceneDebugOptions,
    deform: bool,
) -> SceneUniforms {
    let view_projection = camera.view_projection(projection);
    let selection_color = if selection.selection.is_active() {
        let [r, g, b, _] = selection.highlight_color;
        [r, g, b, selection.fade.clamp(0.0, 1.0)]
    } else {
        [0.0; 4]
    };
    SceneUniforms {
        view_projection: view_projection.to_cols_array_2d(),
        inv_view_projection: view_projection.inverse().to_cols_array_2d(),
        render_options: [
            shading_mode_value(debug.shading_mode),
            if debug.active_material == ActiveMaterial::UvChecker {
                1.0
            } else {
                0.0
            },
            debug.uv_checker_tiling.max(1) as f32,
            vertex_color_value(debug),
        ],
        // `w` enables the vertex-stage deform path (skinning + blend shapes).
        camera_position: camera
            .eye_position()
            .extend(if deform { 1.0 } else { 0.0 })
            .to_array(),
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
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            environment.rotation_degrees.to_radians(),
            // `z` carries the active buffer-inspection view index (-1 when off).
            buffer_view_value(debug),
            // `w` flags the skin-weight heat map.
            skin_weight_value(debug),
        ],
        view: camera.view_matrix().to_cols_array_2d(),
        selection_color,
    }
}

/// Build the [`SceneUniforms`] for the 2D UV viewport: only the UV camera's
/// orthographic view-projection matters (the grid / wireframe / fill return their own
/// vertex colour, never reaching the IBL / shading / selection code).
/// `projection_params.x = 1.0` marks orthographic.
fn uv_scene_uniforms(uv_camera: UvCamera) -> SceneUniforms {
    SceneUniforms {
        view_projection: uv_camera.view_projection().to_cols_array_2d(),
        inv_view_projection: [[0.0; 4]; 4],
        render_options: [0.0; 4],
        camera_position: [0.0; 4],
        env_params: [0.0; 4],
        projection_params: [1.0, 0.0, 0.0, 0.0],
        view: [[0.0; 4]; 4],
        selection_color: [0.0; 4],
    }
}
