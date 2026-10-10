//! The scene renderer. It owns the GPU-side scene resources (pipelines, buffers,
//! offscreen targets, IBL maps, material table) built lazily on the first frame and
//! drives the per-frame render: the scene draws into offscreen linear-HDR MRT
//! targets, and a composite job tone-maps + sRGB-encodes them into the frame's
//! swapchain pass behind the egui chrome.
//!
//! The full shaded look is the skybox + the per-material PBR/IBL `fs_main` (the
//! material uniform block + its seven texture slots + the IBL maps + the UV checker),
//! drawn one indexed range per material, plus the live tone-map operator switch, the
//! debug line views and the selection highlight.
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
//!   highlight colour, takes both.
//! * `vs_main` declares all four deform tables, so every program sharing it must
//!   bind all four. That is what [`DeformDummies`] is for: a model without a skin
//!   has none of them.
//!
//! ## Ambient occlusion
//!
//! GTAO is a run of offscreen passes before the composite (see [`super::gtao`] for the
//! sequence), and it reads a G-buffer of its own rather than the scene MRT: a
//! **single-sample** mesh-only pass writes the view normal and view Z, so MSAA edge
//! averaging never blends a normal across a silhouette. The occlusion and denoise
//! passes are fullscreen `R16F`, and the composite darkens only the AO-eligible ambient
//! attachment by the result — an additive correction over the radiance, so direct and
//! emissive light are never darkened.
//!
//! The pass sequencing lives here; the pieces it sequences are beside it:
//! [`super::targets`] owns the attachments, [`super::gtao`] the occlusion
//! passes, [`super::draw`] the draw verbs, [`super::uv`] the UV viewport, and
//! [`super::uniforms`] the per-frame uniform blocks.

use bytemuck::Zeroable;

use crate::geometry::{scene_lines, uv_grid_lines};
use crate::ibl::IblMaps;
use crate::material::{MaterialKey, MaterialState, MaterialTable, effective_materials};
use crate::rhi::{
    Bindings, Cull, DEPTH_MIP_COLORS, Depth, DepthBias, Format, Frame, GBUFFER_COLORS, GpuResult,
    OCCLUSION_COLORS, Pipeline, PipelineDesc, Sampler, StorageBuffer, Texture, Topology,
    VertexBuffer, Zone, shader,
};
use crate::shaders::generated;
use crate::{
    CheckerTexture, EnvironmentSettings, GhostStyle, OrbitCamera, SceneFrame, ShadingMode,
};

use super::gpu_types::{InfluenceEntry, MorphEntry, PaletteEntry, SceneUniforms};
use super::opt::ghost_tint;
use super::pipelines::{
    LinePipelineSet, SCENE_VERTEX_LAYOUT, ScenePipelineSet, build_line_pipelines,
    build_scene_pipelines,
};
use super::slot::{ModelSlot, SlotId};
use super::targets::{BackbufferRect, TargetSet, TargetSetId};
use super::uniforms::{LineWidths, flat_display, post_uniforms, scene_uniforms};

/// The greyscale + colour UV-checker PNGs (baked in; invariant: assets via
/// `include_bytes!`), uploaded once as sRGB textures and picked per frame.
pub(super) const CHECKER_GREYSCALE_PNG: &[u8] =
    include_bytes!("../../../../assets/textures/T_UV_Checker_BW.png");

pub(super) const CHECKER_COLOR_PNG: &[u8] =
    include_bytes!("../../../../assets/textures/T_UV_Checker_CLR.png");

/// Vertices in a fullscreen triangle — the skybox and the composite, both of which
/// build their corners from `gl_VertexIndex`.
pub(super) const FULLSCREEN_VERTICES: usize = 3;

