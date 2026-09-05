//! `review-render` — the viewer's camera math + GPU layer.
//!
//! The crate owns four things: the **cameras** ([`OrbitCamera`] for the 3D viewport,
//! [`UvCamera`] for the 2D UV viewport, and the 0.3s [`CameraTransition`] easing
//! between framings, all Reversed-Z), the per-view **config** types (re-exported from
//! [`config`]: shading / material / environment / GTAO / tonemap / AA options), the
//! [`Renderer`] — the host-facing handle that holds the live camera + editable
//! material table and draws each frame — and [`EguiRenderer`], which paints the
//! chrome into the same frame.
//!
//! Drawing goes through sokol_gfx, and every GPU call is confined to the [`rhi`]
//! module; the platform `unsafe` is confined further still, to `rhi/backend/`, which
//! is invariant 9's sanctioned GPU site. The camera and material math above stays
//! host-agnostic and safe, so `app` drives the renderer purely through [`Renderer`]'s
//! public API and never touches GPU state directly (invariant 2). Shaders are one
//! annotated-GLSL source (`src/shaders/review.glsl`) generated to per-backend sources
//! and compiled offline to committed bytecode by `build.rs`.
//!
//! ## Being ported (`mac-port-plan.md` Phase 1)
//!
//! [`Renderer::render_opt_scene`] — the Opt workspace's comparison view — has not
//! been moved onto sokol_gfx yet: it is `src/port_pending/scene_opt.rs` and is a stub
//! that draws nothing but the background. So are the scene's MSAA and its ambient
//! occlusion, which arrive with their own stages of step 4. Everything else is live:
//! the frame flow, the device and swapchain, the egui chrome, the Tex viewport and
//! the 3D + UV scenes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Vec2, Vec3};
use review_model::{Bounds, DeformPose, MaterialImportDefaults, ModelData};

mod camera;
mod config;
mod egui_sokol;
mod geometry;
mod ibl;
mod material;
mod rhi;
mod scene;
mod selection;
mod shaders;
mod tex;
mod texture;

use camera::CameraTransition;
pub use camera::{OrbitCamera, UvCamera, ease_in_out_cubic};
pub use config::*;
pub use egui_sokol::EguiRenderer;
pub use material::{
    AlphaMode, MaterialChange, MaterialEdit, MaterialSnapshot, MaterialState, RoughnessWorkflow,
    TextureBinding,
};
pub use rhi::gpu_profiler::enable_tracy_gpu;
pub use rhi::{Format, Frame, Gpu, GpuBringUp, GpuError, GpuResult, PresentStatus};
use scene::SceneGpu;
pub use selection::{Selection, SelectionView, selection_bounds};
use tex::TexGpu;
pub use tex::{TexBackground, TexImage};
pub use texture::{ChannelSelect, DecodedImage, TextureSlot, decode_image, suggested_channel};

/// Size in bytes of one *engine-equivalent* vertex: position, normal, UV,
/// tangent and color — the tuple a game engine's vertex buffer would hold for
/// this mesh.
///
/// Exposed for the Opt workspace's vertex-fetch analysis: meshoptimizer's
/// overfetch figure is "bytes fetched / vertex buffer size", so it describes the
/// asset's real cost only when given the size an engine would fetch. The
/// renderer's own [`scene::SceneVertex`] additionally carries a 16-byte deform
/// lane (skinning / blend-shape run references) that is a viewer-internal
/// mechanism, not part of the asset — so that lane is deliberately excluded and
/// this is pinned by the assertion below rather than measured from the struct.
/// The layout itself stays private — this is a measurement input, not an
/// invitation to build vertices elsewhere (invariant 1).
pub const fn scene_vertex_size() -> usize {
    ENGINE_VERTEX_SIZE
}

/// [`scene_vertex_size`]'s value: `SceneVertex` minus its deform lane.
const ENGINE_VERTEX_SIZE: usize = 64;
const _: () = assert!(
    ENGINE_VERTEX_SIZE + size_of::<[u32; 4]>() == size_of::<scene::SceneVertex>(),
    "SceneVertex changed size: re-derive the engine-equivalent vertex size"
);

