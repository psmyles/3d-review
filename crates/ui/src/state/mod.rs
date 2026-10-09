//! UI state and the intents the UI emits back to `app`.
//!
//! Per invariant 2 this crate holds plain values + displayed stats and emits
//! [`UiOutput`] *intents*; it never owns or mutates renderer/model internals.
//! [`sync_debug_state`] funnels the committed panel values into the
//! [`SceneDebugOptions`] the renderer reads.
//!
//! ## Layout
//!
//! [`UiState`] itself is here, with the slider ranges and the debug-state funnel.
//! Its parts are grouped below, one module per concern: [`view`], [`panels`],
//! [`texture_view`], [`animation`], [`outliner`], [`panels_open`] and [`caches`]
//! — the same split `opt_state` already had — plus [`remembered`], which turns
//! the option-window values into the text **Remember settings** saves.

use std::collections::HashSet;
use std::path::PathBuf;

use review_model::{Bounds, MeshGroupStats, ModelData, ModelStats, StatsScope};
use review_render::{
    ActiveMaterial, AntiAliasing, BoundingBoxScope, EnvironmentSettings, GtaoSettings,
    MaterialEdit, MaterialSnapshot, SceneDebugOptions, Selection, ShadingMode, TonemapSettings,
    UvShadingMode, ViewportBackground, selection_bounds,
};

use crate::opt_state::{OptIntent, OptUiState};
use crate::theme;

mod animation;
mod caches;
mod comments;
mod outliner;
mod panels;
mod panels_open;
mod remembered;
mod selection;
mod texture_view;
mod view;

pub use animation::*;
pub use caches::*;
pub use comments::*;
pub use outliner::*;
pub use panels::*;
pub use panels_open::*;
pub(crate) use selection::SelectionKind;
pub use selection::{SelectMode, apply_pick};
pub use texture_view::*;
pub use view::*;

/// The inclusive ranges the panels' sliders enforce.
///
/// One home, one scheme: `<SUBJECT>_<PROPERTY>_{MIN,MAX}`, so a reader knows both
/// where an existing range lives and where the next one goes — panel-private
/// copies drifted apart and collided (two different `INTENSITY_MAX`es). A range
/// stays inline at its slider only when it is a *definition* rather than a choice:
/// a 0..1 unit factor like roughness or an alpha cutoff has no other range to
/// pick.
pub(crate) mod range {
    /// Checker repeats across the 0..1 UV range.
    pub const CHECKER_TILING_MIN: u32 = 1;
    pub const CHECKER_TILING_MAX: u32 = 16;
    /// Normal-line length: 0.1%–10% of the model's largest bounding extent.
    pub const NORMAL_LENGTH_MIN: f32 = 0.001;
    pub const NORMAL_LENGTH_MAX: f32 = 0.10;
    /// The skeleton overlay's size multiplier. A wide band because rig density
    /// varies enormously — a hand rig needs thinner bones than a vehicle's.
    pub const SKELETON_SCALE_MIN: f32 = 0.2;
    pub const SKELETON_SCALE_MAX: f32 = 4.0;

    /// Ambient occlusion. Radius is a *multiplier* over the radius the renderer
    /// derives for the current view (1 = automatic), so the band is centred on 1
    /// rather than running from nothing: the scene's scale is no longer this
    /// slider's job. Intensity is the power on the GTAO visibility; thickness is
    /// the see-through heuristic.
    pub const AO_RADIUS_MIN: f32 = 0.25;
    pub const AO_RADIUS_MAX: f32 = 4.0;
    pub const AO_INTENSITY_MIN: f32 = 0.0;
    pub const AO_INTENSITY_MAX: f32 = 2.0;
    pub const AO_THICKNESS_MIN: f32 = 0.0;
    pub const AO_THICKNESS_MAX: f32 = 1.0;

    /// Image-based lighting: the environment's intensity multiplier and its yaw
    /// (degrees; 0 = as-authored).
    pub const ENV_INTENSITY_MIN: f32 = 0.0;
    pub const ENV_INTENSITY_MAX: f32 = 3.0;
    pub const ENV_ROTATION_MIN: f32 = 0.0;
    pub const ENV_ROTATION_MAX: f32 = 360.0;

    /// The Opt operations' ranges live with the operations, so a preset loaded
    /// from disk is clamped to exactly what these sliders offer.
    pub use review_optimize::stack::limits::*;
}

