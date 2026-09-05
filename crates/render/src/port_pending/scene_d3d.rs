//! The Direct3D 11 scene renderer. It owns the GPU-side scene resources (pipelines,
//! buffers, offscreen targets, IBL maps, material table) built lazily on the first
//! frame and drives the per-frame render: the scene draws into offscreen linear-HDR
//! MRT targets, then a composite pass tone-maps + sRGB-encodes them to the swapchain
//! backbuffer behind the egui chrome.
//!
//! The full shaded look is the skybox + the per-material PBR/IBL `fs_main` (material
//! cbuffer `b1` + the seven texture slots `t5..t11` + the IBL maps `t1..t4` + the UV
//! checker `t0`), drawn one indexed range per material, plus GTAO + the live tone-map
//! operator switch, the UV / Tex viewports, the debug line views and the selection
//! flash.

use review_model::ModelData;

use crate::geometry::{scene_lines, uv_grid_lines};
use crate::ibl::{IblD3d, PREFILTER_MAX_LOD};
use crate::material::{MaterialDrawRange, MaterialState, MaterialTableD3d, effective_materials};
use crate::rhi::{
    BlendMode, ColorTarget, Cull, DepthBias, DepthCompare, DepthState, DepthTarget,
    DynamicConstantBuffer, Gpu, GpuResult, IndexBuffer, Pipeline, PipelineDesc, Sampler, Texture,
    Topology, VertexBuffer,
};
use crate::selection::SelectionView;
use crate::{
    ActiveMaterial, AntiAliasing, CameraProjection, CheckerTexture, EnvironmentSettings,
    GhostStyle, GtaoSettings, OrbitCamera, SceneDebugOptions, SceneFrame, ShadingMode,
    TonemapSettings, UvCamera, UvShadingMode, ViewportBackground,
};

use super::gpu_types::{
    GtaoUniforms, PostUniforms, SceneUniforms, buffer_view_value, shading_mode_value,
    skin_weight_value, vertex_color_value,
};
use super::pipelines::{SCENE_VERTEX_LAYOUT, SCENE_VS, ScenePipelineSet, build_scene_pipelines};
use super::resources::{DEFORM_INFLUENCES_SLOT, DEFORM_SLOT_COUNT, ModelSlot, SlotId};
use crate::rhi::gpu_profiler::{self, GpuProfiler, Zone};

/// Compiled DXBC — see `build.rs`. (The scene pipelines' own shaders live in
/// [`super::pipelines`].)
const SCENE_GTAO_GBUFFER_PS: &[u8] = include_bytes!("../hlsl/scene.gtao_gbuffer.ps.dxbc");
const GTAO_VS: &[u8] = include_bytes!("../hlsl/gtao.vs.dxbc");
const GTAO_PS: &[u8] = include_bytes!("../hlsl/gtao.ps.dxbc");
const GTAO_BLUR_PS: &[u8] = include_bytes!("../hlsl/gtao.blur.ps.dxbc");
const POST_VS: &[u8] = include_bytes!("../hlsl/post.vs.dxbc");
const POST_PS: &[u8] = include_bytes!("../hlsl/post.ps.dxbc");

/// The greyscale + color UV-checker PNGs (baked in; invariant: assets via
/// `include_bytes!`), uploaded once as sRGB textures and picked per frame.
const CHECKER_GREYSCALE_PNG: &[u8] =
    include_bytes!("../../../../assets/textures/T_UV_Checker_BW.png");
const CHECKER_COLOR_PNG: &[u8] = include_bytes!("../../../../assets/textures/T_UV_Checker_CLR.png");

/// Constant-buffer registers, one per struct across the whole scene/GTAO/post
/// shader set (the register plan in `scene.hlsl`; the material's `b1` is named in
/// `material/d3d.rs`). They are distinct because a pass leaves its cbuffers bound
/// across pass boundaries — the GTAO fullscreen passes run with `SceneUniforms`
/// still on the vertex stage from the G-buffer draw — so sharing a slot would have
/// one stage read another struct's bytes the moment a shader started reading it.
const SCENE_CBUFFER_SLOT: u32 = 0;
const GTAO_CBUFFER_SLOT: u32 = 2;
const POST_CBUFFER_SLOT: u32 = 3;

/// Expand a [`ViewportBackground`]'s display-space top/bottom colors into the
/// `bg_top` / `bg_bottom` cbuffer fields (`xyz` color, `w` unused).
fn background_uniforms(background: ViewportBackground) -> ([f32; 4], [f32; 4]) {
    let (top, bottom) = background.gradient_srgb();
    (
        [top[0], top[1], top[2], 0.0],
        [bottom[0], bottom[1], bottom[2], 0.0],
    )
}

/// Build the composite pass's [`PostUniforms`] from the live settings.
fn post_uniforms(
    background: ViewportBackground,
    gtao_active: bool,
    tonemap: TonemapSettings,
    passthrough: bool,
) -> PostUniforms {
    let (bg_top, bg_bottom) = background_uniforms(background);
    PostUniforms {
        gtao_enabled: u32::from(gtao_active),
        tonemap_enabled: u32::from(tonemap.enabled),
        tonemap_op: tonemap.operator.shader_index(),
        passthrough: u32::from(passthrough),
        bg_top,
        bg_bottom,
    }
}