/// Shorter transition used for the WASD 45° orbit steps, which fire repeatedly
/// and want a snappier response than the default framing/snap animation.
const ORBIT_TRANSITION_SECONDS: f32 = 0.1;
/// Fraction of the safe area the empty "home" grid view fills. Below 1.0 so the
/// reference grid sits comfortably back in the viewport with margin around it,
/// rather than filling the window edge-to-edge. Only affects the home/reset
/// view — loaded models still frame tight to the safe area.
const HOME_FILL_FRACTION: f32 = 0.68;

/// Half-extent of the static reference grid: a 2 m square floor (±1 m) ruled in
/// 10 cm cells. Shared with `geometry::scene_lines` and the home-view framing so
/// the grid's size is defined in exactly one place. World units are meters.
pub(crate) const GRID_HALF_EXTENT: f32 = 1.0;
/// Axis-aligned bounds of that flat grid, used to frame the empty "home" view so
/// the whole floor is visible on launch and on reset.
const GRID_BOUNDS: Bounds = Bounds {
    min: Vec3::new(-GRID_HALF_EXTENT, 0.0, -GRID_HALF_EXTENT),
    max: Vec3::new(GRID_HALF_EXTENT, 0.0, GRID_HALF_EXTENT),
};
/// Worst-case radius of the grid (its corner, ~1.41 m) with margin. The far
/// plane must reach it so the grid isn't clipped behind small models.
const GRID_FAR_RADIUS: f32 = GRID_HALF_EXTENT * 2.0;

/// The host-facing renderer: the live 3D/UV cameras, the editable per-material
/// table, and (built lazily on first draw) the scene + Tex GPU resources. `app` owns
/// one of these and drives every frame through its public API — applying
/// [`MaterialEdit`]/camera intents in, reading stats out (invariant 2). It carries no
/// window or swapchain; the frame in flight is passed per-call as a [`Frame`].
#[derive(Debug)]
pub struct Renderer {
    pub config: RendererConfig,
    pub camera: OrbitCamera,
    /// The 2D camera for the UV viewport, independent of the 3D orbit camera.
    pub uv_camera: UvCamera,
    /// The Opt workspace's *right-hand* camera, used only by the split view with
    /// camera sync off. With sync on it simply mirrors [`Self::camera`], which is
    /// why it needs no transition of its own: the animated moves (framing, home,
    /// the WASD steps, the gizmo) all drive the main camera, and the second view
    /// follows it or is dragged by hand.
    pub opt_camera: OrbitCamera,
    camera_transition: Option<CameraTransition>,
    /// Fraction of the viewport (x = width, y = height) framing should fill,
    /// leaving room for the chrome that overlays the full-window 3D scene. Set
    /// by `app` from the live window + chrome sizes; `Vec2::ONE` = whole window.
    framing_safe_area: Vec2,
    /// Editable per-material PBR parameters, seeded from the loaded model's import
    /// defaults and edited live via [`MaterialEdit`] intents (invariant 2). Carried
    /// into the scene callback each frame; the renderer-side table re-uploads them
    /// when `material_revision` changes.
    material_states: Vec<MaterialState>,
    /// Display names paired with `material_states`, for the app→UI snapshot.
    material_names: Vec<String>,
    /// Bumped on every material edit (and on model load) so the GPU table is
    /// re-uploaded without a full mesh rebuild.
    material_revision: u64,
    /// The Tex viewport's pipelines + texture cache, built on the first Tex frame.
    /// `None` in a session that never opens that workspace, which is most of them.
    tex: Option<TexGpu>,
    /// The scene's pipelines, offscreen targets, IBL maps and per-model caches, built
    /// on the first 3D or UV frame.
    scene: Option<SceneGpu>,
}

