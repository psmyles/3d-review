//! UI state and the intents the UI emits back to `app`.
//!
//! Per invariant 2 this crate holds plain values + displayed stats and emits
//! [`UiOutput`] *intents*; it never owns or mutates renderer/model internals.
//! [`sync_debug_state`] funnels the committed panel values into the
//! [`SceneDebugOptions`] the renderer reads.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use glam::{Vec2, Vec3};
use review_model::{Bounds, ModelData, ModelStats, NodeKind};
use review_render::{
    ActiveMaterial, AntiAliasing, BoundingBoxScope, CameraProjection, CheckerTexture, DecodedImage,
    EnvironmentSettings, GtaoSettings, MaterialEdit, MaterialSnapshot, MsaaSamples,
    SceneDebugOptions, Selection, ShadingMode, TonemapSettings, UvShadingMode, VertexColorMode,
    ViewportBackground, selection_bounds,
};

use crate::opt_state::{OptIntent, OptUiState};
use crate::theme;

/// Default checker repeats across the 0..1 UV range for a fresh / reset panel.
pub(crate) const DEFAULT_CHECKER_TILING: u32 = 4;
/// Default length for the face/vertex normal-line views, as a fraction of the
/// model's largest bounding extent (see `debug_normal_length` in `review-render`).
pub(crate) const DEFAULT_NORMAL_LENGTH: f32 = 0.03;
/// Default multiplier for the skeleton overlay's bone size.
pub(crate) const DEFAULT_SKELETON_SCALE: f32 = 1.0;

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

    /// Ambient occlusion. Radius is a fraction of the framed model's bounding
    /// sphere; intensity is the power on the GTAO visibility; thickness is the
    /// see-through heuristic.
    pub const AO_RADIUS_MIN: f32 = 0.02;
    pub const AO_RADIUS_MAX: f32 = 1.0;
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

    /// Opt operations. The weld tolerance and the prune size threshold are both
    /// fractions of the mesh's overall size; the attribute weights scale a
    /// simplifier's per-attribute error; the overdraw threshold is the vertex-cache
    /// efficiency it may give up (1.0 = none); the LOD rows are the per-level
    /// triangle target and error limit.
    pub const WELD_TOLERANCE_MIN: f32 = 0.0;
    pub const WELD_TOLERANCE_MAX: f32 = 0.1;
    pub const PRUNE_THRESHOLD_MIN: f32 = 0.0;
    pub const PRUNE_THRESHOLD_MAX: f32 = 0.5;
    pub const ATTRIBUTE_WEIGHT_MIN: f32 = 0.0;
    pub const ATTRIBUTE_WEIGHT_MAX: f32 = 4.0;
    pub const OVERDRAW_THRESHOLD_MIN: f32 = 1.0;
    pub const OVERDRAW_THRESHOLD_MAX: f32 = 3.0;
    pub const LOD_RATIO_MIN: f32 = 0.01;
    pub const LOD_RATIO_MAX: f32 = 1.0;
    pub const LOD_ERROR_MIN: f32 = 0.0;
    pub const LOD_ERROR_MAX: f32 = 1.0;
    /// Bake AO: the max ray distance in world meters (0 = unlimited) and the
    /// power on visibility (matching the viewport AO panel's Intensity, whose
    /// range is deliberately wider here — a bake is worth over-driving).
    pub const AO_BAKE_DISTANCE_MIN: f32 = 0.0;
    pub const AO_BAKE_DISTANCE_MAX: f32 = 10.0;
    pub const AO_BAKE_INTENSITY_MIN: f32 = 0.1;
    pub const AO_BAKE_INTENSITY_MAX: f32 = 4.0;
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    ThreeD,
    Uv,
    Texture,
    /// Mesh optimization: the same 3D scene chrome, plus the operation stack and
    /// a source-vs-processed comparison viewport.
    Opt,
}

