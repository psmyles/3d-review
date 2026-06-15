use std::sync::Arc;

use glam::{Vec2, Vec3};
use image::ImageError;
use review_model::{ModelData, ModelStats};
use review_render::{
    CameraProjection, CheckerTexture, OrbitCamera, SceneCallback, SceneDebugOptions, ShadingMode,
};

const TOOLBAR_HEIGHT_PX: f32 = 73.0;
const STATUS_BAR_HEIGHT_PX: f32 = 64.0;
const OVERLAY_MARGIN_PX: f32 = 12.0;
const LEFT_PANEL_WIDTH_PX: f32 = 432.0;
const TOOLBAR_GROUP_SPACING_PX: f32 = 14.0;
const TOOLBAR_ICON_SIZE_PX: f32 = 42.0;
const TOOLBAR_ICON_GAP_PX: f32 = 3.0;
const TOOLBAR_ICON_PADDING_PX: f32 = 8.0;
const TOOLBAR_CENTER_WIDTH_PX: f32 = 180.0;
const TOOLBAR_RIGHT_WIDTH_PX: f32 = 153.0;
const TOOLBAR_LEFT_WIDTH_PX: f32 = 333.0;
const TOOLBAR_SHADING_GROUP_WIDTH_PX: f32 = 183.0;
const TOOLBAR_DEBUG_GROUP_WIDTH_PX: f32 = 138.0;
const TOOLBAR_SINGLE_ICON_GROUP_WIDTH_PX: f32 = 48.0;
const TOOLBAR_DOUBLE_ICON_GROUP_WIDTH_PX: f32 = 93.0;
const TOOLBAR_MODE_GROUP_WIDTH_PX: f32 = 180.0;
const TOOLBAR_GROUP_HEIGHT_PX: f32 = 48.0;
const TOOLBAR_GROUP_PADDING_PX: f32 = 3.0;
const STATS_FONT_SIZE_PX: f32 = 12.0;
const STATS_ROW_SPACING_PX: f32 = 3.0;
const STATS_PANEL_WIDTH_PX: f32 = 132.0;
const STATS_PANEL_PAD_X_PX: i8 = 9;
const STATS_PANEL_PAD_Y_PX: i8 = 9;
/// Inset of the stats overlay from the left and bottom viewport edges. Kept
/// equal so the panel reads as equidistant from both.
const STATS_OVERLAY_MARGIN_PX: f32 = 18.0;
const GIZMO_SIZE_PX: f32 = 156.0;
const GIZMO_INSET_PX: f32 = 26.0;
const GIZMO_REACH_PX: f32 = 52.0;
const GIZMO_BALL_RADIUS_PX: f32 = 13.0;
const GIZMO_LABEL_SIZE_PX: f32 = 16.0;
const GIZMO_LABEL_NEG_SIZE_PX: f32 = 15.0;
const GIZMO_NEG_OPACITY: f32 = 0.05;
/// View-space depth above which an axis is treated as pointing at the viewer
/// (i.e. the camera is snapped down that axis).
const GIZMO_AXIS_ALIGNED_DEPTH: f32 = 0.99;

struct AppIcon {
    id: &'static str,
    png_bytes: &'static [u8],
}

const ICON_SHADING_WIRE: AppIcon = AppIcon {
    id: "icon_shading_wire",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_wire.png"),
};
const ICON_SHADING_UNLIT: AppIcon = AppIcon {
    id: "icon_shading_unlit",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_unlit.png"),
};
const ICON_SHADING_SOLID: AppIcon = AppIcon {
    id: "icon_shading_solid",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_solid.png"),
};
const ICON_SHADING_WIRE_SHADED: AppIcon = AppIcon {
    id: "icon_shading_wire_shaded",
    png_bytes: include_bytes!("../../../assets/icons/icon_shading_wire_shaded.png"),
};
const ICON_UV: AppIcon = AppIcon {
    id: "icon_uv",
    png_bytes: include_bytes!("../../../assets/icons/icon_uv.png"),
};
const ICON_NORMALS_FACE: AppIcon = AppIcon {
    id: "icon_normals_face",
    png_bytes: include_bytes!("../../../assets/icons/icon_normals_face.png"),
};
const ICON_NORMALS_VERTEX: AppIcon = AppIcon {
    id: "icon_normals_vertex",
    png_bytes: include_bytes!("../../../assets/icons/icon_normals_vertex.png"),
};
const ICON_VIEW_ORTHO: AppIcon = AppIcon {
    id: "icon_view_ortho",
    png_bytes: include_bytes!("../../../assets/icons/icon_view_ortho.png"),
};
const ICON_VIEW_PERSPECTIVE: AppIcon = AppIcon {
    id: "icon_view_perspective",
    png_bytes: include_bytes!("../../../assets/icons/icon_view_perspective.png"),
};
const ICON_AXIS_GIZMO: AppIcon = AppIcon {
    id: "icon_empty_axis",
    png_bytes: include_bytes!("../../../assets/icons/icon_empty_axis.png"),
};
const ICON_GRID: AppIcon = AppIcon {
    id: "icon_grid",
    png_bytes: include_bytes!("../../../assets/icons/icon_grid.png"),
};
const ICON_INFO: AppIcon = AppIcon {
    id: "icon_info",
    png_bytes: include_bytes!("../../../assets/icons/icon_info.png"),
};
const ICON_CLOSE: AppIcon = AppIcon {
    id: "icon_close",
    png_bytes: include_bytes!("../../../assets/icons/icon_close.png"),
};

/// A custom font embedded into the binary and registered with egui at startup.
struct CustomFont {
    /// Unique key egui uses to reference this font.
    name: &'static str,
    /// Raw .ttf / .otf bytes, embedded via `include_bytes!`.
    ttf_bytes: &'static [u8],
    /// Family this font becomes the primary face for.
    family: egui::FontFamily,
}

/// Fonts bundled with the app, each made the default (index 0) for its family.
/// Both are variable fonts; egui/ab_glyph rasterizes their default instance
/// (regular weight), which is what we want for UI text.
const CUSTOM_FONTS: &[CustomFont] = &[
    CustomFont {
        name: "inter",
        ttf_bytes: include_bytes!("../../../assets/fonts/InterVariable.ttf"),
        family: egui::FontFamily::Proportional,
    },
    CustomFont {
        name: "jetbrains_mono",
        ttf_bytes: include_bytes!("../../../assets/fonts/JetBrainsMono.ttf"),
        family: egui::FontFamily::Monospace,
    },
];

/// Install bundled custom fonts into the egui context. Call once at startup
/// (the `app` coordinator does this when it builds the `egui::Context`).
///
/// Fonts are embedded at compile time via `include_bytes!`, matching how icons
/// are bundled. To add a font, drop the file in `assets/fonts/` and append a
/// [`CustomFont`] to [`CUSTOM_FONTS`]. Each font is inserted at index 0 of its
/// family so it shadows egui's built-in default while keeping the built-ins as
/// fallbacks for missing glyphs.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for font in CUSTOM_FONTS {
        fonts.font_data.insert(
            font.name.to_owned(),
            Arc::new(egui::FontData::from_static(font.ttf_bytes)),
        );
        fonts
            .families
            .entry(font.family.clone())
            .or_default()
            .insert(0, font.name.to_owned());
    }
    ctx.set_fonts(fonts);
}

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