/// Per-frame inputs for the 3D scene render — everything `app` resolves from the
/// live UI state each frame, bundled in one struct so the render entry points
/// stay self-documenting and immune to argument-order mistakes among their many
/// same-typed inputs.
pub struct SceneFrame<'a> {
    pub model: &'a ModelData,
    pub model_revision: u64,
    pub debug: SceneDebugOptions,
    pub projection: CameraProjection,
    pub environment: EnvironmentSettings,
    pub gtao: GtaoSettings,
    pub tonemap: TonemapSettings,
    pub anti_aliasing: AntiAliasing,
    pub selection: SelectionView,
    pub hidden_meshes: &'a [u32],
    /// Bone nodes selected in the Outliner (sorted, deduplicated node indices).
    /// Drives the skeleton overlay's persistent highlight and the skin-weight heat
    /// map; empty when no bone is selected.
    pub selected_bones: &'a [u32],
    pub background: ViewportBackground,
    /// The pose to deform the model with — the rest pose or a clip's frame, as a
    /// ready palette + blend-shape weights built by `review_model::anim`. `None`
    /// draws the raw bind-pose buffers (a static model, or a workspace that
    /// deliberately shows the bind pose). Uploaded only when `pose_revision`
    /// changes.
    pub pose: Option<&'a DeformPose>,
    /// Identifies the pose above; the renderer re-uploads the palette only when it
    /// moves, so a paused clip costs nothing per frame.
    pub pose_revision: u64,
    /// What the bounding-box overlay (in its All Meshes scope) and framing
    /// describe this frame: the selected clip's motion envelope while a clip is
    /// selected, else the model's own rest bounds. `None` falls back to
    /// `model.bounds`.
    pub scene_bounds: Option<Bounds>,
}

impl<'a> SceneFrame<'a> {
    /// The same frame pointed at a different mesh — how the Opt workspace renders
    /// its processed model through every setting the source uses (shading,
    /// wireframe, normals, AA, AO, tone mapping), rather than a parallel path that
    /// would inevitably drift from it.
    pub fn with_model(&self, model: &'a ModelData, model_revision: u64) -> SceneFrame<'a> {
        SceneFrame {
            model,
            model_revision,
            ..*self
        }
    }
}

/// The processed mesh the Opt workspace compares against, and the revision its
/// GPU buffers are cached by.
#[derive(Debug, Clone, Copy)]
pub struct ProcessedModelRef<'a> {
    pub model: &'a ModelData,
    pub revision: u64,
}

/// How the Opt workspace lays its two meshes out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptView {
    /// Side by side: source on the left, processed on the right.
    Split,
    /// One view, both meshes in it — one shaded, the other a ghost over it.
    Overlay {
        ghost: GhostStyle,
        /// Show the *source* solid and the processed as the ghost (the A/B swap).
        swap: bool,
        /// The ghost's colour, gamma-space RGB. Supplied by the caller because
        /// the chrome shows the same colour in its legend and the two must
        /// match; the alpha is the renderer's, since it depends on the style.
        tint: [f32; 3],
    },
}

/// How the ghosted mesh is drawn in [`OptView::Overlay`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GhostStyle {
    /// A translucent tinted surface: reads the silhouette difference at a glance.
    #[default]
    Xray,
    /// Only the ghost's edges, leaving the solid surface fully visible.
    Wireframe,
}

/// The chrome-free area of the window, in physical pixels: the backbuffer minus
/// the toolbar, the status bar and whichever side panels are open.
///
/// Every other workspace renders across the whole backbuffer and lets the opaque
/// chrome cover what it must; the Opt split can't, because it has to *divide*
/// what the user can see. Splitting the backbuffer instead puts the divider
/// wherever the window's centre happens to fall — off-centre in the visible area
/// the moment a side panel is open, and with each half's content sitting at a
/// different offset from the divider, which reads as the two views disagreeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneViewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl SceneViewport {
    /// The whole backbuffer — the fallback when the chrome hasn't been laid out
    /// yet (the first frame) or covers nothing.
    pub fn full(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }
}

/// Per-frame inputs for the Opt workspace's comparison render.
pub struct OptSceneFrame<'a> {
    /// The source mesh and every shared setting; [`SceneFrame::with_model`]
    /// retargets it at the processed mesh.
    pub base: SceneFrame<'a>,
    /// `None` until a processing run has produced something. The split still
    /// draws two views — both of the source — so the layout the user chose is
    /// the layout they get, whether or not the stack has anything in it yet.
    pub processed: Option<ProcessedModelRef<'a>>,
    /// Where the split lays its two views out. Ignored by
    /// [`OptView::Overlay`], which draws one view across the whole backbuffer
    /// exactly as the 3D workspace does.
    pub viewport: SceneViewport,
    pub view: OptView,
    /// Camera for the source view. Also the camera for both meshes in
    /// [`OptView::Overlay`], where they share one space.
    pub source_camera: OrbitCamera,
    /// Camera for the processed view in [`OptView::Split`]. Equal to
    /// `source_camera` while the views are synced.
    pub processed_camera: OrbitCamera,
}