/// A rectangle of the backbuffer for the composite to write into. The whole
/// backbuffer for a single view; one half of it for each side of the Opt
/// workspace's split.
#[derive(Debug, Clone, Copy)]
pub(super) struct BackbufferRect {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl BackbufferRect {
    pub(super) fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// The Direct3D 11 scene GPU resources, built once (lazily) on the first `render`.
/// The offscreen targets + depth are recreated on resize; the mesh buffers when the
/// model changes; the IBL maps when the environment changes.
pub(crate) struct SceneGpu {
    /// Shared per-frame scene uniforms (cbuffer `b0`).
    pub(super) uniforms: DynamicConstantBuffer,
    /// Composite-pass uniform (cbuffer `b3` in the post shader).
    post_uniforms: DynamicConstantBuffer,
    /// GTAO-pass uniform (cbuffer `b2` in the gtao shader).
    gtao_uniforms: DynamicConstantBuffer,
    /// Every MSAA-dependent scene pipeline, held as the one set
    /// [`build_scene_pipelines`] returns rather than unpacked into fields: an AA
    /// change replaces all of them at once, and a pipeline left behind by a missed
    /// assignment would stay baked at the startup sample count — a debug-layer
    /// error and undefined rendering, with nothing to catch it at compile time.
    pub(super) scene: ScenePipelineSet,
    composite_pipeline: Pipeline,
    /// Mesh-only single-sample view-normal/Z G-buffer pipeline (`fs_gtao_gbuffer`).
    gtao_gbuffer_pipeline: Pipeline,
    /// Horizon-based occlusion fullscreen pass (`fs_gtao`).
    gtao_pipeline: Pipeline,
    /// 5×5 bilateral-blur fullscreen pass (`fs_blur`).
    gtao_blur_pipeline: Pipeline,
    /// Scene MSAA sample count the MSAA-dependent pipelines + targets are built for
    /// (1 = no multisampling). Rebuilt when the live AA level changes.
    scene_sample_count: u32,
    /// The last *requested* AA level, before capability clamping — cached so the
    /// (cheap but per-frame) `CheckMultisampleQualityLevels` re-clamp only runs
    /// when the request actually changes.
    requested_sample_count: u32,
    /// Linear clamp sampler — the composite's input sampler (`s0` of the post pass)
    /// and the IBL sampler (`s1` of the scene pass).
    sampler: Sampler,
    /// Repeat sampler for the UV checker (`s0` of the scene pass).
    checker_sampler: Sampler,
    /// Point-clamp sampler for the GTAO passes (`s0`), so view normals / depths are
    /// never blended across geometry edges.
    gtao_sampler: Sampler,
    /// The two baked UV-checker textures (`t0`), picked per frame.
    checker_greyscale: Texture,
    checker_color: Texture,
    /// The image-based-lighting maps (`t1..t4`), reloaded on environment change.
    ibl: IblD3d,
    /// The editable per-material table (`b1` + `t5..t11` + `s2`).
    materials: MaterialTableD3d,
    /// The static reference grid (built once; model-independent).
    grid: VertexBuffer,
    /// Offscreen linear-HDR targets: location 0 scene color, location 1 ambient.
    color: ColorTarget,
    ambient: ColorTarget,
    depth: DepthTarget,
    /// GTAO targets, all full-resolution + recreated on resize: the single-sample
    /// view-normal/Z G-buffer (HDR) + its own depth, then the raw and blurred
    /// occlusion (`R8`). Sized to the framebuffer like the scene targets.
    gtao_gbuffer: ColorTarget,
    gtao_depth: DepthTarget,
    gtao_raw: ColorTarget,
    gtao_blur: ColorTarget,
    /// Everything cached for the model currently being drawn.
    pub(super) active: ModelSlot,
    /// The *other* model's cache, held so the Opt workspace can show two meshes
    /// without rebuilding either one every frame. See [`SceneGpu::activate`].
    pub(super) idle: ModelSlot,
    /// Which model `active` currently holds.
    active_slot: SlotId,
    // --- 2D UV viewport (drawn instead of the 3D scene in UV workspace mode). ---
    /// The static 0..1 reference grid (built once; model-independent).
    uv_grid: VertexBuffer,
    /// Hand-rolled D3D11 timestamp-query → Tracy GPU profiler. `None` unless `--tracy`
    /// armed it and a Tracy client is running; built lazily on the first profiled
    /// frame and never touched otherwise (every scene pass records no timestamps).
    gpu_profiler: Option<GpuProfiler>,
    /// Building the profiler failed once — don't re-attempt its `RING × (SLOTS+1)`
    /// `CreateQuery` calls every frame on a gpu that keeps refusing them.
    gpu_profiler_failed: bool,
}

impl std::fmt::Debug for SceneGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneGpu").finish_non_exhaustive()
    }
}