impl WorkspaceMode {
    /// True for the workspaces that draw the 3D scene and therefore share its
    /// chrome — side panels, option windows, the axis gizmo, the stats overlay,
    /// and every shading / diagnostic control. Opt is a 3D workspace with extra
    /// tooling, not a separate kind of viewport.
    pub fn is_scene(self) -> bool {
        matches!(self, WorkspaceMode::ThreeD | WorkspaceMode::Opt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewProjectionMode {
    Perspective,
    Orthographic,
}

impl From<ViewProjectionMode> for CameraProjection {
    fn from(value: ViewProjectionMode) -> Self {
        match value {
            ViewProjectionMode::Perspective => Self::Perspective,
            ViewProjectionMode::Orthographic => Self::Orthographic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewAxis {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl ViewAxis {
    pub fn offset_direction(self) -> Vec3 {
        match self {
            Self::PositiveX => Vec3::X,
            Self::NegativeX => Vec3::NEG_X,
            Self::PositiveY => Vec3::Y,
            Self::NegativeY => Vec3::NEG_Y,
            Self::PositiveZ => Vec3::Z,
            Self::NegativeZ => Vec3::NEG_Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisGizmoAction {
    Orbit(Vec2),
    Snap(ViewAxis),
    /// Return the camera to its starting "home" view (the gizmo's reset button).
    ResetView,
}

/// A reference to one material's texture slot (the slot is a
/// [`review_render::TextureSlot`] index, `0..7`), used by the Inspector's
/// assign / clear intents. `app` owns the filesystem + decoded-texture pool; the
/// UI only points at the slot (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureSlotRef {
    pub material: usize,
    pub slot: usize,
}

/// One imported texture in the scene-wide pool, surfaced to the Inspector so it
/// can list the files (with a real thumbnail built from `image`) and offer them
/// in each material property's texture dropdown — and to the Tex viewport for the
/// full-image view + its stats panel. A plain app→UI snapshot value (invariant 2):
/// `app` owns the decode + the pool, the UI only reads this. The `Arc` makes
/// carrying it a refcount bump, not a pixel copy.
#[derive(Debug, Clone)]
pub struct TexturePoolEntry {
    pub path: PathBuf,
    pub image: Arc<DecodedImage>,
    /// Size of the source file on disk in bytes, measured by `app` when it builds
    /// the pool (0 if the file could not be stat'd). Shown in the Tex viewport's
    /// stats panel.
    pub file_size: u64,
}

impl TexturePoolEntry {
    /// Display name of this texture (its file name).
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<texture>")
            .to_owned()
    }

    /// Uppercased source-file format label from the extension (e.g. `PNG`, `TGA`),
    /// or `—` when the path carries none. Shown in the Tex viewport's stats panel.
    pub fn format_label(&self) -> String {
        self.path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_uppercase())
            .unwrap_or_else(|| "-".to_owned())
    }
}

/// Which channel(s) of the viewed texture the Tex viewport displays. `Rgb` shows
/// the full color (transparency composites over the background fill); a single
/// channel shows that channel replicated as opaque greyscale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextureChannelView {
    #[default]
    Rgb,
    R,
    G,
    B,
    A,
}

impl TextureChannelView {
    /// In display order (the toolbar radio group), RGB first.
    pub const ALL: [TextureChannelView; 5] = [
        TextureChannelView::Rgb,
        TextureChannelView::R,
        TextureChannelView::G,
        TextureChannelView::B,
        TextureChannelView::A,
    ];

    /// Toolbar segment label.
    pub fn label(self) -> &'static str {
        match self {
            TextureChannelView::Rgb => "RGB",
            TextureChannelView::R => "R",
            TextureChannelView::G => "G",
            TextureChannelView::B => "B",
            TextureChannelView::A => "A",
        }
    }

    /// Byte offset of a single channel within an RGBA8 pixel, or `None` for the
    /// full-RGB view.
    pub fn channel_offset(self) -> Option<usize> {
        match self {
            TextureChannelView::Rgb => None,
            TextureChannelView::R => Some(0),
            TextureChannelView::G => Some(1),
            TextureChannelView::B => Some(2),
            TextureChannelView::A => Some(3),
        }
    }

    /// The channel index the Tex viewport shader reads (`0` RGB, `1..4` R/G/B/A).
    /// Must match `tex.hlsl`'s `channel` switch.
    pub fn shader_index(self) -> u32 {
        match self {
            TextureChannelView::Rgb => 0,
            TextureChannelView::R => 1,
            TextureChannelView::G => 2,
            TextureChannelView::B => 3,
            TextureChannelView::A => 4,
        }
    }
}

/// The background fill drawn behind the viewed texture in the Tex viewport, so an
/// image's transparency reads against a known backdrop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextureBackground {
    #[default]
    Black,
    White,
    Grey,
    Checker,
}

impl TextureBackground {
    /// In display order (the status-bar radio group).
    pub const ALL: [TextureBackground; 4] = [
        TextureBackground::Black,
        TextureBackground::White,
        TextureBackground::Grey,
        TextureBackground::Checker,
    ];

    /// Single-letter status-bar segment label (Black / White / Grey / Checker).
    pub fn label(self) -> &'static str {
        match self {
            TextureBackground::Black => "B",
            TextureBackground::White => "W",
            TextureBackground::Grey => "G",
            TextureBackground::Checker => "C",
        }
    }
}

/// State backing the Tex viewport: which pooled texture is shown, the channel
/// isolation + background fill, the floating stats toggle, and the pan/zoom view.
/// All plain UI values (invariant 2) — the pixels live in [`UiState::texture_pool`].
#[derive(Debug, Clone)]
pub struct TextureViewState {
    /// Index into [`UiState::texture_pool`] of the viewed texture. Clamped to the
    /// pool each frame; ignored when the pool is empty.
    pub selected: usize,
    pub channel: TextureChannelView,
    pub background: TextureBackground,
    /// Whether the texture stats panel (Format / Dimension / Channels / Bit depth /
    /// File size) is shown — the Tex viewport's analogue of the model-stats overlay.
    pub show_stats: bool,
    /// Screen-point offset of the image center from the viewport center (pan).
    pub pan: egui::Vec2,
    /// Image-pixels → screen-points scale (zoom). 1.0 = one texel per point.
    pub zoom: f32,
    /// Identity (decoded-image `Arc` pointer) the current pan/zoom was fit for; a
    /// mismatch re-fits the image to the viewport (on first show / texture switch /
    /// disk reload). `None` forces a fit on the next frame.
    pub fitted_key: Option<usize>,
    /// A requested animated view change (the zoom-readout toggle / `F` frame
    /// reset), resolved to a concrete pan/zoom target + eased on the next paint.
    /// `None` when nothing is pending.
    pub request: Option<TexViewRequest>,
    /// An in-flight ease of the pan/zoom toward a target. `None` when settled.
    pub transition: Option<TexViewTransition>,
}

/// A requested animated change to the Tex view, set by the zoom-readout toggle
/// and the `F` / `R` frame reset and consumed by `texture_view` on the next paint
/// (where the viewport rect — needed to compute a fit — is known). Resolving it
/// starts a [`TexViewTransition`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TexViewRequest {
    /// Ease to an absolute zoom, keeping the current pan (the 100% reset).
    Zoom(f32),
    /// Ease to the fitted view (computed from the current viewport).
    Fit,
}

/// An in-flight ease of the Tex view's pan/zoom toward a target, run over
/// [`crate::theme::motion::TEXTURE_ZOOM_ANIM_SECS`]. `start_time` is egui's input
/// time (monotonic seconds) at the ease's start.
#[derive(Debug, Clone, Copy)]
pub struct TexViewTransition {
    pub from_zoom: f32,
    pub to_zoom: f32,
    pub from_pan: egui::Vec2,
    pub to_pan: egui::Vec2,
    pub start_time: f64,
}

impl Default for TextureViewState {
    fn default() -> Self {
        Self {
            selected: 0,
            channel: TextureChannelView::default(),
            background: TextureBackground::default(),
            show_stats: true,
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            fitted_key: None,
            request: None,
            transition: None,
        }
    }
}

/// The Inspector asked to bind a pooled texture to a material slot: `app` looks
/// the decoded image up in its pool and assigns it (auto-detecting the channel
/// routing). The matching "unbind" is [`TextureIntent::Clear`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureAssign {
    pub slot: TextureSlotRef,
    pub path: PathBuf,
}

/// A texture-pool command the Inspector emits (at most one per frame). `app` is
/// the sole applier (invariant 2): all decode / pool / disk-watch work lives
/// there. New texture intents (e.g. the Phase 6 Tex-viewport picks) add a variant
/// here rather than another `Option` field on [`UiOutput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureIntent {
    /// "Add textures…" was clicked: open the image picker + import into the pool.
    Import,
    /// A pooled texture was chosen in a property's dropdown: bind it to the slot.
    Assign(TextureAssign),
    /// A property's dropdown was set back to "select texture": revert that slot to
    /// the shader's neutral fallback.
    Clear(TextureSlotRef),
    /// A pooled texture's remove (✕) was clicked: drop it from the pool and unbind
    /// every material slot that referenced it.
    Remove(PathBuf),
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
}