/// How much of the window the docked side panels are covering, in egui points.
///
/// The floating chrome that centres itself — the notice column, the stats and
/// legend cards — has to centre on the area the user can actually see, not on
/// the window: with the Outliner open, the window's centre is well right of the
/// viewport's. Written by [`crate::overlay`] every frame (zero in the
/// workspaces that dock no panels) and read back by `app`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChromeInsets {
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiOutput {
    pub axis_gizmo_action: Option<AxisGizmoAction>,
    /// A live material-parameter edit emitted by the Inspector (base color /
    /// metallic / roughness / emissive / channel routing / alpha). `app` applies
    /// it to the renderer's editable material table.
    pub material_edit: Option<MaterialEdit>,
    /// A texture-pool command emitted by the Inspector this frame (import / assign
    /// / clear / remove), or `None`. One intent at a time — `app` applies it.
    pub texture: Option<TextureIntent>,
    /// Whether a material editor widget is being *actively dragged* this frame (a
    /// slider handle or a color-picker). `app` uses it to coalesce a continuous
    /// drag into a single undo step instead of one per intermediate value.
    pub material_edit_active: bool,
    /// An Opt action `app` must carry out this frame (export / preset IO). The
    /// stack itself is edited in place on [`UiState::opt`]; only the actions that
    /// reach outside the app travel as an intent.
    pub opt: Option<OptIntent>,
    /// Whether an Opt parameter widget is being actively dragged — the same
    /// drag-coalescing hint as [`UiOutput::material_edit_active`], so scrubbing a
    /// LOD ratio produces one undo step rather than one per frame.
    pub opt_edit_active: bool,
    /// A command chosen from the toolbar's menu this frame, for `app` to carry
    /// out — every entry reaches outside the chrome (a file dialog, the loaded
    /// model, the settings file, the process).
    pub menu: Option<MenuIntent>,
    /// A review-comment action that moves the camera or the clock, for `app` to
    /// carry out.
    pub comment: Option<CommentIntent>,
}

/// A review-comment action for `app`: the parts of "go to this comment" the
/// chrome doesn't own — the camera lives in the renderer, the clock in `app`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentIntent {
    /// Show thread `index` the way it was written: fly to its saved view and
    /// jump to its clip and first frame, where it has them.
    Show(usize),
}