impl Renderer {
    /// Create a renderer from its config with default cameras and an empty material
    /// table. No GPU resources are built here — there is no device until the window is
    /// up, so the scene/Tex resources are created lazily on the first render.
    pub fn new(config: RendererConfig) -> Self {
        Self {
            config,
            camera: OrbitCamera::default(),
            uv_camera: UvCamera::default(),
            opt_camera: OrbitCamera::default(),
            camera_transition: None,
            framing_safe_area: Vec2::ONE,
            material_states: Vec::new(),
            material_names: Vec::new(),
            material_revision: 0,
            tex: None,
            scene: None,
        }
    }

    /// Render the scene: the skybox, the per-material PBR/IBL mesh, and the grid into
    /// offscreen linear-HDR targets, then a tone-mapped composite into the frame,
    /// behind the egui chrome `app` paints next.
    ///
    /// The composite paints the viewport background itself, wherever the scene left
    /// no coverage, so the frame's clear is only what a skipped composite would show.
    pub fn render_scene(&mut self, frame: &mut Frame<'_>, scene: &SceneFrame<'_>) -> GpuResult<()> {
        frame.set_clear(scene.background.gradient_srgb().0);
        let camera = self.camera;
        let material_revision = self.material_revision;
        self.ensure_scene(frame.size())?;
        // Borrowed as disjoint fields rather than through a `&mut self` helper: the
        // material table travels *into* the scene renderer, so one whole-`self` borrow
        // would exclude the other.
        let Some(scene_gpu) = self.scene.as_mut() else {
            return Ok(());
        };
        scene_gpu.render(
            frame,
            scene,
            &self.material_states,
            material_revision,
            camera,
        )
    }

    /// Build the scene resources if this is the first frame that needs them. A session
    /// that only ever looks at textures pays for none of it.
    fn ensure_scene(&mut self, size: (u32, u32)) -> GpuResult<()> {
        if self.scene.is_none() {
            self.scene = Some(SceneGpu::new(size)?);
        }
        Ok(())
    }

    /// Render the Opt workspace's comparison view: the source and processed meshes
    /// side by side, or one ghosted over the other. Shares every setting and every GPU
    /// resource with [`Self::render_scene`] — only the layout and the second model
    /// differ. **Stubbed**, as [`Self::render_scene`] is.
    pub fn render_opt_scene(
        &mut self,
        frame: &mut Frame<'_>,
        scene: &OptSceneFrame<'_>,
    ) -> GpuResult<()> {
        frame.set_clear(scene.base.background.gradient_srgb().0);
        Ok(())
    }

    /// Drop the processed mesh's GPU buffers (invariant 3). Called when the Opt
    /// workspace has nothing processed to show, so the memory isn't held while
    /// another workspace is up.
    pub fn release_processed_mesh(&mut self) {
        if let Some(scene) = self.scene.as_mut() {
            scene.release_processed();
        }
    }

    /// Drop the Tex viewport's uploaded textures (invariant 3). Called when the Tex
    /// workspace is left, so a session that visited it once doesn't hold its mipped
    /// uploads — a bounded cache, but a full one is hundreds of megabytes of VRAM —
    /// for the rest of the process. The viewport's pipelines are kept, so re-entering
    /// costs only the re-upload of whatever is looked at next.
    pub fn release_tex_cache(&mut self) {
        if let Some(tex) = self.tex.as_mut() {
            tex.release_cache();
        }
    }