#[derive(Debug, Clone)]
pub struct NormalPanelState {
    pub length: f32,
    pub color: egui::Color32,
}

/// Editable state backing the Skeleton options panel. The renderer bakes both
/// values into the skeleton overlay's vertex buffers via [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct SkeletonPanelState {
    /// Multiplier on the computed bone thickness / joint-marker size.
    pub scale: f32,
    pub color: egui::Color32,
}

impl Default for SkeletonPanelState {
    fn default() -> Self {
        Self {
            scale: DEFAULT_SKELETON_SCALE,
            color: theme::color::SKELETON_DEFAULT,
        }
    }
}

/// Editable state backing the Wireframe options panel. The renderer bakes the
/// chosen color into the final wireframe overlay line buffer via
/// [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct WireframePanelState {
    pub color: egui::Color32,
}

impl Default for WireframePanelState {
    fn default() -> Self {
        Self {
            color: theme::color::WIREFRAME_DEFAULT,
        }
    }
}

/// Which geometry the bounding box wraps: the whole model, or only the meshes
/// the Outliner currently shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundsScope {
    /// Wrap every mesh, regardless of selection or Outliner visibility (default).
    #[default]
    AllMeshes,
    /// Wrap only the geometry the current Outliner selection covers.
    OnlySelection,
    /// Wrap only the currently-visible meshes (Outliner-hidden meshes excluded).
    VisibleOnly,
}

