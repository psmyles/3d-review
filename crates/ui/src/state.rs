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
use review_model::{Bounds, ModelData, ModelStats};
use review_render::{
    AntiAliasing, BloomSettings, CameraProjection, CheckerTexture, DecodedImage,
    EnvironmentSettings, GtaoSettings, MaterialEdit, MaterialSnapshot, MsaaSamples,
    SceneDebugOptions, Selection, ShadingMode, TonemapSettings, UvShadingMode, VertexColorMode,
};

use crate::theme;

/// Default checker repeats across the 0..1 UV range for a fresh / reset panel.
pub(crate) const DEFAULT_CHECKER_TILING: u32 = 4;
/// Inclusive checker-tiling range enforced by the UV-checker panel.
pub(crate) const CHECKER_TILING_MIN: u32 = 1;
pub(crate) const CHECKER_TILING_MAX: u32 = 16;
/// Default length for the face/vertex normal-line views, as a fraction of the
/// model's largest bounding extent (see `debug_normal_length` in `review-render`).
pub(crate) const DEFAULT_NORMAL_LENGTH: f32 = 0.03;
/// Inclusive normal-length range: 0.1%–10% of the model's largest bounding extent.
pub(crate) const NORMAL_LENGTH_MIN: f32 = 0.001;
pub(crate) const NORMAL_LENGTH_MAX: f32 = 0.10;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    ThreeD,
    Uv,
    Texture,
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
            .unwrap_or_else(|| "—".to_owned())
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
/// [`crate::theme::size::TEXTURE_ZOOM_ANIM_SECS`]. `start_time` is egui's input
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
}

#[derive(Debug, Clone)]
pub struct NormalPanelState {
    pub length: f32,
    pub color: egui::Color32,
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
    /// Wrap every mesh, regardless of Outliner visibility (the default).
    #[default]
    AllMeshes,
    /// Wrap only the currently-visible meshes (Outliner-hidden meshes excluded).
    VisibleOnly,
}

impl BoundsScope {
    pub const ALL: [BoundsScope; 2] = [BoundsScope::AllMeshes, BoundsScope::VisibleOnly];

    /// The dropdown label for this scope.
    pub fn label(self) -> &'static str {
        match self {
            BoundsScope::AllMeshes => "all meshes",
            BoundsScope::VisibleOnly => "visible only",
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

/// Which tab the Outliner shows: the flat list of mesh objects or the flat
/// deduplicated material list. A cheap click switches between them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutlinerTab {
    #[default]
    Geometry,
    Materials,
}

/// A tool's options panel. Each is shown as its own native `egui::Window`, so
/// several can be open at once (see [`PanelsOpen`]); a panel is toggled by
/// right-clicking its toolbar / status-bar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionPanel {
    Wireframe,
    BoundingBox,
    UvChecker,
    FaceNormals,
    VertexNormals,
    VertexColors,
    AntiAliasing,
    Environment,
    Bloom,
    Gtao,
    Tonemap,
}

impl OptionPanel {
    /// Every panel, in toolbar order. Iterated each frame to draw the open ones
    /// (and to give each a stable cascade slot), so the order is deterministic.
    pub(crate) const ALL: [OptionPanel; 11] = [
        OptionPanel::Wireframe,
        OptionPanel::BoundingBox,
        OptionPanel::UvChecker,
        OptionPanel::FaceNormals,
        OptionPanel::VertexNormals,
        OptionPanel::VertexColors,
        OptionPanel::AntiAliasing,
        OptionPanel::Environment,
        OptionPanel::Bloom,
        OptionPanel::Gtao,
        OptionPanel::Tonemap,
    ];