/// Editable state backing the UV Checker options panel. The renderer reads the
/// committed values via [`SceneDebugOptions`]; `tiling_text` is the panel's own
/// text-field buffer (kept out of the render options so `render` stays free of
/// UI string state) and is re-sanitized to an integer on commit.
#[derive(Debug, Clone)]
pub struct UvCheckerPanelState {
    pub texture: CheckerTexture,
    pub tiling: u32,
    pub tiling_text: String,
    pub uv_channel: u32,
}

impl Default for UvCheckerPanelState {
    fn default() -> Self {
        Self {
            texture: CheckerTexture::Greyscale,
            tiling: DEFAULT_CHECKER_TILING,
            tiling_text: DEFAULT_CHECKER_TILING.to_string(),
            uv_channel: 0,
        }
    }
}

/// Which tool's options panel is currently open. Only one panel is shown at a
/// time; a panel is opened by right-clicking its toolbar button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionPanel {
    UvChecker,
    FaceNormals,
    VertexNormals,
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
    pub face_normals: NormalPanelState,
    pub vertex_normals: NormalPanelState,
    pub stats: ModelStats,
    /// Most recent measured frames-per-second, fed by `app` from the render
    /// loop. Zero while idle (the viewer redraws on demand, not continuously).
    pub fps: f32,
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
            face_normals: NormalPanelState {
                length: 0.18,
                color: egui::Color32::from_rgb(255, 32, 32),
            },
            vertex_normals: NormalPanelState {
                length: 0.18,
                color: egui::Color32::from_rgb(32, 224, 232),
            },
            stats: ModelStats::default(),
            fps: 0.0,
        }
    }
}