    /// Render the 2D UV viewport (instead of the 3D scene): the 0..1 grid + the
    /// optional island fill + the model's UV edges, framed by the renderer's
    /// `uv_camera`.
    ///
    /// `anti_aliasing` is accepted but not yet applied: the scene renders
    /// single-sample until the AA stage of `mac-port-plan.md` Phase 1 step 4.
    #[allow(clippy::too_many_arguments)]
    pub fn render_uv_scene(
        &mut self,
        frame: &mut Frame<'_>,
        model: &ModelData,
        model_revision: u64,
        channel: u32,
        shading_mode: UvShadingMode,
        anti_aliasing: AntiAliasing,
        background: ViewportBackground,
    ) -> GpuResult<()> {
        let _ = anti_aliasing;
        frame.set_clear(background.gradient_srgb().0);
        let uv_camera = self.uv_camera;
        self.ensure_scene(frame.size())?;
        let Some(scene_gpu) = self.scene.as_mut() else {
            return Ok(());
        };
        scene_gpu.render_uv(
            frame,
            model,
            model_revision,
            uv_camera,
            channel,
            shading_mode,
            background,
        )
    }

    /// Render the 2D Tex viewport (instead of the 3D scene): the chosen background
    /// fill, then the selected image (channel-isolated, placed by `image`'s pixel
    /// rectangle) when one is present. The egui chrome is drawn on top afterwards.
    ///
    /// A solid background needs no draw of its own — it is the frame's clear colour
    /// (`mac-port-plan.md` §3.2); the checker and the image are queued as deferred
    /// draws, because the one swapchain pass has not opened yet.
    pub fn render_texture(
        &mut self,
        frame: &mut Frame<'_>,
        image: Option<TexImage>,
        background: TexBackground,
    ) -> GpuResult<()> {
        // Built on the first Tex frame rather than at startup: a session that never
        // opens this workspace pays for none of it.
        let tex = match self.tex.as_mut() {
            Some(tex) => tex,
            None => self.tex.insert(TexGpu::new()?),
        };
        tex.render(frame, image, background)
    }