    /// Title shown in the panel's native window title bar.
    pub(crate) fn title(self) -> &'static str {
        match self {
            OptionPanel::Wireframe => "Wireframe",
            OptionPanel::BoundingBox => "Bounding Box",
            OptionPanel::UvChecker => "UV Checker",
            OptionPanel::FaceNormals => "Face Normals",
            OptionPanel::VertexNormals => "Vertex Normals",
            OptionPanel::VertexColors => "Vertex Colors",
            OptionPanel::AntiAliasing => "Anti Aliasing",
            OptionPanel::Environment => "Environment",
            OptionPanel::Bloom => "Bloom",
            OptionPanel::Gtao => "Ambient Occlusion",
            OptionPanel::Tonemap => "Tonemapper",
        }
    }

    /// Stable, unique id string for the panel's window, so egui keeps each
    /// window's position/size independent in its memory across frames.
    pub(crate) fn window_id(self) -> &'static str {
        match self {
            OptionPanel::Wireframe => "panel_wireframe",
            OptionPanel::BoundingBox => "panel_bounding_box",
            OptionPanel::UvChecker => "panel_uv_checker",
            OptionPanel::FaceNormals => "panel_face_normals",
            OptionPanel::VertexNormals => "panel_vertex_normals",
            OptionPanel::VertexColors => "panel_vertex_colors",
            OptionPanel::AntiAliasing => "panel_anti_aliasing",
            OptionPanel::Environment => "panel_environment",
            OptionPanel::Bloom => "panel_bloom",
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
    pub wireframe: WireframePanelState,
    pub bounding_box: BoundingBoxPanelState,
    pub face_normals: NormalPanelState,
    pub vertex_normals: NormalPanelState,
    pub vertex_colors: VertexColorPanelState,
    /// Scene antialiasing (MSAA level + FXAA). Read straight by the viewport
    /// callback — not a debug option — and edited by the Anti Aliasing panel.
    pub anti_aliasing: AntiAliasing,
    /// MSAA levels the active adapter actually supports, set by `app` from
    /// [`review_render::supported_msaa_levels`]. The Anti Aliasing menu disables
    /// any level not in this list (invariant 4). Empty until the adapter is known
    /// (the panel then falls back to offering only the current level).
    pub supported_msaa: Vec<MsaaSamples>,
    /// Image-based lighting / environment selection. Read straight by the
    /// viewport callback (not a debug option) and edited by the Environment
    /// panel. Default is IBL on, HDR 01, no background (see [`EnvironmentSettings`]).
    pub environment: EnvironmentSettings,
    /// Whether the active adapter can build the IBL maps, set by `app` from
    /// [`review_render::ibl_supported`]. The Environment panel disables (and
    /// forces off) the IBL toggle when false (invariant 4).
    pub ibl_supported: bool,
    /// Bloom (HDR glow) settings. Read straight by the viewport callback (not a
    /// debug option) and edited by the Bloom panel; the bloom status-bar button
    /// toggles `bloom.enabled`. Default is off (see [`BloomSettings`]).
    pub bloom: BloomSettings,
    /// Ambient occlusion (GTAO) settings. Read straight by the viewport
    /// callback and edited by the Ambient Occlusion panel; the AO status-bar
    /// button toggles `gtao.enabled`. Default is on (see [`GtaoSettings`]).
    pub gtao: GtaoSettings,
    /// Whether the active adapter can run GTAO, set by `app` from
    /// [`review_render::gtao_supported`]. The status-bar AO button is disabled
    /// (and forced off) when false (invariant 4).
    pub gtao_supported: bool,
    /// Tone-mapping settings. Read straight by the viewport callback (not a debug
    /// option) and edited by the Tonemapper panel; the status-bar tonemapper button
    /// toggles `tonemap.enabled`. Default is on with Khronos PBR Neutral (see
    /// [`TonemapSettings`]).
    pub tonemap: TonemapSettings,
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
    /// Which Outliner tab is shown (mesh list vs material list).
    pub outliner_tab: OutlinerTab,
    /// Case-insensitive substring filter applied to the Outliner rows.
    pub outliner_filter: String,
    /// Mesh nodes the user has hidden via the Outliner's per-row visibility
    /// checkbox (node indices into [`review_model::ModelData::nodes`]). The scene
    /// callback filters these meshes' triangles out of the viewport draw + GTAO
    /// (Phase 2). Cleared by `app` on model load (the indices no longer apply).
    pub hidden_meshes: HashSet<usize>,
    /// Whether the dockable Outliner side panel (left) is open. egui owns its
    /// resized width; the UI only tracks open/closed.
    pub outliner_open: bool,
    /// Whether the dockable Inspector side panel (right) is open.
    pub inspector_open: bool,
    /// Axis-aligned bounds of the loaded model (world meters), set by `app`
    /// alongside [`UiState::stats`] (invariant 2: a plain value, not model
    /// ownership). `None` when no model is loaded. Read by the dimension-label
    /// overlay to place each box edge's axis-length readout.
    pub bounds: Option<Bounds>,
    /// Cached `model.visible_bounds(hidden)` for the dimension-label overlay's
    /// "visible only" box. That scan is O(triangles); it must not run per-frame,
    /// so it's rebuilt only when [`UiState::visible_bounds_key`] (the hidden set
    /// it was computed for) no longer matches the live hidden set. Unused — the
    /// overlay falls back to [`UiState::bounds`] — when nothing is hidden or the
    /// whole-model box is shown.
    pub visible_bounds_cache: Option<Bounds>,
    /// The sorted hidden-node set [`UiState::visible_bounds_cache`] was built for;
    /// a mismatch with the live hidden set invalidates the cache.
    pub visible_bounds_key: Vec<u32>,
    /// Most recent measured frames-per-second, fed by `app` from the render
    /// loop. Zero while idle (the viewer redraws on demand, not continuously).
    pub fps: f32,
    /// Whether the startup help overlay (keyboard-shortcut cheat sheet) is shown.
    /// Starts `true` so it greets the user on launch, and is cleared by a click
    /// anywhere (see `help::draw_help_overlay`).
    pub show_help_overlay: bool,
    /// Application version shown in the help overlay title (e.g. "0.1.0"), set by
    /// `app` from its `CARGO_PKG_VERSION`.
    pub app_version: String,
    /// Friendly name of the wgpu backend wgpu actually selected (e.g. "DX12"),
    /// shown in the help overlay title; set by `app` once the adapter is known.
    pub gpu_backend: String,
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
            uv_checker: UvCheckerPanelState::default(),
            uv_sets: Vec::new(),
            uv_view_channel: 0,
            uv_shading_mode: UvShadingMode::default(),
            texture_view: TextureViewState::default(),
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
            vertex_colors: VertexColorPanelState::default(),
            anti_aliasing: AntiAliasing::default(),
            supported_msaa: Vec::new(),
            environment: EnvironmentSettings::default(),
            // Assume supported until the adapter is queried; `app` corrects this
            // once the device is known.
            ibl_supported: true,
            bloom: BloomSettings::default(),
            gtao: GtaoSettings::default(),
            gtao_supported: true,
            tonemap: TonemapSettings::default(),
            stats: ModelStats::default(),
            materials_snapshot: Vec::new(),
            texture_pool: Vec::new(),
            material_revision: 0,
            selection: Selection::None,
            solo: false,
            selection_fade: 0.0,
            outliner_tab: OutlinerTab::default(),
            outliner_filter: String::new(),
            hidden_meshes: HashSet::new(),
            outliner_open: false,
            inspector_open: false,
            bounds: None,
            visible_bounds_cache: None,
            visible_bounds_key: Vec::new(),
            fps: 0.0,
            show_help_overlay: true,
            app_version: String::new(),
            gpu_backend: String::new(),
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
        if !self.debug.bounding_box_visible_only {
            return self.bounds;
        }
        let mut hidden: Vec<u32> = self.hidden_meshes.iter().map(|&i| i as u32).collect();
        hidden.sort_unstable();
        // An empty hidden set makes `visible_bounds` the whole-model box, so skip
        // both the scan and the cache. (Model loads clear the hidden set, so the
        // cache below is never served across a model swap.)
        if hidden.is_empty() {
            return self.bounds;
        }
        if self.visible_bounds_key != hidden {
            self.visible_bounds_cache = model.visible_bounds(&hidden);
            self.visible_bounds_key = hidden;
        }
        self.visible_bounds_cache
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
    state.debug.bounding_box_visible_only =
        matches!(state.bounding_box.scope, BoundsScope::VisibleOnly);
}