impl BoundsScope {
    pub const ALL: [BoundsScope; 3] = [
        BoundsScope::AllMeshes,
        BoundsScope::OnlySelection,
        BoundsScope::VisibleOnly,
    ];

    /// The dropdown label for this scope.
    pub fn label(self) -> &'static str {
        match self {
            BoundsScope::AllMeshes => "All Meshes",
            BoundsScope::OnlySelection => "Only Selection",
            BoundsScope::VisibleOnly => "Only Visible",
        }
    }
}

/// Editable state backing the Bounding Box options panel. The renderer bakes the
/// chosen color + scope into the bounding-box line buffer via [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct BoundingBoxPanelState {
    pub color: egui::Color32,
    /// Whether the box covers all meshes or only the visible ones.
    pub scope: BoundsScope,
}

impl Default for BoundingBoxPanelState {
    fn default() -> Self {
        Self {
            color: theme::color::BOUNDING_BOX_DEFAULT,
            scope: BoundsScope::default(),
        }
    }
}

/// Editable state backing the UV Checker options panel. The renderer reads the
/// committed values via [`SceneDebugOptions`]. `tiling` is edited directly on the
/// standard slider's inline value field, so no separate text buffer is needed.
#[derive(Debug, Clone)]
pub struct UvCheckerPanelState {
    pub texture: CheckerTexture,
    pub tiling: u32,
    pub uv_channel: u32,
}

impl Default for UvCheckerPanelState {
    fn default() -> Self {
        Self {
            texture: CheckerTexture::Greyscale,
            tiling: DEFAULT_CHECKER_TILING,
            uv_channel: 0,
        }
    }
}

/// Editable state backing the Vertex Color options panel. The toggle itself
/// lives in [`SceneDebugOptions::vertex_colors`] (like the UV checker); this
/// holds only the channel-display mode, synced via [`SceneDebugOptions`].
#[derive(Debug, Clone, Default)]
pub struct VertexColorPanelState {
    pub mode: VertexColorMode,
}

/// Which tab the Outliner shows: the scene's nodes or the flat deduplicated
/// material list. A cheap click switches between them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlinerTab {
    #[default]
    Scene,
    Materials,
}

/// How the Outliner's Scene tab presents the model: every node in one flat list,
/// or the full node hierarchy as a collapsible tree. Both honor the type filter;
/// the tree additionally indents and draws parent guides. Toggled by the header
/// button, and overridden while a search is active (matches always list flat).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlinerViewMode {
    /// Every node, in model order, with no hierarchy.
    Flat,
    /// The full scene graph, indented and collapsible.
    #[default]
    SceneTree,
}