/// An entry in the toolbar's menu. Each lands on the handler its keyboard
/// shortcut already reaches, where it has one, so the menu is a second door onto
/// an existing command rather than a second implementation of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuIntent {
    /// Ask for a model to open (the open shortcut).
    OpenFile,
    /// Open the model at this index of [`UiState::recent_files`] (File > Open
    /// Recent). An index rather than the path, so the intent stays `Copy`; `app`
    /// applies it in the same frame the list it indexes was drawn from.
    OpenRecent(usize),
    /// Empty [`UiState::recent_files`] and write the settings file.
    ClearRecentFiles,
    /// Write the review comments back into the opened FBX (the save shortcut).
    SaveComments,
    /// Write the opened FBX, with its review comments, to a new file.
    SaveCommentsAs,
    /// Drop the loaded model and return to the start state (the new shortcut).
    CloseFile,
    /// Flip [`UiState::remember_settings`] and write the settings file at once,
    /// so the choice survives even a session that never exits cleanly.
    ToggleRememberSettings,
    /// Flip [`UiState::tracy_profiler`] and write the settings file; it takes
    /// effect at the next launch.
    ToggleTracyProfiler,
    /// Ask GitHub for the newest release, and open the releases page if it is
    /// newer than this build (or say that this build is the latest).
    CheckForUpdates,
    /// Quit the application.
    Exit,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub mode: WorkspaceMode,
    pub debug: SceneDebugOptions,
    pub shading_mode: ShadingMode,
    pub projection_mode: ViewProjectionMode,
    pub show_grid: bool,
    pub show_axis_gizmo: bool,
    /// Whether the model-stats overlay is shown in the viewport. Toggled by the
    /// info button in the status bar.
    pub show_stats: bool,
    /// The set of tool option panels currently open. Each is its own native
    /// `egui::Window`; egui owns each window's position/size/collapsed state, so
    /// several can be open at once (invariant: UI holds only plain values). The
    /// field is public for struct construction; its mutators are crate-private so
    /// only the UI toggles panels.
    pub panels_open: PanelsOpen,
    /// The Opt workspace's operation stack and comparison-view settings. Present
    /// regardless of the active mode (it is document state, not view state), but
    /// only edited and read while Opt is active.
    pub opt: OptUiState,
    pub uv_checker: UvCheckerPanelState,
    /// Display labels of the loaded model's UV sets, in source-file order, shown
    /// in the UV-view toolbar dropdown. Empty when no model / no UV sets. Set by
    /// `app` from [`review_model::ModelData::uv_set_labels`] (invariant 2: plain
    /// values, not model ownership).
    pub uv_sets: Vec<String>,
    /// The UV set the 2D UV view draws (0-based index into [`UiState::uv_sets`]).
    /// Independent of the 3D UV-checker's own channel ([`UvCheckerPanelState`]).
    pub uv_view_channel: u32,
    /// How the 2D UV view shades the layout: wire-only, solid-shaded islands, or
    /// unique per-island colors. Selected by the UV-shading toolbar group (shown
    /// only in UV mode).
    pub uv_shading_mode: UvShadingMode,
    /// The Tex viewport's state: which pooled texture is shown plus its channel /
    /// background / pan-zoom view. Read by the texture-view chrome (toolbar channel
    /// group, status-bar background group) and the central image painter.
    pub texture_view: TextureViewState,
    /// The Tex viewport's central canvas rect (egui points), written by
    /// [`crate::texture_view`] each Tex frame and read by `app` to place the image in
    /// the Tex image draw. `None` until the Tex viewport has been
    /// laid out at least once.
    pub texture_canvas: Option<egui::Rect>,
    /// The chrome-free scene area (egui points): the window minus the toolbar,
    /// the status bar and whichever side panels are open. Written by
    /// [`crate::overlay`] each scene frame and read by `app`, which converts it
    /// to physical pixels for the Opt split — the divider has to land in the
    /// middle of what the user can see, not the middle of the window. `None`
    /// until the chrome has been laid out once, which the renderer reads as
    /// "the whole backbuffer".
    pub scene_viewport: Option<egui::Rect>,
    /// The open side panels' widths (egui points), written by [`crate::overlay`]
    /// each frame and read by `app` to centre the notice column on the free
    /// viewport rather than on the window. Not derivable from
    /// [`UiState::scene_viewport`], which is written only in the scene
    /// workspaces and left stale in UV / Tex — where the side panels don't draw
    /// and the insets are genuinely zero.
    pub chrome_insets: ChromeInsets,
    pub wireframe: WireframePanelState,
    pub bounding_box: BoundingBoxPanelState,
    pub face_normals: NormalPanelState,
    pub vertex_normals: NormalPanelState,
    pub uv_seams: UvSeamPanelState,
    pub skeleton: SkeletonPanelState,
    pub vertex_colors: VertexColorPanelState,
    /// Scene antialiasing (MSAA level). Read straight by the viewport callback —
    /// not a debug option — and edited by the Anti Aliasing panel.
    pub anti_aliasing: AntiAliasing,
    /// What this build and the active adapter can do — established once at
    /// startup and read-only afterwards (see [`Capabilities`]).
    pub capabilities: Capabilities,
    /// Image-based lighting / environment selection. Read straight by the
    /// viewport callback (not a debug option) and edited by the Environment
    /// panel. Default is IBL on, HDR 01, no background (see [`EnvironmentSettings`]).
    pub environment: EnvironmentSettings,
    /// Ambient occlusion (GTAO) settings. Read straight by the viewport
    /// callback and edited by the Ambient Occlusion panel; the AO status-bar
    /// button toggles `gtao.enabled`. Default is on (see [`GtaoSettings`]).
    pub gtao: GtaoSettings,
    /// Tone-mapping settings. Read straight by the viewport callback (not a debug
    /// option) and edited by the Tonemapper panel; the status-bar tonemapper button
    /// toggles `tonemap.enabled`. Default is on with Khronos PBR Neutral (see
    /// [`TonemapSettings`]).
    pub tonemap: TonemapSettings,
    /// Viewport background fill preset, read straight by the viewport callback. The
    /// status-bar Background button left-clicks to cycle the presets and right-clicks
    /// to open the Background options panel. Default is black; the IBL skybox
    /// (Environment → show background) overrides it when shown.
    pub viewport_background: ViewportBackground,
    pub stats: ModelStats,
    /// Name+value snapshot of the loaded model's editable materials, set by `app`
    /// from the renderer (invariant 2: a plain value, refreshed on load/edit).
    /// Drives the temporary Phase-1 material editor and feeds the per-material
    /// uniforms into the scene callback.
    pub materials_snapshot: Vec<MaterialSnapshot>,
    /// The scene-wide pool of imported textures (decoded images), set by `app`.
    /// The Inspector lists these in its Texture files section and offers them in
    /// each material property's texture dropdown; a texture is decoded once and
    /// shared by every material/slot that references it (invariant 2: plain
    /// snapshot value).
    pub texture_pool: Vec<TexturePoolEntry>,
    /// Material-table revision matching `materials_snapshot`, set by `app` from
    /// the renderer. Carried into the scene callback so the GPU table re-uploads
    /// only when an edit (or a new model) bumps it.
    pub material_revision: u64,
    /// What the left mouse button does in the viewport: turn the camera, or
    /// pick what it is over. View state — a tool, not an edit — so it is not
    /// undone, and a new model resets it.
    pub tool: ViewportTool,
    /// What the pointer is over in the viewport, or `None` outside Select mode
    /// and over empty space. Written by `app` from its pick, read by the
    /// renderer's hover highlight and by the pointer cursor.
    pub hover: Option<HoverTarget>,
    /// The Outliner selection (a node, a material, or nothing). Set by clicking a
    /// row in the Outliner or by picking in the viewport; drives the viewport
    /// highlight + solo and the Inspector.
    pub selection: Selection,
    /// Whether the selection is isolated (solo): only the selected geometry is
    /// drawn. A no-op while nothing is selected.
    pub solo: bool,
    /// The Outliner side panel's own state: tab, view mode, search, type filter,
    /// scene-tree cache and keyboard-navigation flags. Grouped by lifecycle —
    /// see [`OutlinerState`].
    pub outliner: OutlinerState,
    /// Animation clip selection + playback — see [`AnimationUiState`].
    pub animation: AnimationUiState,
    /// The loaded file's review comments, and how the chrome is showing them —
    /// see [`CommentsState`].
    pub comments: CommentsState,
    /// The in-app manual: whether its window is up, which page it is on, and
    /// where its images live. Chrome state like the panel set, edited in place
    /// rather than travelling as an intent — see [`crate::HelpState`].
    pub help: crate::HelpState,
    /// The About box — whether it is up, and the build / renderer facts `app`
    /// handed over for it to show. Opened from the menu's Help > About.
    pub about: crate::AboutState,
    /// The Log window — whether it is up, its level filter, and this session's
    /// lines as `app` last handed them over. Opened from the menu's Debug >
    /// View Log.
    pub log: crate::LogWindowState,
    /// Bone nodes selected in the Outliner, in click order (the last entry is the
    /// primary, mirrored into [`UiState::selection`]). Drives the skeleton
    /// overlay's highlight and the skin-weight heat map. Primary-click toggles a
    /// member, Shift-click takes a range; clicking any non-bone row clears it.
    ///
    /// Kept beside [`UiState::selection`] rather than inside it because
    /// [`Selection`] is `Copy` and threaded through renderer bake keys and undo
    /// snapshots, where a growable set would be the wrong shape.
    pub selected_bones: Vec<usize>,
    /// Mesh and group nodes selected together, in click order (the last entry is
    /// the primary, mirrored into [`UiState::selection`]). The twin of
    /// [`UiState::selected_bones`] for everything that is not a bone: the two are
    /// never both populated, since a click on either kind clears the other.
    ///
    /// Empty while nothing is selected *and* while a material is — a scalar
    /// [`Selection`] still says everything in that case, and every consumer falls
    /// back to it when this is empty.
    pub selected_nodes: Vec<usize>,
    /// Anchor row for Shift-click range selection: the last plainly-clicked or
    /// Primary-clicked row. `None` until a row is clicked. Shared by both sets,
    /// because only one of them is ever live.
    pub row_anchor: Option<usize>,
    /// Whether the loaded model carries any bone node. Gates the skeleton toolbar
    /// button (hidden entirely for an unrigged model). Set by `app` on load.
    pub has_bones: bool,
    /// Whether the loaded model carries skin weights. Gates the Skin Weights
    /// material-mode button. Set by `app` on load.
    pub has_skin: bool,
    /// Mesh nodes the user has hidden via the Outliner's per-row visibility
    /// checkbox (node indices into [`review_model::ModelData::nodes`]). The scene
    /// callback filters these meshes' triangles out of the viewport draw + GTAO.
    /// Cleared by `app` on model load (the indices no longer apply).
    pub hidden_meshes: HashSet<usize>,
    /// Whether the dockable side panels — the Outliner (left) and the Inspector
    /// (right) — are open. They share one flag because they are two halves of one
    /// workflow: the Outliner picks a row, the Inspector describes it. egui owns
    /// each panel's resized width; the UI only tracks open/closed.
    ///
    /// Starts `true`: inspecting a model is what the viewer is for, so the pair is
    /// up on launch rather than behind a toolbar toggle the user has to find.
    pub side_panels_open: bool,
    /// Axis-aligned bounds of the loaded model (world meters), set by `app`
    /// alongside [`UiState::stats`] (invariant 2: a plain value, not model
    /// ownership). `None` when no model is loaded. Read by the dimension-label
    /// overlay to place each box edge's axis-length readout.
    pub bounds: Option<Bounds>,
    /// Per clip, its motion envelope — parallel to `ModelData::animations`, and
    /// what [`UiState::bounds`] becomes while that clip is selected.
    ///
    /// It lives here rather than on the clip because it is *measured after the
    /// model is on screen*: it costs a pose and a mesh walk per frame of the clip,
    /// which would otherwise hold a large scene off the viewport for as long as
    /// the parse itself did. `app` fills each entry in when its import worker
    /// reports it; an entry still `None` simply falls back to the whole model's
    /// box, so a clip selected in the first moments of a load frames on something
    /// sensible and tightens up when the measurement lands.
    pub clip_bounds: Vec<Option<Bounds>>,
    /// The per-model derived measurements that are too costly to recompute every
    /// frame (the visible-only and selection-only boxes, the bone-influence
    /// count). `app` resets the whole set on model load — see [`BoundsCaches`].
    pub caches: BoundsCaches,
    /// Most recent measured frames-per-second, fed by `app` from the render
    /// loop. Zero while idle (the viewer redraws on demand, not continuously).
    pub fps: f32,
    /// Whether the option-window values are carried into the next session — the
    /// menu's **Remember settings** toggle. Read from the settings file at launch
    /// and flipped only by `app`, in answer to [`MenuIntent::ToggleRememberSettings`],
    /// since flipping it is also what writes the file. Off by default: nothing
    /// persists until the user asks for it.
    pub remember_settings: bool,
    /// Whether the Tracy client starts at launch — the menu's **Tracy Profiler**
    /// preference, a saved `--tracy`. It takes effect on the *next* launch only
    /// (a started client cannot be stopped), so it says nothing about whether
    /// Tracy is running now. Flipped only by `app`, which writes it to the
    /// settings file as it does.
    pub tracy_profiler: bool,
    /// The models opened lately, most recent first — File > Open Recent. Kept in
    /// the settings file whether or not **Remember settings** is on (it is a
    /// history, not a tool setting), and maintained only by `app`: a load that
    /// succeeds moves its file to the front, and one that fails because the file
    /// has gone drops it.
    pub recent_files: Vec<PathBuf>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            help: crate::HelpState::default(),
            about: crate::AboutState::default(),
            log: crate::LogWindowState::default(),
            mode: WorkspaceMode::ThreeD,
            debug: SceneDebugOptions::default(),
            shading_mode: ShadingMode::Shaded,
            projection_mode: ViewProjectionMode::Perspective,
            show_grid: true,
            show_axis_gizmo: true,
            show_stats: true,
            panels_open: PanelsOpen::default(),
            opt: OptUiState::default(),
            uv_checker: UvCheckerPanelState::default(),
            uv_sets: Vec::new(),
            uv_view_channel: 0,
            uv_shading_mode: UvShadingMode::default(),
            texture_view: TextureViewState::default(),
            texture_canvas: None,
            scene_viewport: None,
            chrome_insets: ChromeInsets::default(),
            wireframe: WireframePanelState::default(),
            bounding_box: BoundingBoxPanelState::default(),
            face_normals: NormalPanelState {
                length: DEFAULT_NORMAL_LENGTH,
                color: theme::color::FACE_NORMAL_DEFAULT,
            },
            vertex_normals: NormalPanelState {
                length: DEFAULT_NORMAL_LENGTH,
                color: theme::color::VERTEX_NORMAL_DEFAULT,
            },
            uv_seams: UvSeamPanelState::default(),
            skeleton: SkeletonPanelState::default(),
            vertex_colors: VertexColorPanelState::default(),
            anti_aliasing: AntiAliasing::default(),
            capabilities: Capabilities::default(),
            environment: EnvironmentSettings::default(),
            gtao: GtaoSettings::default(),
            tonemap: TonemapSettings::default(),
            viewport_background: ViewportBackground::default(),
            stats: ModelStats::default(),
            materials_snapshot: Vec::new(),
            texture_pool: Vec::new(),
            material_revision: 0,
            selection: Selection::None,
            solo: false,
            outliner: OutlinerState::default(),
            animation: AnimationUiState::default(),
            comments: CommentsState::default(),
            selected_bones: Vec::new(),
            tool: ViewportTool::default(),
            hover: None,
            selected_nodes: Vec::new(),
            row_anchor: None,
            has_bones: false,
            has_skin: false,
            hidden_meshes: HashSet::new(),
            side_panels_open: true,
            bounds: None,
            clip_bounds: Vec::new(),
            caches: BoundsCaches::default(),
            fps: 0.0,
            remember_settings: false,
            tracy_profiler: false,
            recent_files: Vec::new(),
        }
    }
}