    /// Seed the editable material table from a freshly loaded model's import
    /// defaults (or clear it for an empty model). Bumps the material revision so
    /// the GPU table is rebuilt/re-uploaded on the next frame.
    pub fn set_model_materials(&mut self, materials: &[MaterialImportDefaults]) {
        self.material_states = materials
            .iter()
            .map(|material| MaterialState {
                base_color: material.base_color,
                metallic: material.metallic,
                // Roughness is the complement of the imported glossiness.
                roughness: (1.0 - material.smoothness).clamp(0.0, 1.0),
                emissive: material.emissive,
                ..MaterialState::default()
            })
            .collect();
        self.material_names = materials
            .iter()
            .map(|material| material.name.clone())
            .collect();
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Apply one UI material-edit intent to the editable table, bumping the
    /// revision so the renderer re-uploads. Out-of-range indices are ignored.
    pub fn set_material_param(&mut self, edit: MaterialEdit) {
        let Some(state) = self.material_states.get_mut(edit.index) else {
            return;
        };
        match edit.change {
            MaterialChange::BaseColor(rgb) => state.base_color = Vec3::from_array(rgb),
            MaterialChange::Metallic(value) => state.metallic = value.clamp(0.0, 1.0),
            MaterialChange::Roughness(value) => state.roughness = value.clamp(0.0, 1.0),
            MaterialChange::Emissive(rgb) => state.emissive = Vec3::from_array(rgb),
            MaterialChange::Channel(slot, channel) => {
                // Re-route an already-assigned slot; ignored if the slot is empty
                // or out of range.
                if let Some(Some(binding)) = state.textures.get_mut(slot) {
                    binding.channel = channel;
                }
            }
            MaterialChange::AlphaMode(mode) => state.alpha_mode = mode,
            MaterialChange::AlphaCutoff(value) => state.alpha_cutoff = value.clamp(0.0, 1.0),
            MaterialChange::Workflow(workflow) => state.workflow = workflow,
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// The editable material parameters, carried into the scene callback each frame.
    pub fn material_states(&self) -> &[MaterialState] {
        &self.material_states
    }

    /// The current material revision (bumped on edit / load).
    pub fn material_revision(&self) -> u64 {
        self.material_revision
    }

    /// A name+value snapshot of the editable materials for the UI (invariant 2:
    /// the UI reads this plain value, never renderer-owned state).
    pub fn material_snapshot(&self) -> Vec<MaterialSnapshot> {
        self.material_names
            .iter()
            .cloned()
            .zip(self.material_states.iter().cloned())
            .map(|(name, state)| MaterialSnapshot { name, state })
            .collect()
    }

    /// Replace the entire editable material table with a captured set of states
    /// (the undo/redo restore path). Bumps the revision so the GPU table
    /// re-uploads on the next frame. The names are left untouched: the material
    /// count only changes on model load (which clears the undo history), so the
    /// restored states always line up with the current `material_names`.
    pub fn restore_materials(&mut self, states: Vec<MaterialState>) {
        self.material_states = states;
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Assign (or replace) a decoded image to one of a material's seven texture
    /// slots, with the chosen channel routing. The image is shared by `Arc` (the
    /// app decodes once and may reuse it across slots / materials). Bumps the
    /// revision so the GPU table uploads + rebinds on the next frame. Out-of-range
    /// material indices are ignored.
    pub fn set_texture_slot(
        &mut self,
        material: usize,
        slot: TextureSlot,
        path: PathBuf,
        image: Arc<DecodedImage>,
        channel: ChannelSelect,
    ) {
        let Some(state) = self.material_states.get_mut(material) else {
            return;
        };
        state.textures[slot.index()] = Some(TextureBinding {
            path,
            image,
            channel,
        });
        // Assigning an opacity map switches the material to alpha-blend so the
        // translucency shows; the Inspector can switch it to Clip.
        if slot == TextureSlot::Opacity && state.alpha_mode == AlphaMode::Opaque {
            state.alpha_mode = AlphaMode::Blend;
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Clear a material's texture slot back to the shader's neutral fallback.
    pub fn clear_texture_slot(&mut self, material: usize, slot: TextureSlot) {
        let Some(state) = self.material_states.get_mut(material) else {
            return;
        };
        state.textures[slot.index()] = None;
        // Clearing the opacity map restores opaque compositing.
        if slot == TextureSlot::Opacity {
            state.alpha_mode = AlphaMode::Opaque;
        }
        self.material_revision = self.material_revision.wrapping_add(1);
    }

    /// Replace the decoded image of every texture binding that references `path`
    /// (across all materials / slots) with `image` — the disk-auto-reload path.
    /// Keeps each binding's channel routing. Returns `true` (and bumps the
    /// revision) when at least one binding matched.
    pub fn reload_texture(&mut self, path: &Path, image: Arc<DecodedImage>) -> bool {
        let mut changed = false;
        for state in &mut self.material_states {
            for binding in state.textures.iter_mut().flatten() {
                if binding.path == path {
                    binding.image = Arc::clone(&image);
                    changed = true;
                }
            }
        }
        if changed {
            self.material_revision = self.material_revision.wrapping_add(1);
        }
        changed
    }

    /// Every distinct source path currently bound to a material slot (for the disk
    /// watcher to register / reconcile).
    pub fn texture_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        for state in &self.material_states {
            for binding in state.textures.iter().flatten() {
                if !paths.contains(&binding.path) {
                    paths.push(binding.path.clone());
                }
            }
        }
        paths
    }

    pub fn set_uv_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.uv_camera.aspect_ratio = aspect_ratio;
    }

    pub fn pan_uv_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.uv_camera.pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_uv_camera(&mut self, amount: f32) {
        self.uv_camera.zoom(amount);
    }

    pub fn reset_uv_camera(&mut self) {
        self.uv_camera.reset();
    }

    /// Set the fraction of the viewport that subsequent framing should fill, so
    /// the model lands inside the band left visible by the toolbar / status bar.
    pub fn set_framing_safe_area(&mut self, width_fraction: f32, height_fraction: f32) {
        self.framing_safe_area = Vec2::new(width_fraction, height_fraction);
    }

    pub fn orbit_camera(&mut self, delta: Vec2) {
        self.camera_transition = None;
        self.camera.orbit(delta);
    }

    pub fn set_camera_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.camera.aspect_ratio = aspect_ratio;
        self.opt_camera.aspect_ratio = aspect_ratio;
        if let Some(transition) = self.camera_transition.as_mut() {
            transition.start.aspect_ratio = aspect_ratio;
            transition.end.aspect_ratio = aspect_ratio;
        }
    }

    /// Orbit / pan / zoom the Opt split view's right-hand camera. Used only while
    /// camera sync is off; with it on, `app` drives the main camera and both views
    /// follow it.
    pub fn orbit_opt_camera(&mut self, delta: Vec2) {
        self.opt_camera.orbit(delta);
    }

    pub fn pan_opt_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.opt_camera
            .pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_opt_camera(&mut self, amount: f32) {
        self.opt_camera.zoom(amount);
    }

    /// Point the Opt split view's right-hand camera wherever the main one is
    /// looking — what "sync views" does, and what re-enabling it snaps back to.
    pub fn sync_opt_camera(&mut self) {
        self.opt_camera = self.camera;
    }

    pub fn pan_camera(&mut self, delta_pixels: Vec2, viewport_size: Vec2) {
        self.camera_transition = None;
        self.camera.pan_screen_delta(delta_pixels, viewport_size);
    }

    pub fn zoom_camera(&mut self, amount: f32) {
        self.camera_transition = None;
        self.camera.zoom(amount);
    }

    pub fn animate_camera_to(&mut self, end: OrbitCamera) {
        self.camera_transition = Some(CameraTransition::new(self.camera, end));
    }

    pub fn animate_camera_to_bounds(&mut self, bounds: Bounds) {
        self.animate_camera_to(self.camera.framed_to_bounds(bounds, self.framing_safe_area));
    }

    /// Snap (no animation) to a framing of `bounds`. Used when a model is
    /// loaded into the empty viewport, where a fly-in from the home view would
    /// only delay showing the model already framed.
    pub fn snap_camera_to_bounds(&mut self, bounds: Bounds) {
        self.camera_transition = None;
        self.camera = self.camera.framed_to_bounds(bounds, self.framing_safe_area);
    }

    pub fn animate_camera_to_offset_direction(&mut self, direction: Vec3) {
        self.animate_camera_to(self.camera.with_offset_direction(direction));
    }

    /// Animate a relative orbit by the given yaw / pitch deltas (radians). Based
    /// off any in-flight transition's target (not the mid-flight camera) so
    /// repeated key presses chain into successive 45° steps. Pitch is clamped to
    /// match interactive [`OrbitCamera::orbit`]. Uses the shorter
    /// [`ORBIT_TRANSITION_SECONDS`] so each step feels snappy.
    pub fn animate_orbit_by(&mut self, yaw_delta: f32, pitch_delta: f32) {
        let mut end = self.camera_transition.map_or(self.camera, |t| t.end);
        end.yaw += yaw_delta;
        end.pitch = (end.pitch + pitch_delta).clamp(-1.5, 1.5);
        self.camera_transition = Some(CameraTransition::with_duration(
            self.camera,
            end,
            ORBIT_TRANSITION_SECONDS,
        ));
    }

    /// The default "home" view, re-framed for the live aspect ratio and the
    /// chrome-aware safe area so the whole grid stays visible regardless of
    /// window shape. Shared by the animated reset and the instant startup frame
    /// so both land on exactly the same view.
    fn home_camera(&self) -> OrbitCamera {
        let home = OrbitCamera {
            aspect_ratio: self.camera.aspect_ratio,
            ..OrbitCamera::default()
        };
        // Fill only a fraction of the safe area so the grid sits back from the
        // edges (see HOME_FILL_FRACTION) instead of filling the window.
        home.framed_to_bounds(GRID_BOUNDS, self.framing_safe_area * HOME_FILL_FRACTION)
    }

    /// Animate back to the home view.
    pub fn animate_camera_to_home(&mut self) {
        self.animate_camera_to(self.home_camera());
    }

    /// Snap (no animation) to the home view. Used at startup once the real
    /// window size / safe area are known, so the initial frame matches the
    /// reset view rather than the full-window `OrbitCamera::default` framing.
    pub fn reset_camera_to_home(&mut self) {
        self.camera_transition = None;
        self.camera = self.home_camera();
    }

    pub fn update_camera_animation(&mut self, delta_seconds: f32) -> bool {
        let Some(transition) = self.camera_transition.as_mut() else {
            return false;
        };

        let (camera, finished) = transition.step(delta_seconds);
        self.camera = camera;
        if finished {
            self.camera_transition = None;
        }
        true
    }

    pub fn is_camera_animating(&self) -> bool {
        self.camera_transition.is_some()
    }
}