impl SceneGpu {
    /// Build the scene GPU resources. Called once on the first frame; `sample_count`
    /// is the initial scene MSAA level (the live AA setting), so the scene pipelines +
    /// targets start at the right level and the first frame needs no rebuild.
    pub(crate) fn new(gpu: &Gpu, sample_count: u32) -> GpuResult<Self> {
        let (width, height) = gpu.size();
        // Capability-gate the initial level too (invariant 4): a persisted AA
        // setting restored on a weaker adapter degrades instead of failing.
        let requested_sample_count = sample_count.max(1);
        let sample_count = gpu.clamp_msaa(requested_sample_count);

        let uniforms = DynamicConstantBuffer::new::<SceneUniforms>(gpu)?;
        let post_uniforms = DynamicConstantBuffer::new::<PostUniforms>(gpu)?;
        let gtao_uniforms = DynamicConstantBuffer::new::<GtaoUniforms>(gpu)?;

        // The MSAA-dependent scene pipelines (line / mesh / double-sided mesh /
        // skybox / UV fill / selection fill) — all draw into the MSAA scene MRT, so
        // their sample count is baked at the live AA level and rebuilt when it changes
        // ([`Self::rebuild_scene_pipelines`]).
        let scene = build_scene_pipelines(gpu, sample_count)?;

        // The composite: a fullscreen triangle, opaque overwrite of the backbuffer.
        // Always single-sample — it draws to the backbuffer, not the MSAA MRT.
        let composite_pipeline = Pipeline::new(
            gpu,
            &PipelineDesc::fullscreen(POST_VS, POST_PS, BlendMode::Opaque),
        )?;

        // GTAO G-buffer: a mesh-only pass writing the single-sample view normal/Z.
        // No culling (backfaces still occlude), writes + tests depth (Reversed-Z
        // `GreaterEqual`), no bias, opaque single target.
        let gtao_gbuffer_pipeline = Pipeline::new(
            gpu,
            &PipelineDesc {
                vs: SCENE_VS,
                ps: SCENE_GTAO_GBUFFER_PS,
                input: &SCENE_VERTEX_LAYOUT,
                topology: Topology::TriangleList,
                cull: Cull::None,
                depth: DepthState {
                    test: true,
                    write: true,
                    compare: DepthCompare::GreaterEqual,
                },
                blend: BlendMode::Opaque,
                depth_bias: DepthBias::default(),
                sample_count: 1,
            },
        )?;

        // The two GTAO fullscreen passes (occlusion + bilateral blur): opaque
        // writes into the `R8` AO targets, differing only in the PS.
        let gtao_pipeline = Pipeline::new(
            gpu,
            &PipelineDesc::fullscreen(GTAO_VS, GTAO_PS, BlendMode::Opaque),
        )?;
        let gtao_blur_pipeline = Pipeline::new(
            gpu,
            &PipelineDesc::fullscreen(GTAO_VS, GTAO_BLUR_PS, BlendMode::Opaque),
        )?;

        let sampler = Sampler::linear_clamp(gpu)?;
        let checker_sampler = Sampler::linear_repeat(gpu)?;
        let gtao_sampler = Sampler::point_clamp(gpu)?;
        let checker_greyscale = decode_checker(gpu, CHECKER_GREYSCALE_PNG)?;
        let checker_color = decode_checker(gpu, CHECKER_COLOR_PNG)?;
        let ibl = IblD3d::from_baked(gpu, EnvironmentSettings::default().map)?;
        let materials = MaterialTableD3d::new(gpu)?;
        let grid = VertexBuffer::new(gpu, &scene_lines())?;
        let uv_grid = VertexBuffer::new(gpu, &uv_grid_lines())?;
        // The scene MRT + depth carry the MSAA level; the GTAO targets stay single-
        // sample (its own mesh-only pass, never resolved).
        let color = ColorTarget::hdr_msaa(gpu, width, height, sample_count)?;
        let ambient = ColorTarget::hdr_msaa(gpu, width, height, sample_count)?;
        let depth = DepthTarget::with_samples(gpu, width, height, sample_count)?;
        let gtao_gbuffer = ColorTarget::new(gpu, width, height)?;
        let gtao_depth = DepthTarget::new(gpu, width, height)?;
        let gtao_raw = ColorTarget::r8(gpu, width, height)?;
        let gtao_blur = ColorTarget::r8(gpu, width, height)?;

        Ok(Self {
            uniforms,
            post_uniforms,
            gtao_uniforms,
            scene,
            composite_pipeline,
            gtao_gbuffer_pipeline,
            gtao_pipeline,
            gtao_blur_pipeline,
            scene_sample_count: sample_count,
            requested_sample_count,
            sampler,
            checker_sampler,
            gtao_sampler,
            checker_greyscale,
            checker_color,
            ibl,
            materials,
            grid,
            color,
            ambient,
            depth,
            gtao_gbuffer,
            gtao_depth,
            gtao_raw,
            gtao_blur,
            active: ModelSlot::new(),
            idle: ModelSlot::new(),
            active_slot: SlotId::Source,
            uv_grid,
            gpu_profiler: None,
            gpu_profiler_failed: false,
        })
    }