impl UiState {
    /// The bounding box the dimension-label overlay measures this frame. In
    /// "visible only" mode this is the box over just the *unhidden* geometry — an
    /// O(triangle) scan ([`ModelData::visible_bounds`]), so the result is cached
    /// and rebuilt only when the hidden set changes (never per-frame). Otherwise,
    /// and whenever nothing is hidden, it's the whole-model [`UiState::bounds`]
    /// (no scan). Mirrors the box the renderer draws.
    pub(crate) fn measured_bounds(&mut self, model: &ModelData) -> Option<Bounds> {
        match self.bounding_box.scope {
            BoundsScope::AllMeshes => self.bounds,
            BoundsScope::OnlySelection => {
                // The selection scan ([`selection_bounds`]) is O(triangles), so
                // cache it and rebuild only when the selection changes — never
                // per-frame. (Model loads reset the cache, so it is never served
                // across a model swap — see [`BoundsCaches::reset`].)
                let key = (self.selection, self.selected_node_set());
                if self.caches.selection_bounds_key != key {
                    self.caches.selection_bounds = selection_bounds(model, key.0, &key.1);
                    self.caches.selection_bounds_key = key;
                }
                self.caches.selection_bounds
            }
            BoundsScope::VisibleOnly => {
                let mut hidden: Vec<u32> = self.hidden_meshes.iter().map(|&i| i as u32).collect();
                hidden.sort_unstable();
                // An empty hidden set makes `visible_bounds` the whole-model box, so
                // skip both the scan and the cache. (Model loads reset the cache
                // below, so it is never served across a model swap — see
                // [`BoundsCaches::reset`].)
                if hidden.is_empty() {
                    return self.bounds;
                }
                if self.caches.visible_bounds_key != hidden {
                    self.caches.visible_bounds = model.visible_bounds(&hidden);
                    self.caches.visible_bounds_key = hidden;
                }
                self.caches.visible_bounds
            }
        }
    }