impl UiState {
    /// Open (or switch to) a tool's options panel. The shared drag position is
    /// preserved so it spawns where the last panel was left.
    fn open_panel(&mut self, panel: OptionPanel) {
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

pub fn draw_viewport_scene(
    ctx: &egui::Context,
    state: &UiState,
    camera: OrbitCamera,
    model: Arc<ModelData>,
    model_revision: u64,
    output_format: egui_wgpu::wgpu::TextureFormat,
) {
    if state.mode != WorkspaceMode::ThreeD {
        return;
    }

    let rect = ctx.input(|input| input.screen_rect());
    let painter = ctx.layer_painter(egui::LayerId::background());
    let callback = egui_wgpu::Callback::new_paint_callback(
        rect,
        SceneCallback::new(
            camera,
            state.projection_mode.into(),
            output_format,
            model,
            model_revision,
            state.debug,
        ),
    );
    painter.add(callback);
}

pub fn draw_overlay(ctx: &egui::Context, state: &mut UiState, camera: OrbitCamera) -> UiOutput {
    apply_visuals(ctx);
    sync_debug_state(state);
    let mut output = UiOutput::default();

    let toolbar_height = px(ctx, TOOLBAR_HEIGHT_PX);
    let status_bar_height = px(ctx, STATUS_BAR_HEIGHT_PX);
    let overlay_margin = px(ctx, OVERLAY_MARGIN_PX);
    let left_panel_width = px(ctx, LEFT_PANEL_WIDTH_PX);
    let toolbar_group_spacing = px(ctx, TOOLBAR_GROUP_SPACING_PX);
    let toolbar_group_height = px(ctx, TOOLBAR_GROUP_HEIGHT_PX);
    let toolbar_left_width = px(ctx, TOOLBAR_LEFT_WIDTH_PX);
    let toolbar_center_width = px(ctx, TOOLBAR_CENTER_WIDTH_PX);
    let toolbar_right_width = px(ctx, TOOLBAR_RIGHT_WIDTH_PX);
    let toolbar_shading_group_width = px(ctx, TOOLBAR_SHADING_GROUP_WIDTH_PX);
    let toolbar_debug_group_width = px(ctx, TOOLBAR_DEBUG_GROUP_WIDTH_PX);
    let toolbar_single_icon_group_width = px(ctx, TOOLBAR_SINGLE_ICON_GROUP_WIDTH_PX);
    let toolbar_double_icon_group_width = px(ctx, TOOLBAR_DOUBLE_ICON_GROUP_WIDTH_PX);
    let toolbar_mode_group_width = px(ctx, TOOLBAR_MODE_GROUP_WIDTH_PX);

    egui::TopBottomPanel::top("app_toolbar")
        .exact_height(toolbar_height)
        .frame(toolbar_frame())
        .show(ctx, |ui| {
            let bar_rect = ui.max_rect();
            ui.painter().line_segment(
                [
                    egui::pos2(bar_rect.left(), bar_rect.bottom() - 0.5),
                    egui::pos2(bar_rect.right(), bar_rect.bottom() - 0.5),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(58)),
            );
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.left() + overlay_margin,
                    bar_rect.top() + overlay_margin,
                ),
                egui::vec2(
                    (bar_rect.width() - overlay_margin * 2.0).max(0.0),
                    toolbar_group_height,
                ),
            );
            let left_rect = egui::Rect::from_min_size(
                row_rect.left_top(),
                egui::vec2(
                    toolbar_left_width.min(row_rect.width()),
                    toolbar_group_height,
                ),
            );
            let center_rect = egui::Rect::from_center_size(
                egui::pos2(
                    row_rect.center().x,
                    row_rect.top() + toolbar_group_height * 0.5,
                ),
                egui::vec2(
                    toolbar_center_width.min(row_rect.width()),
                    toolbar_group_height,
                ),
            );
            let right_rect = egui::Rect::from_min_size(
                egui::pos2(
                    row_rect.right() - toolbar_right_width.min(row_rect.width()),
                    row_rect.top(),
                ),
                egui::vec2(
                    toolbar_right_width.min(row_rect.width()),
                    toolbar_group_height,
                ),
            );

            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(left_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_height(toolbar_group_height);
                    ui.spacing_mut().item_spacing.x = toolbar_group_spacing;

                    toolbar_group_shell(ui, ctx, toolbar_shading_group_width, |ui| {
                        let solid = matches!(state.shading_mode, ShadingMode::Shaded);
                        let unlit = matches!(state.shading_mode, ShadingMode::Unlit);
                        let rendered = matches!(state.shading_mode, ShadingMode::ShadedWireframe);
                        let wire = matches!(state.shading_mode, ShadingMode::Wireframe);

                        if icon_toggle_button(ui, ctx, &ICON_SHADING_WIRE, wire, "Wire").clicked() {
                            state.shading_mode = ShadingMode::Wireframe;
                        }
                        if icon_toggle_button(ui, ctx, &ICON_SHADING_UNLIT, unlit, "Unlit")
                            .clicked()
                        {
                            state.shading_mode = ShadingMode::Unlit;
                        }
                        if icon_toggle_button(ui, ctx, &ICON_SHADING_SOLID, solid, "Solid")
                            .clicked()
                        {
                            state.shading_mode = ShadingMode::Shaded;
                        }
                        if icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_SHADING_WIRE_SHADED,
                            rendered,
                            "Wireframe over shaded",
                        )
                        .clicked()
                        {
                            state.shading_mode = ShadingMode::ShadedWireframe;
                        }
                    });

                    toolbar_group_shell(ui, ctx, toolbar_debug_group_width, |ui| {
                        let uv = icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_UV,
                            state.debug.uv_checker,
                            "UV Checker (right-click for options)",
                        );
                        if uv.clicked() {
                            state.debug.uv_checker = !state.debug.uv_checker;
                        }
                        if uv.secondary_clicked() {
                            state.open_panel(OptionPanel::UvChecker);
                        }

                        let face = icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_NORMALS_FACE,
                            state.debug.face_normals,
                            "Face Normals (right-click for options)",
                        );
                        if face.clicked() {
                            state.debug.face_normals = !state.debug.face_normals;
                        }
                        if face.secondary_clicked() {
                            state.open_panel(OptionPanel::FaceNormals);
                        }

                        let vertex = icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_NORMALS_VERTEX,
                            state.debug.vertex_normals,
                            "Vertex Normals (right-click for options)",
                        );
                        if vertex.clicked() {
                            state.debug.vertex_normals = !state.debug.vertex_normals;
                        }
                        if vertex.secondary_clicked() {
                            state.open_panel(OptionPanel::VertexNormals);
                        }
                    });
                },
            );

            ui.scope_builder(
                egui::UiBuilder::new().max_rect(center_rect).layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                ),
                |ui| {
                    ui.set_height(toolbar_group_height);
                    toolbar_group_shell(ui, ctx, toolbar_mode_group_width, |ui| {
                        segmented_mode_control(ui, ctx, &mut state.mode);
                    });
                },
            );

            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(right_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.set_height(toolbar_group_height);
                    ui.spacing_mut().item_spacing.x = toolbar_group_spacing;

                    toolbar_group_shell(ui, ctx, toolbar_double_icon_group_width, |ui| {
                        icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_AXIS_GIZMO,
                            state.show_axis_gizmo,
                            "Axis Gizmo",
                        )
                        .clicked()
                        .then(|| state.show_axis_gizmo = !state.show_axis_gizmo);

                        icon_toggle_button(ui, ctx, &ICON_GRID, state.show_grid, "Grid")
                            .clicked()
                            .then(|| state.show_grid = !state.show_grid);
                    });

                    toolbar_group_shell(ui, ctx, toolbar_single_icon_group_width, |ui| {
                        let (icon, tooltip) = match state.projection_mode {
                            ViewProjectionMode::Perspective => (
                                &ICON_VIEW_PERSPECTIVE,
                                "Perspective camera (click for orthographic)",
                            ),
                            ViewProjectionMode::Orthographic => (
                                &ICON_VIEW_ORTHO,
                                "Orthographic camera (click for perspective)",
                            ),
                        };

                        if icon_toggle_button(ui, ctx, icon, false, tooltip).clicked() {
                            state.projection_mode = match state.projection_mode {
                                ViewProjectionMode::Perspective => ViewProjectionMode::Orthographic,
                                ViewProjectionMode::Orthographic => ViewProjectionMode::Perspective,
                            };
                        }
                    });
                },
            );
        });

    if let Some(panel) = state.active_panel {
        let default_pos = egui::pos2(px(ctx, 30.0), toolbar_height + px(ctx, 18.0));
        let panel_pos = state.panel_pos.unwrap_or(default_pos);

        let area_response = egui::Area::new(egui::Id::new("option_panel"))
            // Persistent overlay chrome, not a transient popup: disable egui's
            // default fade-in. For an always-present anchored area `visible_last_frame`
            // reads false every frame, so the fade never completes and egui calls
            // `request_repaint()` forever — a busy-loop that pins the GPU. (See
            // egui area.rs:558-568.)
            .fade_in(false)
            .current_pos(panel_pos)
            // We drive position ourselves via the header drag handle; let egui's
            // own area-move stay off so a body drag can't fight our positioning
            // (that tug-of-war is what made the panel flicker).
            .movable(false)
            .show(ctx, |ui| {
                ui.set_width(left_panel_width);
                let collapsed = state.panel_collapsed;
                match panel {
                    OptionPanel::UvChecker => {
                        let uv_set_count = state.stats.uv_set_count;
                        option_panel(ui, "UV Checker", collapsed, |ui| {
                            checker_texture_row(ui, &mut state.uv_checker.texture);
                            ui.add_space(PANEL_ROW_GAP);
                            checker_tiling_row(ui, &mut state.uv_checker);
                            // The channel picker is only meaningful — and only
                            // shown — when the model carries more than one UV set.
                            if uv_set_count > 1 {
                                ui.add_space(PANEL_ROW_GAP);
                                checker_channel_row(
                                    ui,
                                    &mut state.uv_checker.uv_channel,
                                    uv_set_count,
                                );
                            }
                            ui.add_space(PANEL_ACTION_GAP);
                            if wide_reset_button(ui).clicked() {
                                state.uv_checker = UvCheckerPanelState::default();
                            }
                        })
                    }
                    OptionPanel::FaceNormals => option_panel(ui, "Face Normals", collapsed, |ui| {
                        labeled_slider(ui, "Normal Length", &mut state.face_normals.length);
                        ui.add_space(PANEL_ROW_GAP);
                        color_swatch_row(ui, "Line Color", &mut state.face_normals.color);
                        ui.add_space(PANEL_ACTION_GAP);
                        if wide_reset_button(ui).clicked() {
                            state.face_normals.length = 0.18;
                            state.face_normals.color = egui::Color32::from_rgb(255, 32, 32);
                        }
                    }),
                    OptionPanel::VertexNormals => {
                        option_panel(ui, "Vertex Normals", collapsed, |ui| {
                            labeled_slider(ui, "Normal Length", &mut state.vertex_normals.length);
                            ui.add_space(PANEL_ROW_GAP);
                            color_swatch_row(ui, "Line Color", &mut state.vertex_normals.color);
                            ui.add_space(PANEL_ACTION_GAP);
                            if wide_reset_button(ui).clicked() {
                                state.vertex_normals.length = 0.18;
                                state.vertex_normals.color = egui::Color32::from_rgb(32, 224, 232);
                            }
                        })
                    }
                }
            });
        let outcome = area_response.inner;
        let panel_size = area_response.response.rect.size();

        if outcome.toggle_collapse {
            state.panel_collapsed = !state.panel_collapsed;
        }
        if outcome.close {
            state.active_panel = None;
        }

        // Keep the panel inside the viewport: never let it slide under the top
        // toolbar or the bottom status bar (and not off the left/right edges).
        let screen = ctx.screen_rect();
        let desired = panel_pos + outcome.drag_delta;
        let min_x = screen.left();
        let min_y = screen.top() + toolbar_height;
        let max_x = (screen.right() - panel_size.x).max(min_x);
        let max_y = (screen.bottom() - status_bar_height - panel_size.y).max(min_y);
        state.panel_pos = Some(egui::pos2(
            desired.x.clamp(min_x, max_x),
            desired.y.clamp(min_y, max_y),
        ));
    }

    if state.show_axis_gizmo {
        let gizmo_response = egui::Area::new(egui::Id::new("axis_gizmo"))
            .fade_in(false)
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(
                    -px(ctx, GIZMO_INSET_PX),
                    toolbar_height + px(ctx, GIZMO_INSET_PX),
                ),
            )
            .show(ctx, |ui| {
                draw_axis_gizmo(ui, ctx, camera, state.projection_mode.into())
            });
        output.axis_gizmo_action = gizmo_response.inner;
    }

    if state.show_stats {
        egui::Area::new(egui::Id::new("stats_overlay"))
            .fade_in(false)
            .anchor(
                egui::Align2::LEFT_BOTTOM,
                egui::vec2(
                    px(ctx, STATS_OVERLAY_MARGIN_PX),
                    -(status_bar_height + px(ctx, STATS_OVERLAY_MARGIN_PX)),
                ),
            )
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgba_premultiplied(20, 22, 25, 130))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(52)))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(
                        STATS_PANEL_PAD_X_PX,
                        STATS_PANEL_PAD_Y_PX,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(STATS_PANEL_WIDTH_PX);
                        stats_grid(ui, state);
                    });
            });
    }

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(status_bar_height)
        .frame(status_bar_frame())
        .show(ctx, |ui| {
            // Inset the group equally on all sides: the vertical gap is fixed by
            // centering the group in the bar, so use that same gap on the left
            // edge to keep the button box equidistant from every edge.
            let bar_rect = ui.max_rect();
            let edge_inset = (status_bar_height - toolbar_group_height) * 0.5;
            let group_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.left() + edge_inset,
                    bar_rect.center().y - toolbar_group_height * 0.5,
                ),
                egui::vec2(toolbar_single_icon_group_width, toolbar_group_height),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(group_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.set_height(toolbar_group_height);
                    toolbar_group_shell(ui, ctx, toolbar_single_icon_group_width, |ui| {
                        if icon_toggle_button(ui, ctx, &ICON_INFO, state.show_stats, "Model Stats")
                            .clicked()
                        {
                            state.show_stats = !state.show_stats;
                        }
                    });
                },
            );
        });

    output
}