/// Everything the Outliner side panel owns: which tab and view mode it shows,
/// its search box and type filter, the per-model scene-tree cache it walks, and
/// its keyboard-navigation bookkeeping.
///
/// These group by lifecycle, not by widget: every one of them is scoped to the
/// panel's own session over the loaded model — most are keyed on that model's
/// node indices, and [`UiState::reset_skeletal_state`] drops them together when
/// a new model arrives. Nothing outside `ui` reads any of it, so the fields stay
/// crate-private (`app` never touches the Outliner's view state).
#[derive(Debug, Clone, Default)]
pub struct OutlinerState {
    /// Which Outliner tab is shown (scene nodes vs material list).
    pub(crate) tab: OutlinerTab,
    /// Whether the Scene tab shows every node flat or the full scene tree.
    pub(crate) view: OutlinerViewMode,
    /// The Outliner header's search box. While non-empty it overrides
    /// [`OutlinerState::view`]: both tabs collapse to a flat list of the rows
    /// whose name matches, so a hit is never buried inside a collapsed branch.
    pub(crate) search: String,
    /// Scene-tree nodes the user has *collapsed*. Stored inverted (rather than as
    /// an expanded set) so the default — an empty set — is a fully expanded tree,
    /// with no per-model initialization pass. Cleared on model load.
    pub(crate) collapsed: HashSet<usize>,
    /// Node kinds the scene tree's type filter is currently *hiding*. Empty (the
    /// default) shows everything. A hidden kind's rows still render — greyed and
    /// unselectable — when a visible node lives beneath them, so the hierarchy
    /// never breaks into disconnected fragments.
    pub(crate) hidden_kinds: HashSet<NodeKind>,
    /// Child adjacency for the scene tree (`children[parent]` lists the child
    /// node indices, in model order), built once per model rather than per frame.
    /// Empty means "not built yet"; a model load invalidates it.
    pub(crate) children: Vec<Vec<usize>>,
    /// Root node indices for the scene tree — nodes with no parent, in model
    /// order. Built alongside [`OutlinerState::children`].
    pub(crate) roots: Vec<usize>,
    /// Whether the Outliner owns the arrow keys. Set by clicking a row or by a
    /// handled arrow press, cleared by a pointer press outside the panel, so
    /// navigation survives the pointer wandering back to the viewport without the
    /// Outliner ever swallowing arrows meant for somewhere else.
    pub(crate) nav_focus: bool,
    /// One-frame request from keyboard navigation: the next Outliner draw scrolls
    /// the selected row into view. Cleared by the draw that honors it.
    pub(crate) scroll_to_selection: bool,
}

impl OutlinerState {
    /// Drop the cached scene-tree adjacency so the next Outliner frame rebuilds
    /// it. Called from [`UiState::reset_skeletal_state`], which `app` runs on
    /// model load — where the cached node indices stop meaning anything.
    pub(crate) fn invalidate_tree(&mut self) {
        self.children.clear();
        self.roots.clear();
    }

    /// Build the scene-tree adjacency if it isn't current for `model`. The scan is
    /// O(nodes) and rigs run to hundreds of nodes, so it must not happen per frame
    /// — the cache is rebuilt only after [`OutlinerState::invalidate_tree`].
    ///
    /// A node whose `parent` doesn't resolve (out of range, or itself) is treated as
    /// a root rather than dropped, so a malformed hierarchy still lists every node.
    pub(crate) fn ensure_tree(&mut self, model: &ModelData) {
        if self.children.len() == model.nodes.len() && !model.nodes.is_empty() {
            return;
        }
        self.children = vec![Vec::new(); model.nodes.len()];
        self.roots.clear();
        for (index, node) in model.nodes.iter().enumerate() {
            match node.parent {
                Some(parent) if parent < model.nodes.len() && parent != index => {
                    self.children[parent].push(index);
                }
                _ => self.roots.push(index),
            }
        }
    }
}

/// A tool's options panel. Each is shown as its own native `egui::Window`, so
/// several can be open at once (see [`PanelsOpen`]); a panel is toggled by
/// right-clicking its toolbar / status-bar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionPanel {
    Wireframe,
    MaterialMode,
    BufferView,
    BoundingBox,
    UvChecker,
    FaceNormals,
    VertexNormals,
    Skeleton,
    VertexColors,
    AntiAliasing,
    Background,
    Environment,
    Gtao,
    Tonemap,
}

impl OptionPanel {
    /// Every panel, in toolbar order. Iterated each frame to draw the open ones
    /// (and to give each a stable cascade slot), so the order is deterministic.
    pub(crate) const ALL: [OptionPanel; 14] = [
        OptionPanel::Wireframe,
        OptionPanel::MaterialMode,
        OptionPanel::BufferView,
        OptionPanel::BoundingBox,
        OptionPanel::UvChecker,
        OptionPanel::FaceNormals,
        OptionPanel::VertexNormals,
        OptionPanel::Skeleton,
        OptionPanel::VertexColors,
        OptionPanel::AntiAliasing,
        OptionPanel::Background,
        OptionPanel::Environment,
        OptionPanel::Gtao,
        OptionPanel::Tonemap,
    ];

