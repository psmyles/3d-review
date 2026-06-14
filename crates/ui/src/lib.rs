use std::sync::Arc;

use glam::{Vec2, Vec3};
use image::ImageError;
use review_model::{ModelData, ModelStats};
use review_render::{CameraProjection, OrbitCamera, SceneCallback, SceneDebugOptions, ShadingMode};

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
const GIZMO_SIZE_PX: f32 = 156.0;
const GIZMO_INSET_PX: f32 = 26.0;
const GIZMO_REACH_PX: f32 = 52.0;
const GIZMO_BALL_RADIUS_PX: f32 = 13.0;

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
    pub expanded: bool,
    pub length: f32,
    pub color: egui::Color32,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub mode: WorkspaceMode,
    pub debug: SceneDebugOptions,
    pub shading_mode: ShadingMode,
    pub projection_mode: ViewProjectionMode,
    pub show_grid: bool,
    pub show_axis_gizmo: bool,
    pub uv_checker_panel_expanded: bool,
    pub face_normals: NormalPanelState,
    pub vertex_normals: NormalPanelState,
    pub status: String,
    pub stats: ModelStats,
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
            uv_checker_panel_expanded: false,
            face_normals: NormalPanelState {
                expanded: true,
                length: 0.18,
                color: egui::Color32::from_rgb(255, 32, 32),
            },
            vertex_normals: NormalPanelState {
                expanded: true,
                length: 0.18,
                color: egui::Color32::from_rgb(32, 224, 232),
            },
            status: "Loaded scene".to_owned(),
            stats: ModelStats::default(),
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

    let viewport_rect = ctx.input(|input| input.screen_rect());

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
                        icon_toggle_button(ui, ctx, &ICON_UV, state.debug.uv_checker, "UV Checker")
                            .clicked()
                            .then(|| {
                                state.debug.uv_checker = !state.debug.uv_checker;
                                state.uv_checker_panel_expanded = state.debug.uv_checker;
                            });

                        if icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_NORMALS_FACE,
                            state.debug.face_normals,
                            "Face Normals",
                        )
                        .clicked()
                        {
                            state.debug.face_normals = !state.debug.face_normals;
                            state.face_normals.expanded = state.debug.face_normals;
                        }

                        if icon_toggle_button(
                            ui,
                            ctx,
                            &ICON_NORMALS_VERTEX,
                            state.debug.vertex_normals,
                            "Vertex Normals",
                        )
                        .clicked()
                        {
                            state.debug.vertex_normals = !state.debug.vertex_normals;
                            state.vertex_normals.expanded = state.debug.vertex_normals;
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

    egui::Area::new(egui::Id::new("left_options"))
        .fixed_pos(egui::pos2(px(ctx, 30.0), toolbar_height + px(ctx, 18.0)))
        .show(ctx, |ui| {
            ui.set_width(left_panel_width);
            if state.debug.uv_checker {
                option_panel(
                    ui,
                    "UV Checker Options",
                    &mut state.uv_checker_panel_expanded,
                    |ui| {
                        ui.add_enabled_ui(false, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(rich_label(
                                    "Checker Texture",
                                    15.0,
                                    egui::Color32::from_gray(160),
                                ));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(rich_label(
                                            "Soon",
                                            14.0,
                                            egui::Color32::from_gray(120),
                                        ));
                                    },
                                );
                            });
                        });
                    },
                );
            }

            if state.debug.face_normals {
                if state.debug.uv_checker {
                    ui.add_space(18.0);
                }
                option_panel(
                    ui,
                    "Face Normals Options",
                    &mut state.face_normals.expanded,
                    |ui| {
                        labeled_slider(ui, "Normal Length", &mut state.face_normals.length);
                        ui.add_space(10.0);
                        color_swatch_row(ui, "Line Color", &mut state.face_normals.color);
                        ui.add_space(14.0);
                        if wide_reset_button(ui).clicked() {
                            state.face_normals.length = 0.18;
                            state.face_normals.color = egui::Color32::from_rgb(255, 32, 32);
                        }
                    },
                );
            }

            if state.debug.vertex_normals {
                if state.debug.uv_checker || state.debug.face_normals {
                    ui.add_space(18.0);
                }
                option_panel(
                    ui,
                    "Vertex Normals Options",
                    &mut state.vertex_normals.expanded,
                    |ui| {
                        labeled_slider(ui, "Normal Length", &mut state.vertex_normals.length);
                        ui.add_space(10.0);
                        color_swatch_row(ui, "Line Color", &mut state.vertex_normals.color);
                        ui.add_space(14.0);
                        if wide_reset_button(ui).clicked() {
                            state.vertex_normals.length = 0.18;
                            state.vertex_normals.color = egui::Color32::from_rgb(32, 224, 232);
                        }
                    },
                );
            }
        });

    if state.show_axis_gizmo {
        let gizmo_response = egui::Area::new(egui::Id::new("axis_gizmo"))
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(
                    -px(ctx, GIZMO_INSET_PX),
                    toolbar_height + px(ctx, GIZMO_INSET_PX),
                ),
            )
            .show(ctx, |ui| draw_axis_gizmo(ui, ctx, camera));
        output.axis_gizmo_action = gizmo_response.inner;
    }

    egui::Area::new(egui::Id::new("stats_overlay"))
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            egui::vec2(px(ctx, 30.0), -(status_bar_height + px(ctx, 18.0))),
        )
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_rgba_premultiplied(20, 22, 25, 130))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(52)))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(200.0);
                    stats_grid(ui, state);
                });
        });

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(status_bar_height)
        .frame(status_bar_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let info_response = icon_tile_button(
                    ui,
                    ctx,
                    &ICON_INFO,
                    false,
                    "Info",
                    egui::vec2(px(ctx, 42.0), px(ctx, 42.0)),
                );
                if info_response.clicked() {
                    state.status = "Viewport info".to_owned();
                }

                ui.add_space(14.0);
                ui.label(rich_label(
                    &state.status,
                    16.0,
                    egui::Color32::from_gray(188),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(rich_label(
                        if viewport_rect.width() > 1280.0 {
                            "WebGPU"
                        } else {
                            ""
                        },
                        14.0,
                        egui::Color32::from_gray(130),
                    ));
                });
            });
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
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(40, 39, 38))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(72)))
        .inner_margin(egui::Margin::symmetric(26, 10))
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