fn apply_visuals(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.window_fill = egui::Color32::from_rgb(33, 33, 33);
    style.visuals.panel_fill = egui::Color32::from_rgb(33, 33, 33);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(16, 17, 19);
    style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(36, 36, 36);
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(18, 19, 22);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(47, 79, 131);
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(64, 105, 171);
    style.visuals.widgets.open.bg_fill = egui::Color32::from_rgb(41, 43, 48);
    style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(216, 219, 224);
    style.visuals.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(68, 107, 177);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(88, 135, 217));
    style.visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(58));
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 8.0);
    ctx.set_style(style);
}

fn sync_debug_state(state: &mut UiState) {
    state.debug.shading_mode = state.shading_mode;
    state.debug.show_grid = state.show_grid;
    state.debug.uv_checker_texture = state.uv_checker.texture;
    state.debug.uv_checker_tiling = state.uv_checker.tiling;
    state.debug.uv_channel = state.uv_checker.uv_channel;
    state.debug.face_normal_length = state.face_normals.length;
    state.debug.vertex_normal_length = state.vertex_normals.length;
    state.debug.face_normal_color = color32_to_rgba(state.face_normals.color);
    state.debug.vertex_normal_color = color32_to_rgba(state.vertex_normals.color);
}

fn toolbar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(40, 39, 38))
        .stroke(egui::Stroke::NONE)
        .inner_margin(egui::Margin::same(0))
}

fn status_bar_frame() -> egui::Frame {
    // Zero inner margin, like `toolbar_frame`: content is placed by px-converted
    // rect math in `draw_overlay`, not by frame padding, so no raw pixel literals
    // leak in here.
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(40, 39, 38))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(72)))
        .inner_margin(egui::Margin::same(0))
}

fn toolbar_group_shell(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let group_height = px(ctx, TOOLBAR_GROUP_HEIGHT_PX);
    let desired = egui::vec2(width, group_height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
    ui.painter().rect(
        rect,
        px(ctx, 6.0),
        egui::Color32::from_rgb(18, 19, 22),
        egui::Stroke::NONE,
        egui::StrokeKind::Outside,
    );
    let inner_padding = px(ctx, TOOLBAR_GROUP_PADDING_PX);
    let inner = rect.shrink2(egui::vec2(inner_padding, inner_padding));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = px(ctx, TOOLBAR_ICON_GAP_PX);
            add_contents(ui);
        },
    );
    response
}

fn segmented_mode_control(ui: &mut egui::Ui, ctx: &egui::Context, mode: &mut WorkspaceMode) {
    mode_segment(ui, ctx, mode, WorkspaceMode::ThreeD, "3D");
    mode_segment(ui, ctx, mode, WorkspaceMode::Uv, "UV");
    mode_segment(ui, ctx, mode, WorkspaceMode::Texture, "Tex");
}

fn mode_segment(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    mode: &mut WorkspaceMode,
    value: WorkspaceMode,
    label: &str,
) {
    let selected = *mode == value;
    let desired = egui::vec2(px(ctx, 56.0), px(ctx, 42.0));
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let fill = if selected {
        egui::Color32::from_rgb(66, 103, 163)
    } else if response.hovered() {
        egui::Color32::from_rgb(54, 56, 61)
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = if selected {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(198, 207, 218)
    };

    ui.painter().rect(
        rect,
        px(ctx, 6.0),
        fill,
        egui::Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(px(ctx, 18.0)),
        text_color,
    );

    if response.clicked() {
        *mode = value;
    }
}

fn icon_toggle_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon: &AppIcon,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    icon_tile_button(
        ui,
        ctx,
        icon,
        selected,
        tooltip,
        egui::vec2(px(ctx, TOOLBAR_ICON_SIZE_PX), px(ctx, TOOLBAR_ICON_SIZE_PX)),
    )
}

