//! UI state and the intents the UI emits back to `app`.
//!
//! Per invariant 2 this crate holds plain values + displayed stats and emits
//! [`UiOutput`] *intents*; it never owns or mutates renderer/model internals.
//! [`sync_debug_state`] funnels the committed panel values into the
//! [`SceneDebugOptions`] the renderer reads.

use glam::{Vec2, Vec3};
use review_model::ModelStats;
use review_render::{
    AntiAliasing, BloomSettings, CameraProjection, CheckerTexture, EnvironmentSettings,
    MsaaSamples, SceneDebugOptions, ShadingMode, SsaoSettings, UvShadingMode, VertexColorMode,
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
pub(crate) const DEFAULT_WIREFRAME_SCREEN_THICKNESS: f32 = 1.0;
pub(crate) const WIREFRAME_SCREEN_THICKNESS_MIN: f32 = 0.1;
pub(crate) const WIREFRAME_SCREEN_THICKNESS_MAX: f32 = 2.0;
pub(crate) const DEFAULT_WIREFRAME_WORLD_THICKNESS: f32 = 0.0005;
pub(crate) const WIREFRAME_WORLD_THICKNESS_MIN: f32 = 0.0001;
pub(crate) const WIREFRAME_WORLD_THICKNESS_MAX: f32 = 0.001;
/// World thickness is stored in true world units (mm-scale fractions) but shown
/// on a friendlier 0.1–1.0 scale in the panel. Shown value = actual * SCALE, so
/// the UI reads 0.1–1.0 while the renderer keeps the real 0.0001–0.001 length.
pub(crate) const WIREFRAME_WORLD_THICKNESS_DISPLAY_SCALE: f32 = 1000.0;
pub(crate) const WIREFRAME_WORLD_THICKNESS_DISPLAY_MIN: f32 =
    WIREFRAME_WORLD_THICKNESS_MIN * WIREFRAME_WORLD_THICKNESS_DISPLAY_SCALE;
pub(crate) const WIREFRAME_WORLD_THICKNESS_DISPLAY_MAX: f32 =
    WIREFRAME_WORLD_THICKNESS_MAX * WIREFRAME_WORLD_THICKNESS_DISPLAY_SCALE;

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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UiOutput {
    pub axis_gizmo_action: Option<AxisGizmoAction>,
}

#[derive(Debug, Clone)]
pub struct NormalPanelState {
    pub length: f32,
    pub color: egui::Color32,
}

/// Editable state backing the Wireframe options panel. The renderer bakes the
/// chosen color into the final wireframe overlay segment buffer via
/// [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct WireframePanelState {
    pub color: egui::Color32,
    pub screen_thickness: f32,
    pub world_thickness: f32,
    pub use_world_units: bool,
}

impl Default for WireframePanelState {
    fn default() -> Self {
        Self {
            color: theme::color::WIREFRAME_DEFAULT,
            screen_thickness: DEFAULT_WIREFRAME_SCREEN_THICKNESS,
            world_thickness: DEFAULT_WIREFRAME_WORLD_THICKNESS,
            use_world_units: false,
        }
    }
}

/// Editable state backing the Bounding Box options panel. The renderer bakes the
/// chosen color into the bounding-box line buffer via [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct BoundingBoxPanelState {
    pub color: egui::Color32,
}

impl Default for BoundingBoxPanelState {
    fn default() -> Self {
        Self {
            color: theme::color::BOUNDING_BOX_DEFAULT,
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

/// Which tool's options panel is currently open. Only one panel is shown at a
/// time; a panel is opened by right-clicking its toolbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Ssao,
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
    /// The single options panel currently shown, if any.
    pub active_panel: Option<OptionPanel>,
    /// Whether the active panel is collapsed to just its header bar. Toggled by
    /// a single click on the header; reset to expanded whenever a panel opens.
    pub panel_collapsed: bool,
    /// Last viewport position the panel was dragged to (egui points). Shared by
    /// every option panel so they all spawn where the last one was left.
    pub panel_pos: Option<egui::Pos2>,
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
    /// toggles `bloom.enabled`. Default is on (see [`BloomSettings`]).
    pub bloom: BloomSettings,
    /// Screen-space ambient occlusion settings. Read straight by the viewport
    /// callback and edited by the Ambient Occlusion panel; the SSAO status-bar
    /// button toggles `ssao.enabled`. Default is on (see [`SsaoSettings`]).
    pub ssao: SsaoSettings,
    /// Whether the active adapter can run SSAO, set by `app` from
    /// [`review_render::ssao_supported`]. The status-bar SSAO button is disabled
    /// (and forced off) when false (invariant 4).
    pub ssao_supported: bool,
    pub stats: ModelStats,
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
            active_panel: None,
            panel_collapsed: false,
            panel_pos: None,
            uv_checker: UvCheckerPanelState::default(),
            uv_sets: Vec::new(),
            uv_view_channel: 0,
            uv_shading_mode: UvShadingMode::default(),
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
            ssao: SsaoSettings::default(),
            ssao_supported: true,
            stats: ModelStats::default(),
            fps: 0.0,
            show_help_overlay: true,
            app_version: String::new(),
            gpu_backend: String::new(),
        }
    }
}

impl UiState {
    /// Open (or switch to) a tool's options panel. The shared drag position is
    /// preserved so it spawns where the last panel was left.
    pub(crate) fn open_panel(&mut self, panel: OptionPanel) {
        // Right-clicking the same tool again closes its panel; right-clicking a
        // different tool switches the (single) panel to it.
        if self.active_panel == Some(panel) {
            self.active_panel = None;
        } else {
            self.active_panel = Some(panel);
            // A freshly opened (or switched-to) panel always starts expanded.
            self.panel_collapsed = false;
        }
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
    state.debug.wireframe_screen_thickness = state.wireframe.screen_thickness;
    state.debug.wireframe_world_thickness = state.wireframe.world_thickness;
    state.debug.wireframe_use_world_units = state.wireframe.use_world_units;
    state.debug.bounding_box_color = theme::color32_to_rgba(state.bounding_box.color);
}