/// One-element stand-ins for the four deform tables.
///
/// `vs_main` declares all four storage buffers, and sokol validates that a draw binds
/// every view its shader declared — so a static model, or a skinned one whose morph
/// table is empty, still has to bind *something*. These are that something: 4, 48, 28
/// and 4 bytes of zeroes, never read (the uniform's deform flag is off, and an empty
/// deform lane makes the shader skip them anyway).
pub(super) struct DeformDummies {
    pub(super) influences: StorageBuffer<InfluenceEntry>,
    pub(super) palette: StorageBuffer<PaletteEntry>,
    pub(super) morph: StorageBuffer<MorphEntry>,
    pub(super) weights: StorageBuffer<f32>,
}

impl DeformDummies {
    pub(super) fn new() -> GpuResult<Self> {
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

    pub(super) fn bind(&self, bindings: &mut Bindings) {
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
    /// Every pipeline that draws into the multisampled scene pass, held as the one
    /// set [`build_scene_pipelines`] returns.
    pub(super) scene: ScenePipelineSet,
    /// Every pipeline that draws into the single-sample line pass. Built once:
    /// unlike [`Self::scene`], nothing in it depends on the AA level.
    pub(super) lines: LinePipelineSet,
    /// The composite, drawn into the frame's swapchain pass as a deferred job.
    pub(super) composite: Pipeline,
    /// Mesh-only single-sample view-normal/Z G-buffer pass.
    pub(super) gtao_gbuffer_pipeline: Pipeline,
    /// One level of the depth prefilter chain, fullscreen.
    pub(super) gtao_depth_mip_pipeline: Pipeline,
    /// Horizon-based occlusion, fullscreen.
    pub(super) gtao_pipeline: Pipeline,
    /// 5×5 edge-aware denoise, fullscreen, run once to three times.
    pub(super) gtao_denoise_pipeline: Pipeline,
    /// Linear clamp — the composite's input sampler and the IBL sampler.
    pub(super) sampler: Sampler,
    /// Repeat sampler for the UV checker.
    pub(super) checker_sampler: Sampler,
    /// Point-clamp sampler for the GTAO passes, so view normals and depths are never
    /// blended across geometry edges.
    pub(super) gtao_sampler: Sampler,
    /// The two baked UV-checker textures, picked per frame.
    pub(super) checker_greyscale: Texture,
    pub(super) checker_color: Texture,
    /// The image-based-lighting maps, reloaded on environment change.
    pub(super) ibl: IblMaps,
    /// The editable per-material table.
    pub(super) materials: MaterialTable,
    /// Stand-ins for the deform tables a model doesn't have.
    pub(super) dummies: DeformDummies,
    /// The static reference grid (built once; model-independent).
    pub(super) grid: VertexBuffer,
    /// The static 0..1 UV reference grid (built once; model-independent).
    pub(super) uv_grid: VertexBuffer,
    /// The image the UV viewport lays its islands over, uploaded while it is on
    /// screen and dropped with the other UV views (invariant 3).
    pub(super) uv_texture: Option<super::uv::UvTextureGpu>,
    /// The UV viewport's checkerboard background, while it is the one picked.
    pub(super) uv_checker: Option<super::uv::UvCheckerGpu>,
    /// Everything one view renders through.
    pub(super) targets: TargetSet,
    /// A second set for the Opt split's right-hand view, at the same half-width size
    /// as the first — so the two together cost what one full-width set would.
    ///
    /// It exists because the composite is a *deferred* job (`docs/ARCHITECTURE.md`,
    /// Platform decisions: one swapchain pass): both halves' passes have run by the
    /// time either composite does, so a shared set would show the second view's
    /// contents in both. `None` outside the split (invariant 3).
    pub(super) split_targets: Option<TargetSet>,
    /// The MSAA level the pipelines are built for.
    pub(super) sample_count: u32,
    /// The last *requested* AA level, before capability clamping — cached so the
    /// (cheap, but per-frame) adapter query only runs when the request changes.
    pub(super) requested_sample_count: u32,
    /// Everything cached for the model currently being drawn.
    pub(super) active: ModelSlot,
    /// The *other* model's cache, held so the Opt workspace can show two meshes
    /// without rebuilding either one every frame. See [`SceneGpu::activate`].
    pub(super) idle: ModelSlot,
    /// Which model `active` currently holds.
    pub(super) active_slot: SlotId,
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
        // The G-buffer shares `vs_main` with the mesh, so it takes the same vertex
        // layout and indexed draws; it culls nothing (a back face still occludes) and
        // writes its own depth.
        let gtao_gbuffer_pipeline = Pipeline::new(&PipelineDesc {
            attributes: &SCENE_VERTEX_LAYOUT,
            indexed: true,
            topology: Topology::Triangles,
            cull: Cull::None,
            depth: Depth::WRITE,
            depth_bias: DepthBias::default(),
            depth_format: Some(crate::rhi::SCENE_DEPTH_FORMAT),
            ..PipelineDesc::offscreen(
                shader::make(
                    generated::gtao_gbuffer_shader_desc,
                    shader::bytecode!("gtao_gbuffer"),
                    c"gtao gbuffer",
                )?,
                GBUFFER_COLORS,
                c"gtao gbuffer",
            )
        })?;
        let gtao_pipeline = Pipeline::new(&PipelineDesc::offscreen(
            shader::make(
                generated::gtao_shader_desc,
                shader::bytecode!("gtao"),
                c"gtao",
            )?,
            OCCLUSION_COLORS,
            c"gtao",
        ))?;
        let gtao_denoise_pipeline = Pipeline::new(&PipelineDesc::offscreen(
            shader::make(
                generated::gtao_denoise_shader_desc,
                shader::bytecode!("gtao_denoise"),
                c"gtao denoise",
            )?,
            OCCLUSION_COLORS,
            c"gtao denoise",
        ))?;
        let gtao_depth_mip_pipeline = Pipeline::new(&PipelineDesc::offscreen(
            shader::make(
                generated::gtao_depth_mip_shader_desc,
                shader::bytecode!("gtao_depth_mip"),
                c"gtao depth mip",
            )?,
            DEPTH_MIP_COLORS,
            c"gtao depth mip",
        ))?;

        Ok(Self {
            scene: build_scene_pipelines(sample_count)?,
            lines: build_line_pipelines()?,
            composite: Pipeline::new(&PipelineDesc::swapchain(composite_shader, c"composite"))?,
            gtao_gbuffer_pipeline,
            gtao_depth_mip_pipeline,
            gtao_pipeline,
            gtao_denoise_pipeline,
            sampler: Sampler::linear_clamp()?,
            checker_sampler: Sampler::linear_repeat()?,
            gtao_sampler: Sampler::point_clamp()?,
            checker_greyscale: decode_checker(CHECKER_GREYSCALE_PNG)?,
            checker_color: decode_checker(CHECKER_COLOR_PNG)?,
            ibl: IblMaps::from_baked(EnvironmentSettings::default().map)?,
            materials: MaterialTable::new()?,
            dummies: DeformDummies::new()?,
            // Pullable: `vs_line` reads each line's ends through a storage view.
            grid: VertexBuffer::pullable(&scene_lines(), c"grid")?,
            uv_grid: VertexBuffer::pullable(&uv_grid_lines(), c"uv grid")?,
            uv_texture: None,
            uv_checker: None,
            targets: TargetSet::new(width, height, sample_count)?,
            split_targets: None,
            sample_count,
            requested_sample_count: sample_count,
            active: ModelSlot::new(),
            idle: ModelSlot::new(),
            active_slot: SlotId::Source,
        })
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
        // One view, so what only the Opt comparison draws with goes (invariant 3).
        self.release_opt_views();

        let size = frame.size();
        self.sync_frame(frame, scene, material_states, material_revision, size)?;
        self.sync_ao_history(TargetSetId::Primary, camera, scene);
        self.record_view(
            frame,
            scene,
            camera,
            &self.targets,
            BackbufferRect::full(size),
        )
    }

    /// Draw the active slot's model into `dest`: the scene pass, then the composite.
    /// Assumes [`Self::sync_frame`] has already run for this model.
    pub(super) fn record_view(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        camera: OrbitCamera,
        targets: &TargetSet,
        dest: BackbufferRect,
    ) -> GpuResult<()> {
        self.record_view_with_ghost(frame, scene, camera, targets, dest, None)
    }

    /// [`Self::record_view`], optionally drawing the *idle* slot's mesh as a ghost
    /// inside the same scene pass — the Opt workspace's overlay comparison.
    pub(super) fn record_view_with_ghost(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        camera: OrbitCamera,
        targets: &TargetSet,
        dest: BackbufferRect,
        ghost: Option<(GhostStyle, [f32; 3])>,
    ) -> GpuResult<()> {
        let uniforms = scene_uniforms(
            camera,
            scene.projection,
            scene.environment,
            scene.selection,
            scene.debug,
            self.active.deform_enabled(),
        );
        let lines = LineWidths::new(scene, targets.color.size());
        let gtao_active = self.gtao_active(scene);
        self.record_scene_pass(
            frame,
            scene,
            targets,
            &uniforms,
            &lines,
            ghost.map(|(style, tint)| {
                // The ghost's own uniform: the same camera and projection, with the flat
                // fill colour swapped in. Under sokol nothing has to be *restored*
                // afterwards — uniforms are applied per draw, so the next draw's own call
                // is the restore.
                //
                // The ghost is the idle slot's mesh in its bind pose: the Opt workspace
                // compares static geometry, so neither mesh deforms there.
                let mut ghost_uniforms = scene_uniforms(
                    camera,
                    scene.projection,
                    scene.environment,
                    scene.selection,
                    scene.debug,
                    false,
                );
                ghost_uniforms.selection_color = ghost_tint(style, tint);
                (style, ghost_uniforms)
            }),
        );
        // The occlusion hands back the target it landed in, rather than the composite
        // going looking for it: which of the denoise ping-pongs holds the result
        // depends on the pass count, and on a converged frame on the pass count of the
        // run that last actually happened.
        let ao = gtao_active.then(|| {
            self.record_gtao(
                frame,
                camera,
                scene.projection,
                scene.gtao,
                targets,
                &uniforms,
            )
        });
        self.queue_composite(
            frame,
            &post_uniforms(
                scene.background,
                gtao_active,
                scene.tonemap,
                flat_display(scene),
            ),
            targets,
            ao,
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

    /// Drop what only the Opt comparison draws with (invariant 3): the split's
    /// second target set (a full MSAA HDR set with its G-buffer and AO chain), both
    /// slots' ghost wireframes, and everything derived from the processed mesh.
    /// The processed mesh itself stays uploaded, so returning to Opt redraws it
    /// without a rebuild.
    ///
    /// Called by every path that is not Opt - the 3D scene, the UV viewport and
    /// the Tex viewport - since leaving the workspace is not an event the
    /// renderer sees, only a frame that draws something else.
    pub(crate) fn release_opt_views(&mut self) {
        self.release_split_targets();
        self.release_ghost_wireframes();
        match self.active_slot {
            SlotId::Processed => self.active.release_derived(),
            SlotId::Source => self.idle.release_derived(),
        }
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
        let material_key = MaterialKey::new(
            material_revision,
            scene.debug.material_mode,
            self.active.unique_part_count,
        );
        if !self.materials.is_current(material_key) {
            let effective = effective_materials(
                scene.debug.material_mode,
                material_states,
                self.active.unique_part_count,
            );
            self.materials.sync(&effective, material_key)?;
        }

        // The pose (palette + shape weights) the vertex shader deforms with, uploaded
        // only when its revision moves; then build-on-demand / free-on-off for the
        // derived overlays (invariant 3).
        self.sync_pose(scene.pose, scene.pose_revision)?;
        self.sync_line_views(
            scene.model,
            scene.model_revision,
            scene.debug,
            scene.hidden_meshes,
            scene.selected_nodes,
            scene.scene_bounds,
        )?;
        self.sync_skeleton(
            scene.model,
            scene.model_revision,
            scene.debug,
            scene.selected_bones,
            scene.hover_bone,
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
            scene.selected_nodes,
            scene.hidden_meshes,
            scene.debug.material_mode,
        )?;
        self.sync_hover(
            scene.model,
            scene.model_revision,
            scene.hover,
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

    /// The offscreen 2-MRT scene pass — skybox, the mesh draw list, the hover and
    /// selection fills, the x-ray ghost — and then the line views over it.
    ///
    /// The lines antialias their own edges, so they gain nothing from MSAA and pay a
    /// blend per sample for every pixel they cover, which a dense wireframe makes a
    /// great many. At 2×+ they therefore go in a pass of their own after this one has
    /// resolved ([`Self::record_line_pass`]); at 1× this pass is single-sample anyway
    /// and they are simply drawn last in it. Either way they come after every fill,
    /// so the two paths look the same.
    pub(super) fn record_scene_pass(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        targets: &TargetSet,
        uniforms: &SceneUniforms,
        lines: &LineWidths,
        ghost: Option<(GhostStyle, SceneUniforms)>,
    ) {
        // Clear the scene colour + ambient to zero radiance *and* zero alpha: the
        // alpha is the composite's coverage mask, so the cleared background reads as
        // "no geometry" and the post pass paints the chosen viewport background there
        // (in display space, after tone mapping).
        frame.zone_begin(Zone::Scene);
        frame.begin_offscreen_pass(
            &[&targets.color, &targets.ambient],
            Some(&targets.depth),
            [0.0; 4],
            c"scene",
        );

        let checker = match scene.debug.uv_checker_texture {
            CheckerTexture::Greyscale => &self.checker_greyscale,
            CheckerTexture::Color => &self.checker_color,
        };

        // Back to front: the skybox behind everything, the mesh, the hover and
        // selection fills over it, then the ghost.
        if scene.environment.show_background {
            self.draw_skybox(frame, uniforms);
        }
        self.draw_mesh_list(frame, scene, checker, uniforms);
        self.draw_highlights(frame, scene, uniforms);
        let wireframe_ghost = match &ghost {
            Some((GhostStyle::Xray, ghost_uniforms)) => {
                self.draw_xray_ghost(frame, ghost_uniforms);
                None
            }
            Some((GhostStyle::Wireframe, ghost_uniforms)) => Some(ghost_uniforms),
            None => None,
        };

        let views = LineViews {
            scene,
            checker,
            uniforms,
            lines,
            wireframe_ghost,
        };
        if targets.color.sample_count() > 1 {
            frame.end_pass();
            if self.has_line_views(&views) {
                self.record_line_pass(frame, targets, &views);
            }
        } else {
            self.draw_line_views(frame, &views);
            frame.end_pass();
        }
        frame.zone_end(Zone::Scene);
    }

    /// The single-sample pass the lines draw in when the scene pass was
    /// multisampled: over the scene's *resolved* colour and ambient, against a
    /// depth-only redraw of the mesh in the scratch depth — the multisampled depth
    /// cannot be read, and a line has to be hidden behind the surface just as it was.
    ///
    /// The one thing given up is the occlusion's antialiasing: where a line passes
    /// behind the mesh's silhouette it is now cut on whole pixels rather than on
    /// samples. A line's *own* edges are unaffected — the shader smooths those.
    fn record_line_pass(&self, frame: &mut Frame<'_>, targets: &TargetSet, views: &LineViews<'_>) {
        frame.begin_resolved_pass(
            &[&targets.color, &targets.ambient],
            &targets.scratch_depth,
            c"lines",
        );
        self.draw_depth_prepass(frame, views.scene, views.uniforms);
        self.draw_line_views(frame, views);
        frame.end_pass();
    }

    /// Whether this frame draws any line at all — so that a view with the grid off
    /// and nothing else switched on does not pay for a line pass with nothing in it.
    fn has_line_views(&self, views: &LineViews<'_>) -> bool {
        let debug = views.scene.debug;
        let derived = &self.active.views;
        (debug.show_grid)
            || (wireframe_on(views.scene) && derived.wireframe_edges.is_some())
            || derived.bounding_box_buf.is_some()
            || derived.face_normal_buf.is_some()
            || derived.vertex_normal_buf.is_some()
            || derived.uv_seam_buf.is_some()
            || derived.pivot_buf.is_some()
            || derived.skeleton_fill_buf.is_some()
            || derived.skeleton_line_buf.is_some()
            || (views.wireframe_ghost.is_some() && self.idle.ghost_wireframe_edges.is_some())
    }

    /// The skybox, behind all geometry.
    fn draw_skybox(&self, frame: &mut Frame<'_>, uniforms: &SceneUniforms) {
        // Skybox background first, behind all geometry, when shown. It declares only
        // the environment cube and the IBL sampler, so that is all it binds.
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

    /// The mesh itself, one draw per material range, from whichever index list
    /// is in effect ([`Self::mesh_draw_list`]).
    fn draw_mesh_list(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        checker: &Texture,
        uniforms: &SceneUniforms,
    ) {
        let debug = scene.debug;
        let (Some(mesh), Some((index_buffer, ranges))) =
            (&self.active.mesh, self.mesh_draw_list(scene))
        else {
            return;
        };
        let pipeline = if debug.render_backfaces {
            &self.scene.mesh_double_sided
        } else {
            &self.scene.mesh
        };
        // The skin-weight heat map is a drop-in replacement for the mesh's vertex
        // buffer: same length, same order, so the index buffer, the per-material
        // ranges and the solo / visibility lists all stay valid. It keeps the real
        // normals (the shader Lambert-shades it) and is selected by
        // `projection_params.w`, so the material bound per range is simply ignored.
        let vertices = self
            .active
            .views
            .weights_buf
            .as_ref()
            .unwrap_or(&mesh.vertices);
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

    /// The grid, the wireframe and every derived line view, then the skeleton and
    /// the always-on-top markers, then the Opt overlay's wireframe ghost — all
    /// through the single-sample line pipelines, into whichever pass the caller has
    /// open (see [`Self::record_scene_pass`]).
    fn draw_line_views(&self, frame: &mut Frame<'_>, views: &LineViews<'_>) {
        let &LineViews {
            scene,
            checker,
            uniforms,
            lines,
            wireframe_ghost,
        } = views;
        let debug = scene.debug;
        // The static grid, then the derived line overlays (wireframe / bounding box /
        // face+vertex normals / UV seams) on top of the mesh. All are depth-tested
        // (Reversed-Z `GreaterEqual`, no depth write — the mesh pushed its surface back
        // so coplanar edges win). Each buffer is `None` while its view is off. Seams go
        // last: they sit exactly on wireframe edges, and with equal depth and no depth
        // write the later draw is the one that shows.
        if debug.show_grid {
            self.draw_lines(
                frame,
                &self.lines.line,
                [&self.grid],
                uniforms,
                &lines.overlay,
            );
        }
        // The wireframe sits between the grid and the rest: it is the one line view
        // drawn over the mesh vertex buffer, on its own pipeline, at its own width.
        if wireframe_on(scene) {
            self.draw_wireframe(frame, uniforms, &lines.wireframe, debug.wireframe_color);
        }
        let line_views = [
            &self.active.views.bounding_box_buf,
            &self.active.views.face_normal_buf,
            &self.active.views.vertex_normal_buf,
            &self.active.views.uv_seam_buf,
        ];
        self.draw_lines(
            frame,
            &self.lines.line,
            line_views.into_iter().flatten(),
            uniforms,
            &lines.overlay,
        );

        // Skeleton fills go first, under their own outlines: translucent octahedra,
        // with the per-bone selection tint already baked into the buffer.
        if let Some(fill) = &self.active.views.skeleton_fill_buf {
            let mut bindings = self.mesh_bindings(checker, self.materials.fallback());
            bindings.mesh_vertices(fill);
            frame.apply_pipeline(&self.lines.fill_overlay);
            frame.apply_bindings(&bindings);
            frame.apply_uniforms(generated::UB_SCENE_VS, uniforms);
            frame.apply_uniforms(generated::UB_SCENE_FS, uniforms);
            frame.apply_uniforms(generated::UB_MATERIAL, self.materials.fallback().uniform());
            frame.draw(0, fill.count());
        }
        // Pivot marker + the skeleton's outlines: the always-on-top line pipeline
        // (depth compare `Always`), so they read *through* the mesh instead of being
        // occluded inside it.
        let overlay_lines = [
            &self.active.views.pivot_buf,
            &self.active.views.skeleton_line_buf,
        ];
        self.draw_lines(
            frame,
            &self.lines.line_overlay,
            overlay_lines.into_iter().flatten(),
            uniforms,
            &lines.overlay,
        );

        // The Opt overlay's wireframe ghost, over everything, at the wireframe's width.
        if let Some(ghost_uniforms) = wireframe_ghost {
            self.draw_wireframe_ghost(frame, ghost_uniforms, &lines.wireframe);
        }
    }

    /// The hover preview and the selection highlight, over the mesh.
    fn draw_highlights(
        &self,
        frame: &mut Frame<'_>,
        scene: &SceneFrame<'_>,
        uniforms: &SceneUniforms,
    ) {
        let debug = scene.debug;
        let selection = scene.selection;
        // Hover preview, then the selection highlight over it: two flat-colour
        // fills redrawing those triangles on top of the mesh, each through
        // `fs_selection` with its own tint in the uniform. Selection goes last so
        // it wins where the two meet.
        //
        // The highlight *persists* for as long as something is selected, so
        // unlike the flash it used to be this is not skipped once any animation
        // ends — only an alpha of zero skips it, which is the case where there
        // would be nothing to see.
        if let (Some(mesh), Some(index)) = (&self.active.mesh, &self.active.hover_index) {
            self.draw_highlight(frame, index, &mesh.vertices, uniforms, debug.hover_color);
        }
        if selection.selection.is_active()
            && uniforms.selection_color[3] > 0.0
            && let (Some(mesh), Some(index)) = (&self.active.mesh, &self.active.selection_index)
        {
            self.draw_highlight(
                frame,
                index,
                &mesh.vertices,
                uniforms,
                uniforms.selection_color,
            );
        }
    }
}

/// What drawing one view's lines needs, gathered once so the inline (1×) and
/// separate-pass (2×+) paths of [`SceneGpu::record_scene_pass`] are handed the same
/// thing.
pub(super) struct LineViews<'a> {
    pub(super) scene: &'a SceneFrame<'a>,
    pub(super) checker: &'a Texture,
    pub(super) uniforms: &'a SceneUniforms,
    pub(super) lines: &'a LineWidths,
    /// The Opt overlay's ghost uniforms (its tint in `selection_color`), when the
    /// ghost is drawn as a wireframe.
    pub(super) wireframe_ghost: Option<&'a SceneUniforms>,
}

/// Whether the model wireframe is drawn: the overlay toggle, or the Wireframe-only
/// shading mode, which draws the edges as the only geometry.
pub(super) fn wireframe_on(scene: &SceneFrame<'_>) -> bool {
    scene.debug.wireframe_overlay || matches!(scene.debug.shading_mode, ShadingMode::Wireframe)
}

/// Decode a baked UV-checker PNG into an sRGB GPU texture. A decode failure is a
/// packaging bug — fall back to a 1×1 white texel rather than failing the build of
/// the scene resources.
pub(super) fn decode_checker(png_bytes: &[u8]) -> GpuResult<Texture> {
    match image::load_from_memory(png_bytes) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Texture::immutable_2d(&rgba, width, height, Format::Rgba8Srgb, c"uv checker")
        }
        Err(error) => {
            // Degrading to flat white is deliberate, but not silently: a corrupt baked
            // checker is a packaging bug worth seeing in the log.
            log::warn!("baked UV-checker PNG failed to decode: {error}");
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