fn icon_tile_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon: &AppIcon,
    selected: bool,
    tooltip: &str,
    size: egui::Vec2,
) -> egui::Response {
    let tint = if selected {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(230)
    };
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = if selected {
        (
            egui::Color32::from_rgb(66, 103, 163),
            egui::Color32::TRANSPARENT,
        )
    } else if response.hovered() {
        (
            egui::Color32::from_rgb(54, 56, 61),
            egui::Color32::TRANSPARENT,
        )
    } else {
        (
            egui::Color32::from_rgb(18, 19, 22),
            egui::Color32::TRANSPARENT,
        )
    };

    ui.painter().rect(
        rect,
        px(ctx, 6.0),
        visuals.0,
        egui::Stroke::new(1.0, visuals.1),
        egui::StrokeKind::Inside,
    );

    if let Some(texture) = load_icon_texture(ui, icon) {
        let image_padding = px(ctx, TOOLBAR_ICON_PADDING_PX);
        let image_rect = rect.shrink2(egui::vec2(image_padding, image_padding));
        egui::Image::from_texture(texture)
            .fit_to_exact_size(image_rect.size())
            .tint(tint)
            .paint_at(ui, image_rect);
    }

    response.on_hover_text(tooltip)
}

/// Result of drawing an [`option_panel`]: how far its header was dragged this
/// frame, plus the one-shot header interactions (collapse toggle / close).
struct PanelOutcome {
    drag_delta: egui::Vec2,
    toggle_collapse: bool,
    close: bool,
}

// ──────────────────────────────────────────────────────────────────────────
// Option-panel layout tokens. Tweak padding, sizes, and font sizes here — every
// option panel (Face Normals / Vertex Normals / UV Checker) reads these, so a
// change in this block re-skins all of them. Values are raw pixels (the panel
// is intentionally not DPI-scaled, matching the rest of the panel chrome).
// ──────────────────────────────────────────────────────────────────────────

/// Compact header bar height. Kept short so the title reads as a label, not a
/// title bar (see the reference layout).
const PANEL_HEADER_HEIGHT: f32 = 34.0;
/// Horizontal inset of the title text from the panel's left edge.
const PANEL_HEADER_PAD_X: f32 = 16.0;
/// Outer corner radius of the panel. The header rounds its top corners and the
/// body rounds its bottom corners so the two stacked elements read as one card.
const PANEL_CORNER_RADIUS: u8 = 6;
/// Padding around the body content. Kept tight: this is a diagnostic tool,
/// density matters more than air.
const PANEL_CONTENT_MARGIN: i8 = 14;
/// Padding between the header divider and the body's first control row.
const PANEL_BODY_TOP_MARGIN: i8 = 10;
/// Fixed width of the label column in the two-column control table. Wide
/// enough for the longest label ("Normal Length") so the control column always
/// starts at the same x, giving the rows a tabular alignment.
const PANEL_LABEL_COL_W: f32 = 116.0;
/// Gap between the label column and the control column.
const PANEL_COL_GAP: f32 = 12.0;
/// Height of a control row (label + control share this; both vertically
/// centered within it).
const PANEL_ROW_H: f32 = 22.0;
/// Vertical gap between stacked control rows (slider → swatches).
const PANEL_ROW_GAP: f32 = 6.0;
/// Vertical gap before the full-width action button (Reset all).
const PANEL_ACTION_GAP: f32 = 10.0;
/// Height of the full-width action button.
const PANEL_BUTTON_HEIGHT: f32 = 32.0;
/// Edge of each square color swatch, and the gap between swatches.
const PANEL_SWATCH_SIZE: f32 = 24.0;
const PANEL_SWATCH_GAP: f32 = 6.0;
/// Width of the numeric text field paired with the checker-tiling slider.
const PANEL_TILING_TEXT_W: f32 = 48.0;
/// Vertical padding inside the compact dropdown button. Trimmed (vs. egui's
/// default) so the closed combo is the same height as the slider / text-field
/// rows — every label+control row in the panel then reads as one uniform band.
const PANEL_COMBO_BUTTON_PAD_Y: f32 = 2.0;
/// Max height of an opened dropdown popup before it starts to scroll.
const PANEL_COMBO_POPUP_MAX_H: f32 = 240.0;
/// Per-option row height, inter-option gap, and per-option vertical padding in
/// an opened dropdown. Kept small so a short option list is compact.
const PANEL_COMBO_OPTION_H: f32 = 20.0;
const PANEL_COMBO_OPTION_GAP: f32 = 2.0;
const PANEL_COMBO_OPTION_PAD_Y: f32 = 2.0;
/// Dropdown option text colors. The active option reads from a brighter text
/// color rather than a low-contrast highlight fill; the rest are dimmed.
const PANEL_COMBO_TEXT_SELECTED: egui::Color32 = egui::Color32::from_gray(238);
const PANEL_COMBO_TEXT_DIM: egui::Color32 = egui::Color32::from_gray(150);

/// Inclusive checker-tiling range and the value a fresh / reset panel uses.
const CHECKER_TILING_MIN: u32 = 1;
const CHECKER_TILING_MAX: u32 = 16;
const DEFAULT_CHECKER_TILING: u32 = 4;

/// Title text size in the header bar.
const PANEL_TITLE_FONT_SIZE: f32 = 14.5;
/// Row-label text size in the control table's left column.
const PANEL_LABEL_FONT_SIZE: f32 = 14.0;
/// Action-button label text size.
const PANEL_BUTTON_FONT_SIZE: f32 = 14.0;
/// Side length of the close glyph painted in the header's right-edge button.
/// Kept close to the hit area so the icon reads as a real button, not a speck.
const PANEL_CLOSE_ICON_SIZE: f32 = 20.0;