    /// The box the bounding-box view draws for an Opt **processed** level, under the
    /// active scope — the right half of the split's dimension labels.
    ///
    /// The same three scopes as [`UiState::measured_bounds`], resolved against the
    /// processed mesh instead of the source: All Meshes is that level's own bounds
    /// (never [`UiState::bounds`], which measures the source and would report the
    /// LOD as unchanged), and the two scoped scans are the same O(triangle) walks,
    /// so they are cached against the level's revision as well as the scope's own
    /// inputs. A reprocess bumps the revision and re-measures.
    ///
    /// This mirrors what the renderer draws for its processed slot, which is the
    /// point: the label has to describe the box actually on screen beside it.
    pub(crate) fn processed_bounds(&mut self, model: &ModelData, revision: u64) -> Option<Bounds> {
        let scope = self.bounding_box.scope;
        // Only the inputs the chosen scope reads go in the key, so an unrelated
        // change can't force a re-measure (the same discipline the renderer's
        // bounding-box bake key follows).
        let selection = match scope {
            BoundsScope::OnlySelection => self.selection,
            _ => Selection::None,
        };
        let mut hidden: Vec<u32> = match scope {
            BoundsScope::VisibleOnly => self.hidden_meshes.iter().map(|&i| i as u32).collect(),
            _ => Vec::new(),
        };
        hidden.sort_unstable();

        let nodes = match scope {
            BoundsScope::OnlySelection => self.selected_node_set(),
            _ => Vec::new(),
        };
        let key = (revision, scope, selection, hidden, nodes);
        if self.caches.processed_bounds_key.as_ref() != Some(&key) {
            self.caches.processed_bounds = match scope {
                BoundsScope::AllMeshes => model.bounds,
                BoundsScope::OnlySelection => selection_bounds(model, selection, &key.4),
                // An empty hidden set makes `visible_bounds` the whole-model box.
                BoundsScope::VisibleOnly if key.3.is_empty() => model.bounds,
                BoundsScope::VisibleOnly => model.visible_bounds(&key.3),
            };
            self.caches.processed_bounds_key = Some(key);
        }
        self.caches.processed_bounds
    }

