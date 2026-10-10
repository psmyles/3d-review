//! `review-render` — the viewer's camera math + GPU layer.
//!
//! The crate owns four things: the **cameras** ([`OrbitCamera`] for the 3D viewport,
//! [`UvCamera`] for the 2D UV viewport, and the 0.3s `CameraTransition` easing
//! between framings, all Reversed-Z), the per-view **config** types (re-exported from
//! `config`: shading / material / environment / GTAO / tonemap / AA options), the
//! [`Renderer`] — the host-facing handle that holds the live camera + editable
//! material table and draws each frame — and [`EguiRenderer`], which paints the
//! chrome into the same frame.
//!
//! Drawing goes through sokol_gfx, and every GPU call is confined to the `rhi`
//! module; the platform `unsafe` is confined further still, to the platform GPU leaf
//! `rhi/backend/` (plus sokol's one `extern "C"` logger callback in
//! `rhi/sokol_log.rs`), which is invariant 9's sanctioned GPU site. The camera and
//! material math above stays host-agnostic and safe, so `app` drives the renderer
//! purely through [`Renderer`]'s public API and never touches GPU state directly
//! (invariant 2). Shaders are one annotated-GLSL source (`src/shaders/review.glsl`)
//! generated to per-backend sources and compiled offline to committed bytecode by
//! `build.rs`.

// Invariant 9, enforced: `unsafe` is refused crate-wide, and each sanctioned
// FFI / GPU leaf opts in with a module-level `allow` that says why.
#![deny(unsafe_code)]

use glam::{Vec2, Vec3};
use review_model::{Bounds, DeformPose, ModelData};

mod camera;
mod config;
mod egui_sokol;
mod geometry;
mod ibl;
mod material;
mod renderer;
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
pub use geometry::{BONE_PICK_TOLERANCE_POINTS, pick_bone_shape, posed_joint_positions};
#[cfg(feature = "bake")]
pub use ibl::bake_ibl_assets;
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
/// renderer's own `scene::SceneVertex` additionally carries a 16-byte deform
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

/// Fraction of the safe area the empty "home" grid view fills. Below 1.0 so the
/// reference grid sits comfortably back in the viewport with margin around it,
/// rather than filling the window edge-to-edge. Only affects the home/reset
/// view — loaded models still frame tight to the safe area.
pub(crate) const HOME_FILL_FRACTION: f32 = 0.68;

/// Half-extent of the static reference grid: a 2 m square floor (±1 m) ruled in
/// 10 cm cells. Shared with `geometry::scene_lines` and the home-view framing so
/// the grid's size is defined in exactly one place. World units are meters.
pub(crate) const GRID_HALF_EXTENT: f32 = 1.0;
/// Axis-aligned bounds of that flat grid, used to frame the empty "home" view so
/// the whole floor is visible on launch and on reset.
pub(crate) const GRID_BOUNDS: Bounds = Bounds {
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
    /// the gizmo) all drive the main camera, and the second view
    /// follows it or is dragged by hand.
    pub opt_camera: OrbitCamera,
    camera_transition: Option<CameraTransition>,
    /// Fraction of the viewport (x = width, y = height) framing should fill,
    /// leaving room for the chrome that overlays the full-window 3D scene. Set
    /// by `app` from the live window + chrome sizes; `Vec2::ONE` = whole window.
    framing_safe_area: Vec2,
    /// Editable per-material PBR parameters, seeded from the loaded model's import
    /// defaults and edited live via [`MaterialEdit`] intents (invariant 2). Carried
    /// into the scene render each frame; the renderer-side table re-uploads them
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
    /// Every selected mesh/group node (sorted, deduplicated), of which
    /// `selection` names the primary. Empty falls back to that scalar, which is
    /// how a caller with no multi-selection of its own (the Opt workspace) keeps
    /// single-select behaviour.
    pub selected_nodes: &'a [u32],
    /// The node the pointer is over while the Select tool is active, tinted as a
    /// preview of what a click would pick. `None` outside Select mode, over empty
    /// space, and over a node that is already selected (which is already tinted).
    pub hover: Option<u32>,
    /// The bone the pointer is over, when the skeleton overlay has made bones the
    /// pick target instead of the mesh. Mutually exclusive with `hover`.
    pub hover_bone: Option<u32>,
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
    /// Physical pixels per egui point — the window's display scale (2 on a Retina
    /// screen, 1.25 at 125% on Windows). What turns
    /// [`SceneDebugOptions::wireframe_width`], which is in points, into pixels.
    pub pixels_per_point: f32,
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

/// Per-frame inputs for the 2D UV viewport, bundled for the reason
/// [`SceneFrame`] is.
pub struct UvFrame<'a> {
    pub model: &'a ModelData,
    pub model_revision: u64,
    /// The UV set to lay out.
    pub channel: u32,
    pub shading_mode: UvShadingMode,
    pub anti_aliasing: AntiAliasing,
    /// The fill around the layout: the Tex viewport's four backgrounds.
    pub background: TexBackground,
    /// The Outliner's node selection (sorted, deduplicated mesh/group nodes; each
    /// covers its subtree). Non-empty lays out only those nodes' UVs; empty lays
    /// out every node.
    pub selected_nodes: &'a [u32],
    /// Outliner-hidden mesh nodes (sorted), never laid out — hiding a part hides
    /// it in every workspace.
    pub hidden_meshes: &'a [u32],
    /// The texture picked in the Textures tab, drawn over the 0..1 square behind
    /// the layout so the islands can be read against the image they map.
    pub texture: Option<UvTexture<'a>>,
    /// Physical pixels per egui point, as [`SceneFrame::pixels_per_point`]: what
    /// gives the UV view's lines the same weight on every display.
    pub pixels_per_point: f32,
}