fn option_panel(
    ui: &mut egui::Ui,
    title: &str,
    collapsed: bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> PanelOutcome {
    let mut outcome = PanelOutcome {
        drag_delta: egui::Vec2::ZERO,
        toggle_collapse: false,
        close: false,
    };

    let panel_fill = egui::Color32::from_rgb(39, 39, 39);
    let panel_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(66));

    // The panel is two stacked elements: an always-visible header bar and a
    // controls body shown only when expanded. They sit flush (no inter-frame
    // spacing) and share fill/stroke, so their touching borders read as a single
    // full-width divider and the pair looks like one rounded card.
    ui.spacing_mut().item_spacing.y = 0.0;

    // ── Header bar ──────────────────────────────────────────────────────────
    // Rounds all four corners when collapsed (a standalone pill); only the top
    // corners when expanded so it meets the body squarely.
    let header_corner = if collapsed {
        egui::CornerRadius::same(PANEL_CORNER_RADIUS)
    } else {
        egui::CornerRadius {
            nw: PANEL_CORNER_RADIUS,
            ne: PANEL_CORNER_RADIUS,
            sw: 0,
            se: 0,
        }
    };
    egui::Frame::NONE
        .fill(panel_fill)
        .stroke(panel_stroke)
        .corner_radius(header_corner)
        .show(ui, |ui| {
            // Allocate the full-width bar so the frame actually paints its
            // background. (An interact-only header claims zero width, which is
            // why the bar went transparent when collapsed.) The bar is the
            // interaction handle: a single click toggles collapse, a drag moves.
            let (header_rect, header_response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), PANEL_HEADER_HEIGHT),
                egui::Sense::click_and_drag(),
            );
            if header_response.dragged() {
                outcome.drag_delta = header_response.drag_delta();
            }
            if header_response.clicked() {
                outcome.toggle_collapse = true;
            }
            if header_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }

            // Title: left-aligned and vertically centered in the compact bar.
            ui.painter().text(
                egui::pos2(
                    header_rect.left() + PANEL_HEADER_PAD_X,
                    header_rect.center().y,
                ),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(PANEL_TITLE_FONT_SIZE),
                egui::Color32::from_gray(224),
            );

            // Close button, right-aligned in the bar (a square hit area the full
            // height of the header), painted on top of the header handle. Its
            // own Sense::click() wins over the header's, so clicking the X closes
            // rather than toggling collapse.
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(header_rect.right() - PANEL_HEADER_HEIGHT, header_rect.top()),
                egui::vec2(PANEL_HEADER_HEIGHT, PANEL_HEADER_HEIGHT),
            );
            let close_response = ui.interact(
                close_rect,
                ui.make_persistent_id(("option_panel_close", title)),
                egui::Sense::click(),
            );
            if close_response.hovered() {
                ui.painter()
                    .rect_filled(close_rect.shrink(4.0), 5.0, egui::Color32::from_gray(58));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some(texture) = load_icon_texture(ui, &ICON_CLOSE) {
                let icon_rect = egui::Rect::from_center_size(
                    close_rect.center(),
                    egui::vec2(PANEL_CLOSE_ICON_SIZE, PANEL_CLOSE_ICON_SIZE),
                );
                let tint = if close_response.hovered() {
                    egui::Color32::from_gray(235)
                } else {
                    egui::Color32::from_gray(176)
                };
                ui.painter().image(
                    texture.id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            }
            if close_response.clicked() {
                outcome.close = true;
            }
        });

    // ── Controls body ──────────────────────────────────────────────────────
    // A separate element below the header, hidden entirely when collapsed. The
    // divider between the two is their shared (full-width) border.
    if !collapsed {
        egui::Frame::NONE
            .fill(panel_fill)
            .stroke(panel_stroke)
            .corner_radius(egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: PANEL_CORNER_RADIUS,
                se: PANEL_CORNER_RADIUS,
            })
            .inner_margin(egui::Margin {
                left: PANEL_CONTENT_MARGIN,
                right: PANEL_CONTENT_MARGIN,
                top: PANEL_BODY_TOP_MARGIN,
                bottom: PANEL_CONTENT_MARGIN,
            })
            .show(ui, |ui| {
                // Rows space themselves via add_space; kill the implicit gap.
                ui.spacing_mut().item_spacing.y = 0.0;
                add_contents(ui);
            });
    }

    outcome
}

/// Paint a fixed-width label cell (left column of the control table) and return
/// the width remaining for the control cell. Advances the cursor past the label
/// column + gap so the caller can drop the control straight after it.
fn table_label_cell(ui: &mut egui::Ui, label: &str) -> f32 {
    let control_w = (ui.available_width() - PANEL_LABEL_COL_W - PANEL_COL_GAP).max(0.0);
    let (label_rect, _) = ui.allocate_exact_size(
        egui::vec2(PANEL_LABEL_COL_W, PANEL_ROW_H),
        egui::Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(label_rect.left(), label_rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(PANEL_LABEL_FONT_SIZE),
        egui::Color32::from_gray(180),
    );
    ui.add_space(PANEL_COL_GAP);
    control_w
}

/// A dropdown sized to one control row: the closed button matches `PANEL_ROW_H`
/// (so combo rows are the same height as slider / text-field rows), and the
/// opened popup uses short option rows with the active option distinguished by a
/// brighter text color instead of a low-contrast highlight fill.
fn compact_combo(
    ui: &mut egui::Ui,
    id_salt: &str,
    control_w: f32,
    selected_text: impl Into<egui::WidgetText>,
    contents: impl FnOnce(&mut egui::Ui),
) {
    // Shrink the closed button to row height.
    ui.spacing_mut().interact_size.y = PANEL_ROW_H;
    ui.spacing_mut().button_padding.y = PANEL_COMBO_BUTTON_PAD_Y;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(control_w)
        .height(PANEL_COMBO_POPUP_MAX_H)
        .show_ui(ui, |ui| {
            // Compact the option rows.
            ui.spacing_mut().item_spacing.y = PANEL_COMBO_OPTION_GAP;
            ui.spacing_mut().interact_size.y = PANEL_COMBO_OPTION_H;
            ui.spacing_mut().button_padding.y = PANEL_COMBO_OPTION_PAD_Y;
            // Drop the blue selection fill (zero-width stroke paints no border)
            // and carry the brighter color through as the selected text color;
            // dim the unselected options so the active one stands out by
            // contrast alone.
            ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().selection.stroke = egui::Stroke::new(0.0, PANEL_COMBO_TEXT_SELECTED);
            ui.visuals_mut().widgets.inactive.fg_stroke.color = PANEL_COMBO_TEXT_DIM;
            contents(ui);
        });
}

/// "Checker Texture" row: a dropdown choosing which built-in checker the UV
/// view samples.
fn checker_texture_row(ui: &mut egui::Ui, texture: &mut CheckerTexture) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Checker Texture");
        compact_combo(
            ui,
            "uv_checker_texture",
            control_w,
            checker_texture_label(*texture),
            |ui| {
                ui.selectable_value(texture, CheckerTexture::Greyscale, "Greyscale");
                ui.selectable_value(texture, CheckerTexture::Color, "Color");
            },
        );
    });
}

/// "Checker Tiling" row: an integer slider (1..=16) paired with a text field.
/// The slider updates the text mirror live; the text field is re-sanitized to a
/// clamped integer when editing finishes (focus loss or Enter).
fn checker_tiling_row(ui: &mut egui::Ui, state: &mut UvCheckerPanelState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Checker Tiling");
        let slider_w = (control_w - PANEL_TILING_TEXT_W - PANEL_COL_GAP).max(0.0);

        ui.spacing_mut().slider_width = slider_w;
        let slider = ui.add(
            egui::Slider::new(&mut state.tiling, CHECKER_TILING_MIN..=CHECKER_TILING_MAX)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        if slider.changed() {
            state.tiling_text = state.tiling.to_string();
        }

        ui.add_space(PANEL_COL_GAP);
        let field = ui.add_sized(
            egui::vec2(PANEL_TILING_TEXT_W, PANEL_ROW_H),
            egui::TextEdit::singleline(&mut state.tiling_text)
                .horizontal_align(egui::Align::Center),
        );
        // Commit only when editing ends: an empty / non-numeric entry reverts to
        // the current value, anything else is clamped into range.
        if field.lost_focus() {
            let committed = state
                .tiling_text
                .trim()
                .parse::<u32>()
                .unwrap_or(state.tiling)
                .clamp(CHECKER_TILING_MIN, CHECKER_TILING_MAX);
            state.tiling = committed;
            state.tiling_text = committed.to_string();
        }
    });
}