    /// The selection view the renderer reads each frame (invariant 2: a plain
    /// value): what is selected, whether it is isolated (solo), and the
    /// gamma-space highlight color sourced from the theme.
    pub fn selection_view(&self) -> review_render::SelectionView {
        review_render::SelectionView {
            selection: self.selection,
            solo: self.solo,
            // Alpha is the fill's opacity, held for as long as the selection
            // lasts.
            highlight_color: {
                let [r, g, b, _] = theme::color32_to_rgba(theme::color::SELECTION_OUTLINE);
                [r, g, b, theme::color::SELECTION_FILL_OPACITY]
            },
        }
    }

    /// The Outliner-hidden mesh nodes as a sorted `u32` list (the renderer's
    /// per-mesh visibility + the line-overlay hidden filter read this).
    pub fn hidden_mesh_nodes(&self) -> Vec<u32> {
        let mut hidden: Vec<u32> = self
            .hidden_meshes
            .iter()
            .map(|&index| index as u32)
            .collect();
        hidden.sort_unstable();
        hidden
    }

    /// The selected bone nodes as a sorted, deduplicated `u32` set — the shape the
    /// renderer wants (its bake keys compare it, and the skin lookup binary-searches
    /// it). Mirrors [`UiState::hidden_mesh_nodes`].
    pub fn selected_bone_nodes(&self) -> Vec<u32> {
        let mut bones: Vec<u32> = self
            .selected_bones
            .iter()
            .map(|&index| index as u32)
            .collect();
        bones.sort_unstable();
        bones.dedup();
        bones
    }

    /// The selected mesh/group nodes as a sorted, deduplicated `u32` set — the
    /// shape the renderer's bake keys compare and its subtree union walks.
    /// Mirrors [`UiState::selected_bone_nodes`].
    ///
    /// A lone [`Selection::Node`] with no set behind it (an Opt-workspace click,
    /// a restored undo snapshot) still reports that one node, so every caller can
    /// treat this as *the* selection rather than having to check both.
    pub fn selected_node_set(&self) -> Vec<u32> {
        if self.selected_nodes.is_empty() {
            return match self.selection {
                Selection::Node(node) => vec![node as u32],
                _ => Vec::new(),
            };
        }
        let mut nodes: Vec<u32> = self
            .selected_nodes
            .iter()
            .map(|&index| index as u32)
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        nodes
    }

    /// The nodes the UV workspace lays out, in the shape [`Self::selected_node_set`]
    /// gives: the mesh/group selection, or empty — meaning every node — when
    /// nothing is selected, or only a material or bones are (neither has a UV
    /// layout of its own to isolate).
    pub fn uv_scope_nodes(&self) -> Vec<u32> {
        if !self.selected_bones.is_empty() || !matches!(self.selection, Selection::Node(_)) {
            return Vec::new();
        }
        self.selected_node_set()
    }

    /// Drop every selection — both sets, the primary and the range anchor — and
    /// hand the Inspector back from a texture to the (now empty) selection.
    /// `Esc` and a click on empty viewport both land here, so neither can leave
    /// half a selection behind (a cleared primary with the skeleton still lit).
    pub fn clear_selection(&mut self) {
        self.texture_view.inspected = false;
        self.selection = Selection::None;
        self.selected_nodes.clear();
        self.selected_bones.clear();
        self.row_anchor = None;
    }

    /// Whether anything at all is selected — what `Esc` asks before it bothers
    /// clearing, so a stray set with no primary is still noticed.
    pub fn has_selection(&self) -> bool {
        self.selection.is_active()
            || !self.selected_nodes.is_empty()
            || !self.selected_bones.is_empty()
    }

    /// Re-point every piece of skeleton/skin UI state at a freshly loaded `model`:
    /// drop selections and caches keyed by the old model's node indices, and
    /// re-derive the capability flags that gate the skeleton toolbar button and the
    /// Skin Weights material mode.
    pub fn reset_skeletal_state(&mut self, model: &ModelData) {
        self.selected_bones.clear();
        self.selected_nodes.clear();
        self.row_anchor = None;
        self.outliner.collapsed.clear();
        self.outliner.hidden_kinds.clear();
        self.outliner.search.clear();
        self.outliner.nav_focus = false;
        self.outliner.scroll_to_selection = false;
        self.outliner.invalidate_tree();

        self.has_bones = model.stats.bone_count > 0;
        self.has_skin = model.skin.is_some();
        self.caches.bone_influence = 0;
        self.caches.bone_influence_key.clear();

        // Loading an unrigged mesh over a rigged one must not leave the viewer in
        // a mode whose toolbar button no longer exists.
        if !self.has_bones {
            self.debug.show_skeleton = false;
            self.panels_open.set(OptionPanel::Skeleton, false);
        }
        if !self.has_skin && self.debug.active_material == ActiveMaterial::SkinWeights {
            self.debug.active_material = ActiveMaterial::Source;
        }
    }