    /// Title shown in the panel's native window title bar.
    pub(crate) fn title(self) -> &'static str {
        match self {
            OptionPanel::Wireframe => "Wireframe",
            OptionPanel::MaterialMode => "Material Mode",
            OptionPanel::BufferView => "Buffers",
            OptionPanel::BoundingBox => "Bounding Box",
            OptionPanel::UvChecker => "UV Checker",
            OptionPanel::FaceNormals => "Face Normals",
            OptionPanel::VertexNormals => "Vertex Normals",
            OptionPanel::Skeleton => "Skeleton",
            OptionPanel::VertexColors => "Vertex Colors",
            OptionPanel::AntiAliasing => "Anti Aliasing",
            OptionPanel::Background => "Background",
            OptionPanel::Environment => "Environment",
            OptionPanel::Gtao => "Ambient Occlusion",
            OptionPanel::Tonemap => "Tonemapper",
        }
    }

    /// Stable, unique id string for the panel's window, so egui keeps each
    /// window's position/size independent in its memory across frames.
    pub(crate) fn window_id(self) -> &'static str {
        match self {
            OptionPanel::Wireframe => "panel_wireframe",
            OptionPanel::MaterialMode => "panel_material_mode",
            OptionPanel::BufferView => "panel_buffer_view",
            OptionPanel::BoundingBox => "panel_bounding_box",
            OptionPanel::UvChecker => "panel_uv_checker",
            OptionPanel::FaceNormals => "panel_face_normals",
            OptionPanel::VertexNormals => "panel_vertex_normals",
            OptionPanel::Skeleton => "panel_skeleton",
            OptionPanel::VertexColors => "panel_vertex_colors",
            OptionPanel::AntiAliasing => "panel_anti_aliasing",
            OptionPanel::Background => "panel_background",
            OptionPanel::Environment => "panel_environment",
            OptionPanel::Gtao => "panel_gtao",
            OptionPanel::Tonemap => "panel_tonemap",
        }
    }
}

/// The set of option panels currently open. Each open panel is its own native
/// `egui::Window` (egui owns its position/size/collapsed state in memory), so
/// any number can be open simultaneously — a button right-click toggles its
/// panel's membership here.
#[derive(Debug, Clone, Default)]
pub struct PanelsOpen {
    open: HashSet<OptionPanel>,
}

impl PanelsOpen {
    pub(crate) fn is_open(&self, panel: OptionPanel) -> bool {
        self.open.contains(&panel)
    }

    /// Toggle a panel open/closed (the right-click-button behavior).
    pub(crate) fn toggle(&mut self, panel: OptionPanel) {
        if !self.open.remove(&panel) {
            self.open.insert(panel);
        }
    }

    /// Force a panel's open state (used when egui's window X-button closes it).
    pub(crate) fn set(&mut self, panel: OptionPanel, open: bool) {
        if open {
            self.open.insert(panel);
        } else {
            self.open.remove(&panel);
        }
    }
}

/// The measurements the chrome derives from the loaded model that are too
/// expensive to redo every frame, each stored beside the *input* it was computed
/// for so a change to that input — and nothing else — rebuilds it.
///
/// What binds them is one lifecycle, not one panel: every entry is a fact about
/// the currently-loaded [`ModelData`], keyed on node indices or a [`Selection`]
/// that mean nothing once a different model is on screen. [`BoundsCaches::reset`]
/// drops the whole set at once, so that invalidation is stated here instead of
/// riding on `app` happening to clear the inputs each key is compared against.
#[derive(Debug, Clone, Default)]
pub struct BoundsCaches {
    /// Cached `model.visible_bounds(hidden)` for the dimension-label overlay's
    /// "visible only" box. That scan is O(triangles); it must not run per-frame,
    /// so it's rebuilt only when [`BoundsCaches::visible_bounds_key`] (the hidden
    /// set it was computed for) no longer matches the live hidden set. Unused —
    /// the overlay falls back to [`UiState::bounds`] — when nothing is hidden or
    /// the whole-model box is shown.
    visible_bounds: Option<Bounds>,
    /// The sorted hidden-node set [`BoundsCaches::visible_bounds`] was built for;
    /// a mismatch with the live hidden set invalidates the cache.
    visible_bounds_key: Vec<u32>,
    /// Cached `selection_bounds(selection)` for the dimension-label overlay's
    /// "only selection" box (same O(triangles) caching as the visible-only box,
    /// keyed by the selection it was computed for).
    selection_bounds: Option<Bounds>,
    /// The selection [`BoundsCaches::selection_bounds`] was built for; a mismatch
    /// with the live selection invalidates the cache.
    selection_bounds_key: Selection,
    /// How many logical source vertices the current bone selection influences,
    /// shown by the Inspector. The scan is O(influences) — 168k on a game
    /// character — so it must not run per frame; it is recomputed only when
    /// [`BoundsCaches::bone_influence_key`] no longer matches the live selection.
    pub(crate) bone_influence: usize,
    /// The sorted bone set [`BoundsCaches::bone_influence`] was measured for; a
    /// mismatch with the live selection invalidates it.
    bone_influence_key: Vec<u32>,
}