    /// Record a profiled zone's begin timestamp (a no-op without `--tracy`).
    fn zone_begin(&self, gpu: &Gpu, zone: Zone) {
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_begin(gpu, zone);
        }
    }

    /// Record a profiled zone's end timestamp (a no-op without `--tracy`).
    fn zone_end(&self, gpu: &Gpu, zone: Zone) {
        if let Some(profiler) = self.gpu_profiler.as_ref() {
            profiler.zone_end(gpu, zone);
        }
    }

    /// Reconcile the offscreen targets + scene pipelines with the backbuffer size +
    /// the scene MSAA level. The scene MRT + depth carry the MSAA level (recreated on
    /// a size *or* sample-count change); the GTAO targets stay single-sample (size
    /// only); the MSAA-dependent scene pipelines rebuild on a sample-count change
    /// (their sample count is baked at creation). Steady-state frames allocate
    /// nothing. Shared by the 3D scene + UV viewport paths.
    /// `size` is the offscreen resolution to render at, which is *not* always the
    /// backbuffer's: the Opt workspace's split view renders each half at half
    /// width so the composite maps its target onto its half of the backbuffer
    /// one-to-one instead of squashing a full-width image into it.
    fn sync_targets(&mut self, gpu: &Gpu, sample_count: u32, size: (u32, u32)) -> GpuResult<()> {
        let (width, height) = (size.0.max(1), size.1.max(1));
        // Capability-clamp a *changed* request (invariant 4: an unsupported level
        // — e.g. restored settings on a weaker adapter — degrades to the nearest
        // supported one rather than failing target creation every frame).
        let requested = sample_count.max(1);
        let sample_count = if requested == self.requested_sample_count {
            self.scene_sample_count
        } else {
            gpu.clamp_msaa(requested)
        };
        let size_changed = self.color.size() != (width, height);
        let samples_changed = self.scene_sample_count != sample_count;

        // Pipelines before targets. Their sample count is baked at creation but they
        // own nothing the targets depend on, so a failure here leaves every resource
        // and every field describing the level still in force, and the next frame
        // simply tries again. Recreating the targets first would strand them at the
        // new level with pipelines built for the old one.
        if samples_changed {
            self.rebuild_scene_pipelines(gpu, sample_count)?;
        }
        if size_changed || samples_changed {
            self.color = ColorTarget::hdr_msaa(gpu, width, height, sample_count)?;
            self.ambient = ColorTarget::hdr_msaa(gpu, width, height, sample_count)?;
            self.depth = DepthTarget::with_samples(gpu, width, height, sample_count)?;
        }
        if size_changed {
            self.gtao_gbuffer = ColorTarget::new(gpu, width, height)?;
            self.gtao_depth = DepthTarget::new(gpu, width, height)?;
            self.gtao_raw = ColorTarget::r8(gpu, width, height)?;
            self.gtao_blur = ColorTarget::r8(gpu, width, height)?;
        }
        // Both fields are the record of what was actually built, so they are written
        // only once everything above exists. `requested_sample_count` in particular
        // is what suppresses the re-clamp next frame — caching it past a failure
        // would report the level as satisfied and leave the request silently dropped
        // until the user changed it again.
        self.scene_sample_count = sample_count;
        self.requested_sample_count = requested;
        Ok(())
    }

    /// Rebuild the MSAA-dependent scene pipelines at `sample_count` (their sample
    /// count is baked into the rasterizer + must match the MSAA targets). The
    /// composite + GTAO pipelines are single-sample and untouched.
    fn rebuild_scene_pipelines(&mut self, gpu: &Gpu, sample_count: u32) -> GpuResult<()> {
        self.scene = build_scene_pipelines(gpu, sample_count)?;
        Ok(())
    }

    /// Render the scene: skybox + mesh (per-material PBR/IBL) + grid into the
    /// offscreen HDR MRT, then — when GTAO is on — a single-sample G-buffer +
    /// horizon occlusion + bilateral blur, and finally a composite (ambient-only AO
    /// darkening + tone map + sRGB) to the backbuffer. The egui chrome is drawn on
    /// top afterwards by `app`.
    pub(crate) fn render(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        camera: OrbitCamera,
    ) -> GpuResult<()> {
        // Every workspace but Opt draws the source model, and Opt may have left
        // the processed slot active. Without this the source mesh would be synced
        // *into* the processed slot — rebuilding both, and discarding the
        // processed cache on every switch between workspaces.
        self.activate(SlotId::Source);

        let size = gpu.size();
        self.sync_frame(gpu, frame, material_states, material_revision, size)?;

        let gtao_active = self.gtao_active(frame);
        self.begin_gpu_frame(gpu, gtao_active);
        let result = self.record_view(gpu, frame, camera, gtao_active, BackbufferRect::full(size));
        self.end_gpu_frame(gpu);
        result
    }

    /// Whether GTAO should run for this frame: enabled, a mesh is present, and the
    /// view isn't one of the flat data-inspection ones.
    pub(super) fn gtao_active(&self, frame: &SceneFrame<'_>) -> bool {
        frame.gtao.enabled && self.active.has_mesh() && !flat_display(frame)
    }

    /// Arm the GPU profiler (lazily, only under `--tracy` with a running client)
    /// and open this frame's timing window. Absent on a normal launch, so the
    /// passes record no timestamps. A failed build is reported once and never
    /// retried (44 `CreateQuery` calls per frame otherwise).
    pub(super) fn begin_gpu_frame(&mut self, gpu: &Gpu, gtao_active: bool) {
        if self.gpu_profiler.is_none() && !self.gpu_profiler_failed && gpu_profiler::should_enable()
        {
            match GpuProfiler::new(gpu) {
                Ok(profiler) => self.gpu_profiler = Some(profiler),
                Err(error) => {
                    self.gpu_profiler_failed = true;
                    gpu_profiler::note(&format!("GPU profiler unavailable: {error}"));
                }
            }
        }
        if let Some(profiler) = self.gpu_profiler.as_mut() {
            profiler.begin_frame(gpu, gpu_profiler::frame_mask(gtao_active));
        }
    }

    /// Close the GPU profiler's timing window (resolves + reads back a few frames
    /// later in `begin_frame`).
    pub(super) fn end_gpu_frame(&mut self, gpu: &Gpu) {
        if let Some(profiler) = self.gpu_profiler.as_mut() {
            profiler.end_frame(gpu);
        }
    }

    /// Draw the active slot's model into `dest`: the scene pass, GTAO when active,
    /// then the composite. Assumes [`Self::sync_frame`] has already run for this
    /// model.
    pub(super) fn record_view(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        camera: OrbitCamera,
        gtao_active: bool,
        dest: BackbufferRect,
    ) -> GpuResult<()> {
        self.record_view_with_ghost(gpu, frame, camera, gtao_active, dest, None)
    }

    /// [`Self::record_view`], optionally drawing the *idle* slot's mesh as a ghost
    /// inside the same scene pass.
    pub(super) fn record_view_with_ghost(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        camera: OrbitCamera,
        gtao_active: bool,
        dest: BackbufferRect,
        ghost: Option<(GhostStyle, [f32; 3])>,
    ) -> GpuResult<()> {
        let deform = self.active.deform_enabled();
        let uniforms = scene_uniforms(
            camera,
            frame.projection,
            frame.environment,
            frame.selection,
            frame.debug,
            deform,
        );
        self.uniforms.update(gpu, &uniforms)?;

        self.record_scene_pass(gpu, frame)?;
        if let Some((style, tint)) = ghost {
            self.record_ghost(gpu, camera, frame, style, tint)?;
        }

        if gtao_active {
            self.record_gtao(gpu, camera, frame.projection, frame.gtao)?;
        }

        // Composite to the backbuffer (ambient-only AO + tone map + sRGB, over the
        // viewport background). `t1` is the blurred GTAO when active; otherwise a
        // harmless placeholder (the shader ignores it when `gtao_enabled` is 0).
        let post = post_uniforms(
            frame.background,
            gtao_active,
            frame.tonemap,
            flat_display(frame),
        );
        self.zone_begin(gpu, Zone::Composite);
        self.record_composite(gpu, &post, gtao_active.then_some(&self.gtao_blur), dest)?;
        self.zone_end(gpu, Zone::Composite);
        // Release the deform tables so a model swap can drop them cleanly.
        gpu.unbind_vs_srvs(DEFORM_INFLUENCES_SLOT, DEFORM_SLOT_COUNT);
        Ok(())
    }

    /// Make `slot` the active one. A no-op when it already is; otherwise a single
    /// `mem::swap`, which is why alternating between two models within a frame
    /// costs nothing.
    pub(super) fn activate(&mut self, slot: SlotId) {
        if self.active_slot == slot {
            return;
        }
        std::mem::swap(&mut self.active, &mut self.idle);
        self.active_slot = slot;
    }

    /// Drop the processed model's cached buffers (invariant 3), whichever slot
    /// currently holds them. Called when the Opt workspace has no processed mesh
    /// to show, so its GPU memory isn't held while another workspace is up.
    pub(crate) fn release_processed(&mut self) {
        if self.active_slot == SlotId::Processed {
            self.active.release();
        } else {
            self.idle.release();
        }
    }

    /// Reconcile every GPU resource with this frame's inputs: the offscreen
    /// targets + scene pipelines (size / AA level), the Unique-mode part key, the
    /// mesh buffers + effective material table, the derived line views, the
    /// selection / visibility draw lists, and the IBL maps.
    pub(super) fn sync_frame(
        &mut self,
        gpu: &Gpu,
        frame: &SceneFrame<'_>,
        material_states: &[MaterialState],
        material_revision: u64,
        target_size: (u32, u32),
    ) -> GpuResult<()> {
        self.sync_targets(
            gpu,
            frame.anti_aliasing.effective_sample_count(),
            target_size,
        )?;

        // Every caller of this is a 3D frame, so the UV viewport's buffers are off
        // (invariant 3); `render_uv` builds them instead and never comes through here.
        self.release_uv_views();

        // The Unique-mode part key, then the mesh + the effective material table
        // (both depend on the active material mode's grouping).
        self.sync_unique_parts(frame.model, frame.model_revision, frame.debug.material_mode);
        self.sync_mesh(
            gpu,
            frame.model,
            frame.model_revision,
            frame.debug.uv_channel,
            frame.debug.material_mode,
        )?;
        let effective = effective_materials(
            frame.debug.material_mode,
            material_states,
            self.active.unique_part_count,
        );
        self.materials.sync(
            gpu,
            &effective,
            material_revision,
            frame.debug.material_mode,
        )?;

        // Build-on-demand / free-on-off for the derived 3D line overlays (invariant
        // 3): each view's buffer exists only while its toggle is on, rebuilt live
        // when its baked params (color / length / hidden set / scope) drift.
        // The pose (palette + shape weights) the vertex shader deforms with,
        // uploaded only when its revision moves.
        self.sync_pose(gpu, frame.pose, frame.pose_revision)?;
        self.sync_line_views(
            gpu,
            frame.model,
            frame.model_revision,
            frame.debug,
            frame.hidden_meshes,
            frame.scene_bounds,
        )?;
        self.sync_skeleton(
            gpu,
            frame.model,
            frame.model_revision,
            frame.debug,
            frame.selected_bones,
        )?;
        self.sync_skin_weights(
            gpu,
            frame.model,
            frame.model_revision,
            frame.debug,
            frame.selected_bones,
        )?;

        // Build (or free) the selected-triangle draw list (the solo isolate list +
        // the highlight-flash fill source) and the per-mesh visibility filter, when
        // the selection / hidden set / model / mode drifts (invariant 3).
        self.sync_selection(
            gpu,
            frame.model,
            frame.model_revision,
            frame.selection,
            frame.hidden_meshes,
            frame.debug.material_mode,
        )?;
        self.sync_visibility(
            gpu,
            frame.model,
            frame.model_revision,
            frame.hidden_meshes,
            frame.debug.material_mode,
        )?;

        // Reload the IBL maps when the chosen environment changes (a pure upload).
        if self.ibl.environment != frame.environment.map {
            self.ibl = IblD3d::from_baked(gpu, frame.environment.map)?;
        }
        Ok(())
    }

    /// Shared per-pass bindings for the scene shader: `b0` (VS + PS), the checker
    /// (`t0`/`s0`), the IBL maps (`t1..t4`) + their sampler (`s1`), and the
    /// material cbuffer + sampler (`b1`/`s2`). The mesh loop rebinds `b1` +
    /// `t5..t11` per range.
    fn bind_scene_shared(&self, gpu: &Gpu, checker: &Texture) -> GpuResult<()> {
        self.uniforms.bind_vs(gpu, SCENE_CBUFFER_SLOT);
        self.uniforms.bind_ps(gpu, SCENE_CBUFFER_SLOT);
        checker.bind_ps(gpu, 0);
        self.checker_sampler.bind_ps(gpu, 0);
        self.ibl.bind_ps(gpu);
        self.sampler.bind_ps(gpu, 1);
        self.materials.bind_shared(gpu);
        self.materials.bind_fallback(gpu)
    }

    /// The offscreen 2-MRT scene pass: skybox, the mesh draw list, the grid, the
    /// derived line overlays, the pivot marker and the selection flash.
    fn record_scene_pass(&self, gpu: &Gpu, frame: &SceneFrame<'_>) -> GpuResult<()> {
        let debug = frame.debug;
        let selection = frame.selection;

        // Clear the scene color + ambient to zero radiance *and* zero alpha: the
        // alpha is the composite's coverage mask, so the cleared background reads as
        // "no geometry" and the post pass paints the chosen viewport background
        // there (in display space, after tone mapping).
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, [0.0; 4]);
        self.zone_begin(gpu, Zone::Scene);
        let checker = match debug.uv_checker_texture {
            CheckerTexture::Greyscale => &self.checker_greyscale,
            CheckerTexture::Color => &self.checker_color,
        };
        self.bind_scene_shared(gpu, checker)?;
        // The deform tables, bound for the whole frame (the GTAO G-buffer pass
        // shares `vs_main` and reads them too); the uniform flag decides whether
        // the shader looks at them.
        if let Some(deform) = self
            .active
            .mesh
            .as_ref()
            .and_then(|mesh| mesh.deform.as_ref())
        {
            deform.bind_vs(gpu);
        }

        // Skybox background first, behind all geometry, when shown.
        if frame.environment.show_background {
            self.scene.skybox.bind(gpu);
            gpu.draw(3);
        }

        // Mesh draw list, in precedence order: solo (isolate the selection) wins;
        // otherwise per-mesh visibility (the filtered list, present only while some
        // mesh is hidden); otherwise the whole mesh. All three share the mesh vertex
        // buffer, so only the index source + ranges differ. Wireframe shading draws
        // no filled surface.
        let solo = selection.solo && selection.selection.is_active();
        if let Some(mesh) = &self.active.mesh
            && !matches!(debug.shading_mode, ShadingMode::Wireframe)
        {
            let pipeline = if debug.render_backfaces {
                &self.scene.mesh_double_sided
            } else {
                &self.scene.mesh
            };
            pipeline.bind(gpu);
            // The skin-weight heat map is a drop-in replacement for the mesh's
            // vertex buffer: same length, same order, so the index buffer, the
            // per-material ranges and the solo / visibility lists below all stay
            // valid. It keeps the real normals (the shader Lambert-shades it) and
            // is selected by `projection_params.w`, so the material bound per range
            // is simply ignored.
            let vertex_source = self
                .active
                .views
                .weights_buf
                .as_ref()
                .unwrap_or(&mesh.vertices);
            vertex_source.bind(gpu);
            // Solo draws only the selection (empty → nothing); visible draws the
            // filtered list (None while active means every mesh is hidden → nothing);
            // otherwise the whole mesh.
            let draw_list: Option<(&IndexBuffer, &[MaterialDrawRange])> = if solo {
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
                index_buffer.bind(gpu);
                for range in ranges {
                    self.materials.set_range(gpu, range.material)?;
                    gpu.draw_indexed_range(range.index_count, range.first_index);
                }
            }
        }

        if debug.show_grid {
            self.scene.line.bind(gpu);
            self.grid.bind(gpu);
            gpu.draw(self.grid.count());
        }

        // Derived line overlays (wireframe / bounding box / face+vertex normals),
        // drawn after the mesh + grid so they sit on top. All share the line
        // pipeline (depth-tested Reversed-Z `GreaterEqual`, no depth write — the
        // mesh pushed its surface back so coplanar edges win) + the scene uniforms
        // (`b0`, still bound). Each buffer is `None` while its view is off.
        let line_views = [
            &self.active.views.wireframe_buf,
            &self.active.views.bounding_box_buf,
            &self.active.views.face_normal_buf,
            &self.active.views.vertex_normal_buf,
        ];
        if line_views.iter().any(|view| view.is_some()) {
            self.scene.line.bind(gpu);
            for buffer in line_views.into_iter().flatten() {
                buffer.bind(gpu);
                gpu.draw(buffer.count());
            }
        }

        // Pivot marker: drawn last among the line overlays with the always-on-top
        // line pipeline (depth compare `Always`), so the 3-axis cross reads through
        // the mesh instead of being occluded inside it.
        if let Some(pivot) = &self.active.views.pivot_buf {
            self.scene.line_overlay.bind(gpu);
            pivot.bind(gpu);
            gpu.draw(pivot.count());
        }

        // Skeleton overlay: translucent octahedron fills first, then their opaque
        // outlines on top, both always-on-top so the rig reads through the
        // character it deforms. The per-bone selection tint is already baked into
        // these buffers (see `sync_skeleton`).
        if let Some(fill) = &self.active.views.skeleton_fill_buf {
            self.scene.fill_overlay.bind(gpu);
            fill.bind(gpu);
            gpu.draw(fill.count());
        }
        if let Some(lines) = &self.active.views.skeleton_line_buf {
            self.scene.line_overlay.bind(gpu);
            lines.bind(gpu);
            gpu.draw(lines.count());
        }

        // Selection highlight flash: a flat bright-color fill redrawing the selected
        // triangles over the mesh, fading out after a selection change. Drawn last in
        // the scene pass so it sits on top. Reuses the selection index buffer over the
        // shared mesh vertex buffer; `fs_selection` tints it with the uniform
        // highlight color × the flash fade (`selection_color`, already in `b0`).
        // Skipped once the flash has faded, so the steady state pays nothing.
        let flash = selection.selection.is_active() && selection.fade > 0.0;
        if flash
            && let (Some(mesh), Some(index)) = (&self.active.mesh, &self.active.selection_index)
        {
            self.scene.selection.bind(gpu);
            mesh.vertices.bind(gpu);
            index.bind(gpu);
            gpu.draw_indexed_range(index.count(), 0);
        }
        self.zone_end(gpu, Zone::Scene);
        Ok(())
    }

    /// The GTAO passes: a single-sample mesh-only G-buffer (view normal + Z), then
    /// the horizon occlusion pass (→ raw `R8`) and the bilateral blur (→ blurred
    /// `R8`) the composite darkens the ambient radiance by. The G-buffer has its
    /// own depth (nearest-surface) and shares `b0` (the scene uniforms carry
    /// `view`); the fullscreen passes read `b2` as the GTAO uniform + `s0` as the
    /// point sampler.
    fn record_gtao(
        &self,
        gpu: &Gpu,
        camera: OrbitCamera,
        projection: CameraProjection,
        gtao: GtaoSettings,
    ) -> GpuResult<()> {
        let gtao_uniforms = build_gtao_uniforms(camera, projection, gtao);
        self.gtao_uniforms.update(gpu, &gtao_uniforms)?;

        // G-buffer: redraw the whole mesh (material irrelevant) into the
        // single-sample normal/Z target, clearing the target + its depth.
        self.zone_begin(gpu, Zone::GtaoGbuffer);
        gpu.begin_scene_pass(&[&self.gtao_gbuffer], &self.gtao_depth, [0.0; 4]);
        self.uniforms.bind_vs(gpu, SCENE_CBUFFER_SLOT);
        self.uniforms.bind_ps(gpu, SCENE_CBUFFER_SLOT);
        self.gtao_gbuffer_pipeline.bind(gpu);
        if let Some(mesh) = &self.active.mesh {
            mesh.vertices.bind(gpu);
            // Match the shaded mesh's visibility so a hidden mesh casts no AO; solo
            // is deliberately left out, so only the per-mesh hide filters the AO.
            // `None` while active means every mesh is hidden → nothing to occlude.
            let index = if self.active.visible_active {
                self.active.visible_index.as_ref()
            } else {
                Some(&mesh.indices)
            };
            if let Some(index) = index {
                index.bind(gpu);
                gpu.draw_indexed_range(index.count(), 0);
            }
        }
        self.zone_end(gpu, Zone::GtaoGbuffer);

        // Occlusion: a fullscreen pass reading the G-buffer (`t0`) → raw AO.
        self.zone_begin(gpu, Zone::Gtao);
        gpu.begin_color_pass(&self.gtao_raw);
        self.gtao_pipeline.bind(gpu);
        self.gtao_uniforms.bind_ps(gpu, GTAO_CBUFFER_SLOT);
        self.gtao_sampler.bind_ps(gpu, 0);
        self.gtao_gbuffer.bind_ps_srv(gpu, 0);
        gpu.draw(3);
        self.zone_end(gpu, Zone::Gtao);

        // Bilateral blur: reads the G-buffer (`t0`) + raw AO (`t1`) → blurred AO.
        // `begin_color_pass` rebinds the RTV to `gtao_blur`, releasing `gtao_raw`
        // as a render target before it's bound below as an SRV.
        self.zone_begin(gpu, Zone::GtaoBlur);
        gpu.begin_color_pass(&self.gtao_blur);
        self.gtao_blur_pipeline.bind(gpu);
        self.gtao_gbuffer.bind_ps_srv(gpu, 0);
        self.gtao_raw.bind_ps_srv(gpu, 1);
        gpu.draw(3);
        self.zone_end(gpu, Zone::GtaoBlur);
        // Drop the G-buffer / raw SRVs before the composite binds the scene
        // targets (and before next frame rebinds them as render targets).
        gpu.unbind_ps_srvs(2);
        Ok(())
    }

    /// Composite the offscreen HDR scene into the backbuffer: resolve the MSAA MRT
    /// into the single-sample textures the post pass samples (a no-op at 1×), then
    /// the fullscreen post pass (AO-darkened ambient + tone map + sRGB) over the
    /// viewport background. `ao` is the blurred GTAO target, or `None` to bind the
    /// ambient as a harmless placeholder (the shader ignores `t1` when
    /// `gtao_enabled` is 0). `self.sampler` (linear clamp) is bound at `s0`.
    fn record_composite(
        &self,
        gpu: &Gpu,
        post: &PostUniforms,
        ao: Option<&ColorTarget>,
        dest: BackbufferRect,
    ) -> GpuResult<()> {
        self.post_uniforms.update(gpu, post)?;
        gpu.begin_backbuffer_blit_rect(dest.x, dest.y, dest.width, dest.height);
        // The scene RTVs are unbound now (the backbuffer is the only bound
        // target), so the multisample resolve source is free.
        self.color.resolve(gpu);
        self.ambient.resolve(gpu);
        self.composite_pipeline.bind(gpu);
        self.post_uniforms.bind_ps(gpu, POST_CBUFFER_SLOT);
        self.color.bind_ps_srv(gpu, 0);
        ao.unwrap_or(&self.ambient).bind_ps_srv(gpu, 1);
        self.ambient.bind_ps_srv(gpu, 2);
        self.sampler.bind_ps(gpu, 0);
        gpu.draw(3);
        // Release the offscreen SRVs so next frame can bind them as render targets.
        gpu.unbind_ps_srvs(3);
        Ok(())
    }

    /// Render the 2D UV viewport (instead of the 3D scene): the 0..1 grid, the
    /// optional island fill (Shaded / Islands modes), then the model's UV edges on
    /// top — all framed by the 2D `uv_camera` and composited like the 3D scene
    /// (tone-mapped, no GTAO).
    // Independent per-frame inputs (gpu + model + revision + camera + channel +
    // shading mode + clear); none is redundant.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_uv(
        &mut self,
        gpu: &Gpu,
        model: &ModelData,
        model_revision: u64,
        uv_camera: UvCamera,
        channel: u32,
        shading_mode: UvShadingMode,
        anti_aliasing: AntiAliasing,
        background: ViewportBackground,
    ) -> GpuResult<()> {
        // The UV viewport shows the source model, so its derived buffers belong in
        // the source slot (see the note in `render`).
        self.activate(SlotId::Source);
        self.sync_targets(gpu, anti_aliasing.effective_sample_count(), gpu.size())?;
        self.sync_uv_view(gpu, model, model_revision, channel, shading_mode)?;

        // The UV camera's orthographic view-projection; the rest of the uniform is
        // unused by the UV path (lines/fill return their own vertex color).
        let uniforms = uv_scene_uniforms(uv_camera);
        self.uniforms.update(gpu, &uniforms)?;

        // Clear to zero (radiance + coverage); the background is painted in the
        // composite, matching the 3D path.
        gpu.begin_scene_pass(&[&self.color, &self.ambient], &self.depth, [0.0; 4]);
        // `fs_main` (the UV fill) samples the checker + every material slot at the top
        // (uniform control flow) before its zero-normal early-out, so all of group
        // 1/2/3 must be bound even though the UV draws never use the sampled values.
        self.bind_scene_shared(gpu, &self.checker_greyscale)?;

        // Reference grid first.
        self.scene.line.bind(gpu);
        self.uv_grid.bind(gpu);
        gpu.draw(self.uv_grid.count());

        // Island fill (Shaded / Islands), under the wireframe.
        if let Some(fill) = &self.active.views.uv_fill_buf {
            self.scene.uv_fill.bind(gpu);
            fill.bind(gpu);
            gpu.draw(fill.count());
        }

        // The model's UV edges on top.
        if let Some(wireframe) = &self.active.views.uv_wireframe_buf {
            self.scene.line.bind(gpu);
            wireframe.bind(gpu);
            gpu.draw(wireframe.count());
        }

        // Composite to the backbuffer: tone-mapped (the default operator, so shaded
        // fills read like the 3D scene), no GTAO (the flat UV viewport has no depth
        // to occlude), over the chosen viewport background.
        let post = post_uniforms(background, false, TonemapSettings::default(), false);
        self.record_composite(gpu, &post, None, BackbufferRect::full(gpu.size()))?;

        Ok(())
    }
}