    /// Re-point the animation state at `model` (the one about to be shown):
    /// drop the clip selection and playback (they index the old model's clips)
    /// and re-derive the capability flag that gates the Animations tab. A stale
    /// Animations tab needs no snapping back here: the Outliner resolves its tab
    /// against what the model offers every frame ([`OutlinerState::tab`]).
    pub fn reset_animation_state(&mut self, model: &ModelData) {
        self.animation = AnimationUiState {
            has_clips: !model.animations.is_empty(),
            ..AnimationUiState::default()
        };
    }

    /// Select clip `clip` (or none), landing paused on its first frame; the app's
    /// clock picks the change up on the next frame.
    pub fn select_clip(&mut self, model: &ModelData, clip: Option<usize>) {
        let clip = clip.filter(|&index| index < model.animations.len());
        self.animation.selected_clip = clip;
        self.animation.playing = false;
        self.animation.time = clip.map_or(0.0, |index| model.animations[index].time_begin);
    }

    /// The stats overlay's three scoped columns, re-summed only when the
    /// selection or the hidden set has moved since they were last measured.
    ///
    /// The expensive half — the per-draw-group table the sums come from — is
    /// measured off the main thread and handed over by
    /// [`UiState::set_mesh_group_stats`]; both are dropped together by
    /// [`BoundsCaches::reset`] on load, since a new model can reproduce either
    /// key (the same node index selected again, the same one hidden again) while
    /// meaning something entirely different. Until the table arrives the columns
    /// sum an empty one and read as zero, which is what the card already renders
    /// as "not measured yet". Nothing here may build it: the walk is O(corners)
    /// with a hash per vertex (invariant 6).
    pub(crate) fn scoped_stats(&mut self, model: &ModelData) -> ScopedStats {
        let Some(groups) = self.caches.mesh_groups.as_deref() else {
            return ScopedStats::default();
        };

        let key = (
            self.selection,
            self.hidden_mesh_nodes(),
            self.selected_node_set(),
        );
        if self.caches.scoped_stats.is_none() || self.caches.scoped_stats_key != key {
            // The selection covers each selected node's whole subtree, matching
            // the geometry the viewport highlights and the "only selection" box
            // wraps.
            let selected = match key.0 {
                Selection::None => None,
                Selection::Material(slot) => {
                    Some(model.scope_stats(groups, StatsScope::Material(slot as u32)))
                }
                Selection::Node(node) => {
                    let mut mask = vec![false; model.nodes.len()];
                    for &selected in &key.2 {
                        for (slot, covered) in model
                            .node_subtree_mask(selected as usize)
                            .iter()
                            .enumerate()
                        {
                            if *covered && let Some(entry) = mask.get_mut(slot) {
                                *entry = true;
                            }
                        }
                    }
                    if key.2.is_empty() {
                        mask = model.node_subtree_mask(node);
                    }
                    Some(model.scope_stats(groups, StatsScope::Nodes(&mask)))
                }
            };
            let visible: Vec<bool> = (0..model.nodes.len())
                .map(|node| !self.hidden_meshes.contains(&node))
                .collect();
            self.caches.scoped_stats = Some(ScopedStats {
                selected,
                visible: model.scope_stats(groups, StatsScope::Nodes(&visible)),
            });
            self.caches.scoped_stats_key = key;
        }
        self.caches.scoped_stats.unwrap_or_default()
    }

    /// Take the model's measured per-draw-group table — the walk behind both the
    /// `GPU Verts` row and the stats card's scoped columns — from `app`, which
    /// measures it on the import worker once the model is already drawn.
    ///
    /// Both consumers are updated together on purpose: [`UiState::scoped_stats`]
    /// would otherwise build the same table itself, on the main thread, the first
    /// frame the card asked for it — an O(corners) hash walk that is a 2.6 s
    /// stall on a 2.8M-triangle scene (invariant 6: nothing that size runs on the
    /// redraw path).
    pub fn set_mesh_group_stats(&mut self, groups: Vec<MeshGroupStats>) {
        self.stats.gpu_vertex_count = groups.iter().map(|group| group.gpu_vertex_count).sum();
        self.caches.mesh_groups = Some(groups);
        // The columns were summed from no table (or the outgoing model's); re-sum.
        self.caches.scoped_stats = None;
    }

    /// Refresh [`BoundsCaches::bone_influence`] if the bone selection changed
    /// since it was last measured. Called once per frame before the panels draw,
    /// so the Inspector can read a real measured number (invariant 5) without
    /// re-scanning the skin table on every repaint.
    ///
    /// Counts the *union* of the selected bones' influenced vertices, so
    /// overlapping regions aren't double-counted — the honest answer to "how much
    /// of the mesh does this selection move".
    pub(crate) fn sync_bone_influence(&mut self, model: &ModelData) {
        let key = self.selected_bone_nodes();
        if key == self.caches.bone_influence_key {
            return;
        }
        self.caches.bone_influence = match model.skin.as_ref() {
            Some(skin) if !key.is_empty() => (0..skin.logical_vertex_count())
                .filter(|&logical| {
                    skin.bones[skin.influence_range(logical)]
                        .iter()
                        .any(|bone| key.binary_search(bone).is_ok())
                })
                .count(),
            _ => 0,
        };
        self.caches.bone_influence_key = key;
    }
}