/// "Model UV channel" row: a dropdown selecting which of the model's UV sets the
/// checker view visualizes. Only shown for models with more than one set.
fn checker_channel_row(ui: &mut egui::Ui, uv_channel: &mut u32, uv_set_count: usize) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Model UV channel");
        compact_combo(
            ui,
            "uv_checker_channel",
            control_w,
            format!("Channel {uv_channel}"),
            |ui| {
                for channel in 0..uv_set_count as u32 {
                    ui.selectable_value(uv_channel, channel, format!("Channel {channel}"));
                }
            },
        );
    });
}

fn checker_texture_label(texture: CheckerTexture) -> &'static str {
    match texture {
        CheckerTexture::Greyscale => "Greyscale",
        CheckerTexture::Color => "Color",
    }
}

fn labeled_slider(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.horizontal(|ui| {
        // Zero egui's implicit inter-item gap so our explicit column gap is the
        // only horizontal spacing.
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, label);
        // Fill the control column exactly; egui keeps the handle inside the rail
        // rect, so the rail spans to the column's right edge without spilling.
        ui.spacing_mut().slider_width = control_w;
        ui.add(
            egui::Slider::new(value, 0.02..=1.0)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
    });
}

fn color_swatch_row(ui: &mut egui::Ui, label: &str, selected: &mut egui::Color32) {
    const COLORS: [egui::Color32; 5] = [
        egui::Color32::WHITE,
        egui::Color32::BLACK,
        egui::Color32::from_rgb(32, 224, 232),
        egui::Color32::from_rgb(29, 255, 27),
        egui::Color32::from_rgb(255, 27, 27),
    ];

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, label);
        // Compact thumbnails with tight, even gaps, left-aligned in the column;
        // capped so they stay small rather than filling the whole column.
        let gaps = PANEL_SWATCH_GAP * (COLORS.len() as f32 - 1.0);
        let swatch = ((control_w - gaps) / COLORS.len() as f32).clamp(0.0, PANEL_SWATCH_SIZE);
        for (i, color) in COLORS.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(swatch, swatch), egui::Sense::click());
            if response.clicked() {
                *selected = *color;
            }
            let stroke = if selected == color {
                egui::Stroke::new(2.0, egui::Color32::from_rgb(88, 135, 217))
            } else {
                egui::Stroke::new(1.0, egui::Color32::from_gray(28))
            };
            ui.painter()
                .rect(rect, 5.0, *color, stroke, egui::StrokeKind::Outside);
            if i + 1 < COLORS.len() {
                ui.add_space(PANEL_SWATCH_GAP);
            }
        }
    });
}

fn wide_reset_button(ui: &mut egui::Ui) -> egui::Response {
    // Full-width button with the label painted dead-center. egui's `Button`
    // left-aligns text within an over-wide rect (it honors the parent layout's
    // horizontal align), so paint the label ourselves to keep it centered.
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, PANEL_BUTTON_HEIGHT), egui::Sense::click());
    let fill = if response.hovered() {
        egui::Color32::from_rgb(28, 32, 37)
    } else {
        egui::Color32::from_rgb(20, 23, 26)
    };
    ui.painter().rect(
        rect,
        6.0,
        fill,
        egui::Stroke::new(1.0, egui::Color32::from_gray(62)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Reset all",
        egui::FontId::proportional(PANEL_BUTTON_FONT_SIZE),
        egui::Color32::from_gray(214),
    );
    response
}

fn load_icon_texture(ui: &mut egui::Ui, icon: &AppIcon) -> Option<egui::load::SizedTexture> {
    let texture_id = egui::Id::new(("toolbar_icon_texture", icon.id));
    if let Some(handle) = ui.data(|data| data.get_temp::<egui::TextureHandle>(texture_id)) {
        return Some(egui::load::SizedTexture::from_handle(&handle));
    }

    let color_image = decode_icon_color_image(icon).ok()?;
    let handle = ui
        .ctx()
        .load_texture(icon.id, color_image, egui::TextureOptions::LINEAR);
    let texture = egui::load::SizedTexture::from_handle(&handle);
    ui.data_mut(|data| data.insert_temp(texture_id, handle));
    Some(texture)
}

fn decode_icon_color_image(icon: &AppIcon) -> Result<egui::ColorImage, ImageError> {
    let decoded = image::load_from_memory(icon.png_bytes)?.into_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let rgba = decoded.into_raw();
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
}

fn stats_grid(ui: &mut egui::Ui, state: &UiState) {
    let stats = &state.stats;
    ui.spacing_mut().item_spacing.y = STATS_ROW_SPACING_PX;
    stat_row(ui, "Draws", &stats.draw_count.to_string());
    stat_row(ui, "Polys", &stats.polygon_count.to_string());
    stat_row(ui, "Tris", &stats.triangle_count.to_string());
    stat_row(ui, "Verts", &stats.vertex_count.to_string());
    stat_row(ui, "UV Sets", &stats.uv_set_count.to_string());
    stat_row(ui, "FPS", &format!("{:.0}", state.fps));
}

/// One stats row: label hugs the left edge, value right-aligns against the
/// panel's right edge so the numeric column reads as a tidy block.
fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(mono_label(
            label,
            STATS_FONT_SIZE_PX,
            egui::Color32::from_gray(178),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(mono_label(
                value,
                STATS_FONT_SIZE_PX,
                egui::Color32::from_gray(232),
            ));
        });
    });
}