/// Decode a baked UV-checker PNG into an sRGB GPU texture. A decode failure is a
/// packaging bug — fall back to a 1×1 white texel rather than failing the build of
/// the scene resources.
fn decode_checker(gpu: &Gpu, png_bytes: &[u8]) -> GpuResult<Texture> {
    match image::load_from_memory(png_bytes) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Texture::rgba8_single(gpu, width, height, &rgba, true)
        }
        Err(error) => {
            // Degrading to flat white is deliberate, but not silently: a corrupt
            // baked checker is a packaging bug worth seeing under `--tracy`.
            gpu_profiler::note(&format!("baked UV-checker PNG failed to decode: {error}"));
            Texture::rgba8_single(gpu, 1, 1, &[255, 255, 255, 255], true)
        }
    }
}

/// Build the per-frame [`SceneUniforms`] from the camera, projection, environment,
/// selection and debug options. The selection
/// flash rides in `selection_color` (gamma-space rgb + the flash fade in alpha,
/// zero while nothing is selected/flashing), read only by `fs_selection`.
/// Whether this frame is one of the flat data-inspection views, which emit final
/// display pixels from the scene shader.
///
/// They bypass lighting, the composite's tone map and GTAO entirely — a value
/// shown through a tone curve is no longer the value — so the composite blits
/// straight through and the occlusion passes are skipped.
fn flat_display(frame: &SceneFrame<'_>) -> bool {
    matches!(
        frame.debug.active_material,
        ActiveMaterial::Buffers | ActiveMaterial::SkinWeights
    )
}

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
/// orthographic view-projection matters (the grid / wireframe / fill return their
/// own vertex color, never reaching the IBL / shading / selection code).
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

/// Build the per-frame [`GtaoUniforms`] from the camera, projection and GTAO
/// settings. The settings' `radius` is a
/// fraction of the framed model's bounding-sphere radius, so it's scaled into view
/// units by the live `scene_radius` here, keeping the AO look scale-invariant.
fn build_gtao_uniforms(
    camera: OrbitCamera,
    projection: CameraProjection,
    gtao: GtaoSettings,
) -> GtaoUniforms {
    let scene_radius = camera.scene_radius.max(1e-3);
    let (slices, steps) = gtao.quality.slices_steps();
    GtaoUniforms {
        proj: camera.projection_matrix(projection).to_cols_array_2d(),
        params: [
            (gtao.radius * scene_radius).max(1e-4),
            gtao.intensity.max(0.0),
            gtao.thickness.clamp(0.0, 1.0),
            0.0,
        ],
        config: [
            if matches!(projection, CameraProjection::Orthographic) {
                1.0
            } else {
                0.0
            },
            slices.max(1) as f32,
            steps.max(1) as f32,
            0.0,
        ],
    }
}