/// Copy the committed panel values into the [`SceneDebugOptions`] the renderer
/// reads. Called once per frame before the overlay is drawn.
pub(crate) fn sync_debug_state(state: &mut UiState) {
    state.debug.shading_mode = state.shading_mode;
    state.debug.show_grid = state.show_grid;
    state.debug.uv_checker_texture = state.uv_checker.texture;
    state.debug.uv_checker_tiling = state.uv_checker.tiling;
    state.debug.uv_channel = state.uv_checker.uv_channel;
    state.debug.vertex_color_mode = state.vertex_colors.mode;
    state.debug.face_normal_length = state.face_normals.length;
    state.debug.vertex_normal_length = state.vertex_normals.length;
    state.debug.face_normal_color = theme::color32_to_rgba(state.face_normals.color);
    state.debug.vertex_normal_color = theme::color32_to_rgba(state.vertex_normals.color);
    state.debug.uv_seam_color = theme::color32_to_rgba(state.uv_seams.color);
    // A reload can shrink the UV-set list under a channel the panel still points
    // at; the renderer falls back to `Vertex::uv` for an out-of-range channel, so
    // clamping here is what keeps the panel's readout honest about that.
    if !state.uv_sets.is_empty() && state.uv_seams.uv_channel >= state.uv_sets.len() as u32 {
        state.uv_seams.uv_channel = 0;
    }
    state.debug.uv_seam_channel = state.uv_seams.uv_channel;
    state.debug.wireframe_color = theme::color32_to_rgba(state.wireframe.color);
    state.debug.bounding_box_color = theme::color32_to_rgba(state.bounding_box.color);
    state.debug.bounding_box_scope = match state.bounding_box.scope {
        BoundsScope::AllMeshes => BoundingBoxScope::AllMeshes,
        BoundsScope::OnlySelection => BoundingBoxScope::OnlySelection,
        BoundsScope::VisibleOnly => BoundingBoxScope::VisibleOnly,
    };
    state.debug.bounding_box_selection = state.selection;
    state.debug.hover_color = {
        let [r, g, b, _] = theme::color32_to_rgba(theme::color::HOVER_HIGHLIGHT);
        [r, g, b, theme::color::HOVER_FILL_OPACITY]
    };
    // The skeleton bakes its tints into vertex colours, so the hover one goes
    // over opaque — the overlay's own fill alpha is applied on top of it.
    state.debug.skeleton_hover_color = {
        let [r, g, b, _] = theme::color32_to_rgba(theme::color::HOVER_HIGHLIGHT);
        [r, g, b, 1.0]
    };
    state.debug.skeleton_joint_scale = state.skeleton.scale;
    state.debug.skeleton_color = theme::color32_to_rgba(state.skeleton.color);
    // The selected-bone tint reuses the viewport's selection color, so a bone
    // highlights the same hue as a selected mesh part.
    state.debug.skeleton_selected_color = theme::color32_to_rgba(theme::color::SELECTION_OUTLINE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_view_shader_index_and_offset_agree() {
        // RGB is the combined view: shader index 0, no single-channel byte offset.
        assert_eq!(TextureChannelView::Rgb.shader_index(), 0);
        assert_eq!(TextureChannelView::Rgb.channel_offset(), None);
        // Each single channel's byte offset is one less than its shader index.
        for channel in [
            TextureChannelView::R,
            TextureChannelView::G,
            TextureChannelView::B,
            TextureChannelView::A,
        ] {
            let offset = channel
                .channel_offset()
                .expect("a single channel has a byte offset");
            assert_eq!(channel.shader_index() as usize, offset + 1);
        }
    }

    #[test]
    fn channel_view_all_is_distinct_and_display_ordered() {
        let labels: Vec<String> = TextureChannelView::ALL
            .into_iter()
            .map(|channel| review_localization::tr(channel.label()).into_owned())
            .collect();
        assert_eq!(labels, ["RGB", "R", "G", "B", "A"]);
    }

    #[test]
    fn panels_open_toggle_flips_state() {
        let mut panels = PanelsOpen::default();
        assert!(!panels.is_open(OptionPanel::Wireframe));
        panels.toggle(OptionPanel::Wireframe);
        assert!(panels.is_open(OptionPanel::Wireframe));
        panels.toggle(OptionPanel::Wireframe);
        assert!(!panels.is_open(OptionPanel::Wireframe));
    }

    #[test]
    fn panels_open_set_is_idempotent_and_independent() {
        let mut panels = PanelsOpen::default();
        panels.set(OptionPanel::Gtao, true);
        panels.set(OptionPanel::Gtao, true);
        assert!(panels.is_open(OptionPanel::Gtao));
        // Toggling a different panel doesn't disturb this one.
        panels.toggle(OptionPanel::Tonemap);
        assert!(panels.is_open(OptionPanel::Gtao));
        panels.set(OptionPanel::Gtao, false);
        assert!(!panels.is_open(OptionPanel::Gtao));
    }
}