/// An image the UV viewport lays its islands over: the decoded pixels from the
/// app-owned pool, and the path that keys the upload (with the `Arc` as the
/// identity check, so a disk reload re-uploads).
#[derive(Clone, Copy)]
pub struct UvTexture<'a> {
    pub path: &'a std::path::Path,
    pub image: &'a std::sync::Arc<DecodedImage>,
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
        self.ensure_scene(frame, scene.anti_aliasing.effective_sample_count())?;
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

    /// Whether the ambient occlusion is still averaging frames and would visibly
    /// improve if given another one.
    ///
    /// GTAO's per-frame estimate is noisy at any affordable sample count, so whenever
    /// nothing that affects it is changing, each frame is folded into a running mean
    /// and the image converges (`scene/ao_accum.rs`). This is what lets `app` keep
    /// pacing frames for the ~0.4 s that takes — a genuine animation that ends on its
    /// own, which is the bar invariant 6 sets — and it must be read *after* the render
    /// that advanced it, or a single-event redraw would report the state from before
    /// its own reset and the average would stall at one sample.
    ///
    /// False whenever AO is off, the view is one of the flat data-inspection ones, or
    /// the result has already converged.
    pub fn is_ao_converging(&self) -> bool {
        self.scene.as_ref().is_some_and(SceneGpu::ao_converging)
    }

    /// Build the scene resources if this is the first frame that needs them, at the
    /// live MSAA level so the first frame needs no rebuild. A session that only ever
    /// looks at textures pays for none of it.
    fn ensure_scene(&mut self, frame: &Frame<'_>, sample_count: u32) -> GpuResult<()> {
        if self.scene.is_none() {
            self.scene = Some(SceneGpu::new(frame.size(), frame.clamp_msaa(sample_count))?);
        }
        Ok(())
    }

    /// Render the Opt workspace's comparison view: the source and processed meshes
    /// side by side, or one ghosted over the other. Shares every setting and every GPU
    /// resource with [`Self::render_scene`] — only the layout and the second model
    /// differ.
    pub fn render_opt_scene(
        &mut self,
        frame: &mut Frame<'_>,
        scene: &OptSceneFrame<'_>,
    ) -> GpuResult<()> {
        frame.set_clear(scene.base.background.gradient_srgb().0);
        let material_revision = self.material_revision;
        self.ensure_scene(frame, scene.base.anti_aliasing.effective_sample_count())?;
        let Some(scene_gpu) = self.scene.as_mut() else {
            return Ok(());
        };
        scene_gpu.render_opt(frame, scene, &self.material_states, material_revision)
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
    pub fn render_uv_scene(&mut self, frame: &mut Frame<'_>, uv: &UvFrame<'_>) -> GpuResult<()> {
        frame.set_clear(uv.background.clear_color());
        let uv_camera = self.uv_camera;
        self.ensure_scene(frame, uv.anti_aliasing.effective_sample_count())?;
        let Some(scene_gpu) = self.scene.as_mut() else {
            return Ok(());
        };
        scene_gpu.render_uv(frame, uv, uv_camera)
    }

    /// Render the 2D Tex viewport (instead of the 3D scene): the chosen background
    /// fill, then the selected image (channel-isolated, placed by `image`'s pixel
    /// rectangle) when one is present. The egui chrome is drawn on top afterwards.
    ///
    /// A solid background needs no draw of its own — it is the frame's clear colour
    /// (`docs/ARCHITECTURE.md`, Platform decisions: one swapchain pass); the checker
    /// and the image are queued as deferred draws, because the one swapchain pass has
    /// not opened yet.
    pub fn render_texture(
        &mut self,
        frame: &mut Frame<'_>,
        image: Option<TexImage>,
        background: TexBackground,
    ) -> GpuResult<()> {
        // Nothing the 3D, UV or Opt views derived is on screen here (invariant 3).
        // The meshes themselves stay uploaded, so returning costs no rebuild.
        if let Some(scene) = self.scene.as_mut() {
            scene.release_opt_views();
            scene.release_uv_views();
        }
        // Built on the first Tex frame rather than at startup: a session that never
        // opens this workspace pays for none of it.
        let tex = match self.tex.as_mut() {
            Some(tex) => tex,
            None => self.tex.insert(TexGpu::new()?),
        };
        tex.render(frame, image, background)
    }
}