fn draw_axis_gizmo(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    camera: OrbitCamera,
    projection: CameraProjection,
) -> Option<AxisGizmoAction> {
    let size = px(ctx, GIZMO_SIZE_PX);
    let reach = px(ctx, GIZMO_REACH_PX);
    let ball_radius = px(ctx, GIZMO_BALL_RADIUS_PX);
    let (rect, panel_response) =
        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
    let painter = ui.painter();
    let center = rect.center();
    let mut action = panel_response
        .dragged()
        .then(|| panel_response.drag_motion())
        .filter(|delta| delta.length_sq() > 0.0)
        .map(|delta| AxisGizmoAction::Orbit(Vec2::new(delta.x, delta.y) * ctx.pixels_per_point()));

    if panel_response.hovered() || panel_response.dragged() {
        painter.rect_filled(
            rect,
            px(ctx, 12.0),
            egui::Color32::from_rgba_premultiplied(8, 12, 16, 90),
        );
    }

    let mut points = axis_gizmo_points(camera, projection, center, reach);
    points.sort_by(|a, b| a.depth.total_cmp(&b.depth));

    for point in points.iter().filter(|point| point.positive) {
        let alpha = ((0.4 + 0.6 * ((point.depth + 1.0) * 0.5)).clamp(0.0, 1.0) * 255.0) as u8;
        painter.line_segment(
            [center, point.position],
            egui::Stroke::new(
                px(ctx, 2.5),
                point.color.linear_multiply(alpha as f32 / 255.0),
            ),
        );
    }

    for point in points {
        let ball_rect = egui::Rect::from_center_size(
            point.position,
            egui::vec2(ball_radius * 2.0, ball_radius * 2.0),
        );
        let response = ui
            .interact(
                ball_rect.expand(px(ctx, 4.0)),
                ui.make_persistent_id(("axis_gizmo_ball", point.axis)),
                egui::Sense::click(),
            )
            .on_hover_text(point.tooltip);
        let radius = if response.hovered() {
            ball_radius * 1.18
        } else {
            ball_radius
        };

        if point.positive {
            painter.circle_filled(point.position, radius, point.color);
            bold_text(
                painter,
                ctx,
                point.position,
                point.label,
                px(ctx, GIZMO_LABEL_SIZE_PX),
                egui::Color32::from_rgb(16, 21, 27),
            );
        } else if projection == CameraProjection::Orthographic
            && point.depth > GIZMO_AXIS_ALIGNED_DEPTH
        {
            // The axis we're snapped down points straight at the viewer: render
            // it solid like a positive axis so its label stays readable and the
            // current view stays identified even without hovering.
            painter.circle_filled(point.position, radius, point.color);
            bold_text(
                painter,
                ctx,
                point.position,
                point.label,
                px(ctx, GIZMO_LABEL_NEG_SIZE_PX),
                egui::Color32::from_rgb(16, 21, 27),
            );
        } else if response.hovered() {
            // Hovered: crisp full-color outline plus the axis label.
            painter.circle_stroke(
                point.position,
                radius,
                egui::Stroke::new(px(ctx, 2.0), point.color),
            );
            bold_text(
                painter,
                ctx,
                point.position,
                point.label,
                px(ctx, GIZMO_LABEL_NEG_SIZE_PX),
                point.color,
            );
        } else {
            // Idle: a translucent filled disc. egui's antialiased closed-path
            // strokes over-render thin rings, so a faded `circle_stroke` reads
            // near-opaque; a `circle_filled` honours the alpha and clearly looks
            // translucent.
            painter.circle_filled(
                point.position,
                radius,
                with_opacity(point.color, GIZMO_NEG_OPACITY),
            );
        }

        if response.clicked() {
            action = Some(AxisGizmoAction::Snap(point.axis));
        }
    }

    action
}

#[derive(Debug, Clone, Copy)]
struct AxisGizmoPoint {
    axis: ViewAxis,
    position: egui::Pos2,
    depth: f32,
    color: egui::Color32,
    label: &'static str,
    tooltip: &'static str,
    positive: bool,
}

fn axis_gizmo_points(
    camera: OrbitCamera,
    projection: CameraProjection,
    center: egui::Pos2,
    reach: f32,
) -> Vec<AxisGizmoPoint> {
    const AXES: [(ViewAxis, Vec3, egui::Color32, &str, &str, bool); 6] = [
        (
            ViewAxis::PositiveX,
            Vec3::X,
            egui::Color32::from_rgb(242, 81, 93),
            "X",
            "View +X",
            true,
        ),
        (
            ViewAxis::NegativeX,
            Vec3::NEG_X,
            egui::Color32::from_rgb(242, 81, 93),
            "-X",
            "View -X",
            false,
        ),
        (
            ViewAxis::PositiveY,
            Vec3::Y,
            egui::Color32::from_rgb(78, 238, 57),
            "Y",
            "View +Y",
            true,
        ),
        (
            ViewAxis::NegativeY,
            Vec3::NEG_Y,
            egui::Color32::from_rgb(78, 238, 57),
            "-Y",
            "View -Y",
            false,
        ),
        (
            ViewAxis::PositiveZ,
            Vec3::Z,
            egui::Color32::from_rgb(63, 166, 239),
            "Z",
            "View +Z",
            true,
        ),
        (
            ViewAxis::NegativeZ,
            Vec3::NEG_Z,
            egui::Color32::from_rgb(63, 166, 239),
            "-Z",
            "View -Z",
            false,
        ),
    ];

    // Distance from a virtual eye to the gizmo's center, derived from the
    // viewport camera's vertical FOV (clamped to keep the foreshortening stable
    // and the denominator strictly positive). `None` in orthographic mode, where
    // the eye is effectively at infinity and the projection stays flat.
    let perspective_eye = match projection {
        CameraProjection::Orthographic => None,
        CameraProjection::Perspective => {
            Some(reach / (camera.fov_y_radians * 0.5).clamp(0.1, 0.6).tan())
        }
    };

    AXES.into_iter()
        .map(|(axis, world, color, label, tooltip, positive)| {
            let view = camera.view_space_direction(world);
            // +Z in view space points toward the viewer: tips facing the camera
            // are magnified, tips behind the center shrink.
            let scale = perspective_eye
                .map(|eye| eye / (eye - view.z * reach))
                .unwrap_or(1.0);
            AxisGizmoPoint {
                axis,
                position: egui::pos2(
                    center.x + view.x * reach * scale,
                    center.y - view.y * reach * scale,
                ),
                depth: view.z,
                color,
                label,
                tooltip,
                positive,
            }
        })
        .collect()
}

/// Monospace label using the bundled JetBrains Mono face. Used by the stats
/// overlay so numeric columns align on a fixed grid.
fn mono_label(text: &str, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text)
        .size(size)
        .color(color)
        .family(egui::FontFamily::Monospace)
}

/// Return `color` with its alpha set to `opacity` (0..=1) of fully opaque,
/// regardless of the input alpha. Used to fade gizmo rings without dimming hue.
fn with_opacity(color: egui::Color32, opacity: f32) -> egui::Color32 {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Draw centered text with a faux-bold weight. Only the regular instance of the
/// variable UI font is registered with egui, so there is no true bold face to
/// select — the heavier stroke is approximated by layering the glyphs with
/// small sub-pixel offsets before the crisp center pass.
fn bold_text(
    painter: &egui::Painter,
    ctx: &egui::Context,
    pos: egui::Pos2,
    text: &str,
    size: f32,
    color: egui::Color32,
) {
    let font = egui::FontId::proportional(size);
    let offset = px(ctx, 0.6);
    for delta in [
        egui::vec2(-offset, 0.0),
        egui::vec2(offset, 0.0),
        egui::vec2(0.0, -offset),
        egui::vec2(0.0, offset),
    ] {
        painter.text(
            pos + delta,
            egui::Align2::CENTER_CENTER,
            text,
            font.clone(),
            color,
        );
    }
    painter.text(pos, egui::Align2::CENTER_CENTER, text, font, color);
}

fn px(ctx: &egui::Context, value: f32) -> f32 {
    value / ctx.pixels_per_point()
}

fn color32_to_rgba(color: egui::Color32) -> [f32; 4] {
    [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ]
}