fn option_panel(
    ui: &mut egui::Ui,
    title: &str,
    expanded: &mut bool,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(39, 39, 39))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(66)))
        .corner_radius(6.0)
        .show(ui, |ui| {
            let header_rect = ui
                .horizontal(|ui| {
                    ui.set_min_height(42.0);
                    ui.add_space(12.0);
                    ui.label(rich_label(title, 17.0, egui::Color32::from_gray(226)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let arrow = if *expanded { "⌃" } else { "⌄" };
                        if ui
                            .add(
                                egui::Button::new(rich_label(
                                    arrow,
                                    18.0,
                                    egui::Color32::from_gray(190),
                                ))
                                .frame(false),
                            )
                            .clicked()
                        {
                            *expanded = !*expanded;
                        }
                    });
                })
                .response
                .rect;

            let header_response = ui.interact(
                header_rect,
                ui.make_persistent_id(title),
                egui::Sense::click(),
            );
            if header_response.clicked() {
                *expanded = !*expanded;
            }

            if *expanded {
                ui.separator();
                ui.add_space(14.0);
                ui.scope(|ui| {
                    ui.add_space(18.0);
                    add_contents(ui);
                    ui.add_space(18.0);
                });
            }
        });
}

fn labeled_slider(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.horizontal(|ui| {
        ui.set_min_width(400.0);
        ui.label(rich_label(label, 15.0, egui::Color32::from_gray(180)));
        ui.add_space(16.0);
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
        ui.label(rich_label(label, 15.0, egui::Color32::from_gray(180)));
        ui.add_space(16.0);
        for color in COLORS {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
            if response.clicked() {
                *selected = color;
            }
            let stroke = if *selected == color {
                egui::Stroke::new(2.0, egui::Color32::from_rgb(88, 135, 217))
            } else {
                egui::Stroke::new(1.0, egui::Color32::from_gray(28))
            };
            ui.painter()
                .rect(rect, 6.0, color, stroke, egui::StrokeKind::Outside);
        }
    });
}

fn wide_reset_button(ui: &mut egui::Ui) -> egui::Response {
    ui.add(
        egui::Button::new(rich_label("Reset all", 15.0, egui::Color32::from_gray(214)))
            .min_size(egui::vec2(394.0, 42.0))
            .fill(egui::Color32::from_rgb(20, 23, 26))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(62)))
            .corner_radius(6.0),
    )
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
    egui::Grid::new("stats_grid")
        .num_columns(2)
        .spacing(egui::vec2(18.0, 6.0))
        .show(ui, |ui| {
            stat_row(ui, "FPS", "0");
            stat_row(ui, "Draws", &state.stats.draw_count.to_string());
            stat_row(ui, "Polys", &state.stats.polygon_count.to_string());
            stat_row(ui, "Tris", &state.stats.triangle_count.to_string());
            stat_row(ui, "Verts", &state.stats.vertex_count.to_string());
            stat_row(ui, "UV Sets", &state.stats.uv_set_count.to_string());
            stat_row(ui, "Colors", "yes");
            stat_row(ui, "WebGPU", "yes");
        });
}

fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(rich_label(label, 14.5, egui::Color32::from_gray(178)));
    ui.label(rich_label(value, 14.5, egui::Color32::from_gray(232)));
    ui.end_row();
}

fn draw_axis_gizmo(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    camera: OrbitCamera,
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

    let mut points = axis_gizmo_points(camera, center, reach);
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
            painter.text(
                point.position,
                egui::Align2::CENTER_CENTER,
                point.label,
                egui::FontId::proportional(px(ctx, 13.0)),
                egui::Color32::from_rgb(16, 21, 27),
            );
        } else {
            let fill = if response.hovered() {
                point.color.linear_multiply(0.35)
            } else {
                egui::Color32::TRANSPARENT
            };
            painter.circle_filled(point.position, radius, fill);
            painter.circle_stroke(
                point.position,
                radius,
                egui::Stroke::new(px(ctx, 2.0), point.color),
            );
            if response.hovered() {
                painter.text(
                    point.position,
                    egui::Align2::CENTER_CENTER,
                    point.label,
                    egui::FontId::proportional(px(ctx, 11.0)),
                    point.color,
                );
            }
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

fn axis_gizmo_points(camera: OrbitCamera, center: egui::Pos2, reach: f32) -> Vec<AxisGizmoPoint> {
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

    AXES.into_iter()
        .map(|(axis, world, color, label, tooltip, positive)| {
            let view = camera.view_space_direction(world);
            AxisGizmoPoint {
                axis,
                position: egui::pos2(center.x + view.x * reach, center.y - view.y * reach),
                depth: view.z,
                color,
                label,
                tooltip,
                positive,
            }
        })
        .collect()
}

fn rich_label(text: &str, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text).size(size).color(color).strong()
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