impl BoundsCaches {
    /// Drop every cached measurement, so the next frame that needs one rebuilds
    /// it against the model now on screen.
    ///
    /// `app` calls this on model load. Each key is a node-index list or a
    /// [`Selection`] naming the *outgoing* model, so an incoming model that
    /// happens to reproduce one — the same node hidden again, the same node
    /// selected again — would otherwise be served the previous model's box.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What the running build and the active adapter can do — the facts `app`
/// establishes once during startup and never revises.
///
/// The lifecycle is what groups them: each is written at most once (in `App`'s
/// construction or its device bootstrap) and is read-only for the rest of the
/// session, so nothing in `ui` ever needs a `&mut` to one. That is why the
/// capability seams live here rather than beside the settings they gate — the
/// settings change every frame, these never do.
#[derive(Debug, Clone)]
pub struct Capabilities {
    /// MSAA levels the active adapter actually supports, set by `app` from the
    /// device's `supported_msaa_counts()` (D3D11 `CheckMultisampleQualityLevels`).
    /// The Anti Aliasing menu disables any level not in this list (invariant 4).
    /// Empty until the adapter is known (the panel then falls back to offering only
    /// the current level).
    pub msaa_levels: Vec<MsaaSamples>,
    /// Whether the active adapter can build the IBL maps. Always true on the D3D11
    /// target (the device hard-requires `TEXTURE_COMPRESSION_BC` for the BC6H IBL
    /// cubes, and 11_0+ guarantees the float formats), so the Environment panel never
    /// disables the IBL toggle in practice; kept as a field for the capability seam.
    pub(crate) ibl: bool,
    /// Whether the active adapter can run GTAO. Always true on the D3D11 target
    /// (the G-buffer + horizon passes need only float render targets + samplers
    /// guaranteed at feature level 11_0+), so the status-bar AO button is never
    /// disabled in practice; kept as a field for the capability seam.
    pub(crate) gtao: bool,
    /// Friendly name of the graphics backend (e.g. "DX11"), shown in the help
    /// overlay title; set by `app` (the renderer is always Direct3D 11).
    pub gpu_backend: String,
    /// Application version shown in the help overlay title (e.g. "0.1.0"), set by
    /// `app` from its `CARGO_PKG_VERSION`.
    pub app_version: String,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            msaa_levels: Vec::new(),
            // Assume supported until the adapter is queried; `app` corrects these
            // once the device is known.
            ibl: true,
            gtao: true,
            gpu_backend: String::new(),
            app_version: String::new(),
        }
    }
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
    /// the D3D11 Tex draw (migration Phase 4). `None` until the Tex viewport has been
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
    pub wireframe: WireframePanelState,
    pub bounding_box: BoundingBoxPanelState,
    pub face_normals: NormalPanelState,
    pub vertex_normals: NormalPanelState,
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
    /// The Outliner selection (a node, a material, or nothing). Set by clicking a
    /// row in the Outliner; drives the viewport highlight + solo and the Inspector.
    pub selection: Selection,
    /// Whether the selection is isolated (solo): only the selected geometry is
    /// drawn. A no-op while nothing is selected.
    pub solo: bool,
    /// Selection-flash fade, set by `app` each frame (data flows app→UI, invariant
    /// 2; the redraw animation lives in `app`, invariant 6). Runs 1→0 over ~0.5s
    /// after a selection change, modulating the viewport highlight fill's alpha so
    /// the flash blinks then fades. 0 when no flash is playing.
    pub selection_fade: f32,
    /// The Outliner side panel's own state: tab, view mode, search, type filter,
    /// scene-tree cache and keyboard-navigation flags. Grouped by lifecycle —
    /// see [`OutlinerState`].
    pub outliner: OutlinerState,
    /// Bone nodes selected in the Outliner, in click order (the last entry is the
    /// primary, mirrored into [`UiState::selection`]). Drives the skeleton
    /// overlay's highlight and the skin-weight heat map. Ctrl-click toggles a
    /// member, Shift-click takes a range; clicking any non-bone row clears it.
    ///
    /// Kept beside [`UiState::selection`] rather than inside it because
    /// [`Selection`] is `Copy` and threaded through renderer bake keys and undo
    /// snapshots, where a growable set would be the wrong shape.
    pub selected_bones: Vec<usize>,
    /// Anchor row for Shift-click range selection: the last plainly-clicked or
    /// Ctrl-clicked bone. `None` until a bone is clicked.
    pub bone_anchor: Option<usize>,
    /// Whether the loaded model carries any bone node. Gates the skeleton toolbar
    /// button (hidden entirely for an unrigged model). Set by `app` on load.
    pub has_bones: bool,
    /// Whether the loaded model carries skin weights. Gates the Skin Weights
    /// material-mode button. Set by `app` on load.
    pub has_skin: bool,
    /// Mesh nodes the user has hidden via the Outliner's per-row visibility
    /// checkbox (node indices into [`review_model::ModelData::nodes`]). The scene
    /// callback filters these meshes' triangles out of the viewport draw + GTAO
    /// (Phase 2). Cleared by `app` on model load (the indices no longer apply).
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
    /// The per-model derived measurements that are too costly to recompute every
    /// frame (the visible-only and selection-only boxes, the bone-influence
    /// count). `app` resets the whole set on model load — see [`BoundsCaches`].
    pub caches: BoundsCaches,
    /// Most recent measured frames-per-second, fed by `app` from the render
    /// loop. Zero while idle (the viewer redraws on demand, not continuously).
    pub fps: f32,
    /// Whether the startup help overlay (keyboard-shortcut cheat sheet) is shown.
    /// Starts `true` so it greets the user on launch, and is cleared by a click
    /// anywhere (see `help::draw_help_overlay`).
    pub show_help_overlay: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
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
            selection_fade: 0.0,
            outliner: OutlinerState::default(),
            selected_bones: Vec::new(),
            bone_anchor: None,
            has_bones: false,
            has_skin: false,
            hidden_meshes: HashSet::new(),
            side_panels_open: true,
            bounds: None,
            caches: BoundsCaches::default(),
            fps: 0.0,
            show_help_overlay: true,
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
                if self.caches.selection_bounds_key != self.selection {
                    self.caches.selection_bounds = selection_bounds(model, self.selection);
                    self.caches.selection_bounds_key = self.selection;
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

    /// The selection view the renderer reads each frame (invariant 2: a plain
    /// value): what is selected, whether it is isolated (solo), the gamma-space
    /// highlight color sourced from the theme, and the live flash fade.
    pub fn selection_view(&self) -> review_render::SelectionView {
        review_render::SelectionView {
            selection: self.selection,
            solo: self.solo,
            highlight_color: theme::color32_to_rgba(theme::color::SELECTION_OUTLINE),
            fade: self.selection_fade,
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

    /// Re-point every piece of skeleton/skin UI state at a freshly loaded `model`:
    /// drop selections and caches keyed by the old model's node indices, and
    /// re-derive the capability flags that gate the skeleton toolbar button and the
    /// Skin Weights material mode.
    pub fn reset_skeletal_state(&mut self, model: &ModelData) {
        self.selected_bones.clear();
        self.bone_anchor = None;
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
    state.debug.wireframe_color = theme::color32_to_rgba(state.wireframe.color);
    state.debug.bounding_box_color = theme::color32_to_rgba(state.bounding_box.color);
    state.debug.bounding_box_scope = match state.bounding_box.scope {
        BoundsScope::AllMeshes => BoundingBoxScope::AllMeshes,
        BoundsScope::OnlySelection => BoundingBoxScope::OnlySelection,
        BoundsScope::VisibleOnly => BoundingBoxScope::VisibleOnly,
    };
    state.debug.bounding_box_selection = state.selection;
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
        let labels: Vec<&str> = TextureChannelView::ALL
            .into_iter()
            .map(|channel| channel.label())
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
