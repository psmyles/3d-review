use std::sync::Arc;

use review_model::{ModelData, ModelStats};
use review_render::{OrbitCamera, SceneCallback, SceneDebugOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    OpenModel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    ThreeD,
    Uv,
    Texture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadingMode {
    Wire,
    Unlit,
    Solid,
    Rendered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewProjectionMode {
    Perspective,
    Orthographic,
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
    pub show_bbox: bool,
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
            shading_mode: ShadingMode::Solid,
            projection_mode: ViewProjectionMode::Perspective,
            show_grid: true,
            show_bbox: false,
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
        SceneCallback::new(camera, output_format, model, model_revision, state.debug),
    );
    painter.add(callback);
}

pub fn draw_overlay(ctx: &egui::Context, state: &mut UiState) -> Option<UiAction> {
    apply_visuals(ctx);
    sync_debug_state(state);

    let mut action = None;
    let viewport_rect = ctx.input(|input| input.screen_rect());

    egui::TopBottomPanel::top("app_toolbar")
        .exact_height(72.0)
        .frame(toolbar_frame())
        .show(ctx, |ui| {
            ui.columns(3, |columns| {
                columns[0].with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_height(44.0);
                    ui.spacing_mut().item_spacing.x = 12.0;

                    if ui
                        .add(sized_text_button("Open", egui::vec2(68.0, 30.0)))
                        .clicked()
                    {
                        action = Some(UiAction::OpenModel);
                    }

                    toolbar_group(ui, |ui| {
                        let solid = matches!(state.shading_mode, ShadingMode::Solid);
                        let unlit = matches!(state.shading_mode, ShadingMode::Unlit);
                        let rendered = matches!(state.shading_mode, ShadingMode::Rendered);
                        let wire = matches!(state.shading_mode, ShadingMode::Wire);

                        if icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_shading_wire.png"),
                            wire,
                            "Wire",
                        )
                        .clicked()
                        {
                            state.shading_mode = ShadingMode::Wire;
                            state.debug.wireframe = true;
                        }
                        if icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_shading_unlit.png"),
                            unlit,
                            "Unlit",
                        )
                        .clicked()
                        {
                            state.shading_mode = ShadingMode::Unlit;
                        }
                        if icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_shading_solid.png"),
                            solid,
                            "Solid",
                        )
                        .clicked()
                        {
                            state.shading_mode = ShadingMode::Solid;
                            state.debug.wireframe = false;
                        }
                        if icon_toggle_button(
                            ui,
                            egui::include_image!(
                                "../../../assets/icons/icon_shading_wire_shaded.png"
                            ),
                            rendered,
                            "Rendered",
                        )
                        .clicked()
                        {
                            state.shading_mode = ShadingMode::Rendered;
                        }
                    });

                    toolbar_group(ui, |ui| {
                        icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_uv.png"),
                            state.uv_checker_panel_expanded,
                            "UV Checker",
                        )
                        .clicked()
                        .then(|| {
                            state.uv_checker_panel_expanded = !state.uv_checker_panel_expanded
                        });

                        if icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_normals_face.png"),
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
                            egui::include_image!("../../../assets/icons/icon_normals_vertex.png"),
                            state.debug.vertex_normals,
                            "Vertex Normals",
                        )
                        .clicked()
                        {
                            state.debug.vertex_normals = !state.debug.vertex_normals;
                            state.vertex_normals.expanded = state.debug.vertex_normals;
                        }
                    });
                });

                columns[1].with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        segmented_mode_control(ui, &mut state.mode);
                    },
                );

                columns[2].with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.set_min_height(44.0);
                    ui.spacing_mut().item_spacing.x = 12.0;

                    toolbar_group(ui, |ui| {
                        icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_view_ortho.png"),
                            matches!(state.projection_mode, ViewProjectionMode::Orthographic),
                            "Orthographic",
                        )
                        .clicked()
                        .then(|| state.projection_mode = ViewProjectionMode::Orthographic);

                        icon_toggle_button(
                            ui,
                            egui::include_image!(
                                "../../../assets/icons/icon_view_perspective.png"
                            ),
                            matches!(state.projection_mode, ViewProjectionMode::Perspective),
                            "Perspective",
                        )
                        .clicked()
                        .then(|| state.projection_mode = ViewProjectionMode::Perspective);
                    });

                    toolbar_group(ui, |ui| {
                        icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_bbox.png"),
                            state.show_bbox,
                            "Bounds",
                        )
                        .clicked()
                        .then(|| state.show_bbox = !state.show_bbox);

                        icon_toggle_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_grid.png"),
                            state.show_grid,
                            "Grid",
                        )
                        .clicked()
                        .then(|| state.show_grid = !state.show_grid);
                    });
                });
            });
        });

    egui::Area::new(egui::Id::new("left_options"))
        .fixed_pos(egui::pos2(18.0, 92.0))
        .show(ctx, |ui| {
            ui.set_width(432.0);
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

            ui.add_space(18.0);
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

            ui.add_space(18.0);
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
        });

    egui::Area::new(egui::Id::new("axis_gizmo"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-28.0, 116.0))
        .show(ctx, |ui| {
            draw_axis_gizmo(ui);
        });

    egui::Area::new(egui::Id::new("stats_overlay"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(18.0, -92.0))
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
        .exact_height(72.0)
        .frame(status_bar_frame())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let info_response = ui
                    .add_enabled_ui(true, |ui| {
                        icon_tile_button(
                            ui,
                            egui::include_image!("../../../assets/icons/icon_info.png"),
                            false,
                            "Info",
                            egui::vec2(40.0, 40.0),
                        )
                    })
                    .inner;
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

    action
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
    state.debug.wireframe = matches!(state.shading_mode, ShadingMode::Wire);
}

fn toolbar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(40, 39, 38))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(72)))
        .inner_margin(egui::Margin::symmetric(14, 12))
}

fn status_bar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(40, 39, 38))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(72)))
        .inner_margin(egui::Margin::symmetric(18, 14))
}

fn toolbar_group(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(18, 19, 22))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(48)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| add_contents(ui));
        });
}

fn segmented_mode_control(ui: &mut egui::Ui, mode: &mut WorkspaceMode) {
    toolbar_group(ui, |ui| {
        mode_segment(ui, mode, WorkspaceMode::ThreeD, "3D", true);
        mode_segment(ui, mode, WorkspaceMode::Uv, "UV", false);
        mode_segment(ui, mode, WorkspaceMode::Texture, "Tex", false);
    });
}

fn mode_segment(
    ui: &mut egui::Ui,
    mode: &mut WorkspaceMode,
    value: WorkspaceMode,
    label: &str,
    enabled: bool,
) {
    let selected = *mode == value;
    let button = egui::Button::new(rich_label(
        label,
        16.0,
        if selected {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(188)
        },
    ))
    .min_size(egui::vec2(56.0, 34.0))
    .fill(if selected {
        egui::Color32::from_rgb(66, 103, 163)
    } else {
        egui::Color32::TRANSPARENT
    })
    .stroke(egui::Stroke::NONE)
    .corner_radius(6.0);

    let response = ui.add_enabled(enabled, button);
    if enabled && response.clicked() {
        *mode = value;
    }
}

fn icon_toggle_button(
    ui: &mut egui::Ui,
    image: egui::ImageSource<'static>,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    icon_tile_button(ui, image, selected, tooltip, egui::vec2(36.0, 36.0))
}

fn icon_tile_button(
    ui: &mut egui::Ui,
    image: egui::ImageSource<'static>,
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
            egui::Color32::from_rgb(82, 126, 201),
        )
    } else if response.hovered() {
        (
            egui::Color32::from_rgb(47, 79, 131),
            egui::Color32::from_rgb(82, 126, 201),
        )
    } else {
        (
            egui::Color32::from_rgb(18, 19, 22),
            egui::Color32::from_gray(42),
        )
    };

    ui.painter().rect(
        rect,
        6.0,
        visuals.0,
        egui::Stroke::new(1.0, visuals.1),
        egui::StrokeKind::Outside,
    );

    let image_rect = rect.shrink2(egui::vec2(5.0, 5.0));
    egui::Image::new(image)
        .fit_to_exact_size(image_rect.size())
        .tint(tint)
        .paint_at(ui, image_rect);

    response.on_hover_text(tooltip)
}

fn sized_text_button(label: &str, size: egui::Vec2) -> egui::Button<'_> {
    egui::Button::new(rich_label(label, 15.0, egui::Color32::from_gray(220)))
        .min_size(size)
        .fill(egui::Color32::from_rgb(66, 66, 66))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(84)))
        .corner_radius(6.0)
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
                    ui.add_space(10.0);
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
                ui.scope(add_contents);
                ui.add_space(14.0);
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
            .min_size(egui::vec2(392.0, 42.0))
            .fill(egui::Color32::from_rgb(20, 23, 26))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(62)))
            .corner_radius(6.0),
    )
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

fn draw_axis_gizmo(ui: &mut egui::Ui) {
    let desired_size = egui::vec2(132.0, 132.0);
    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let origin = egui::pos2(center.x + 18.0, center.y - 6.0);
    let x_end = egui::pos2(origin.x + 46.0, origin.y - 16.0);
    let y_end = egui::pos2(origin.x, origin.y - 54.0);
    let z_end = egui::pos2(origin.x + 36.0, origin.y + 22.0);

    painter.line_segment(
        [origin, x_end],
        egui::Stroke::new(3.0, egui::Color32::from_rgb(242, 81, 93)),
    );
    painter.line_segment(
        [origin, y_end],
        egui::Stroke::new(3.0, egui::Color32::from_rgb(78, 238, 57)),
    );
    painter.line_segment(
        [origin, z_end],
        egui::Stroke::new(3.0, egui::Color32::from_rgb(63, 166, 239)),
    );

    painter.circle_stroke(
        egui::pos2(origin.x - 46.0, origin.y + 16.0),
        14.0,
        egui::Stroke::new(3.0, egui::Color32::from_rgb(242, 81, 93)),
    );
    painter.circle_stroke(
        egui::pos2(origin.x - 46.0, origin.y - 26.0),
        14.0,
        egui::Stroke::new(3.0, egui::Color32::from_rgb(63, 166, 239)),
    );
    painter.circle_stroke(
        egui::pos2(origin.x, origin.y + 56.0),
        14.0,
        egui::Stroke::new(3.0, egui::Color32::from_rgb(78, 238, 57)),
    );

    axis_bubble(painter, x_end, "X", egui::Color32::from_rgb(242, 81, 93));
    axis_bubble(painter, y_end, "Y", egui::Color32::from_rgb(78, 238, 57));
    axis_bubble(painter, z_end, "Z", egui::Color32::from_rgb(63, 166, 239));
}

fn axis_bubble(painter: &egui::Painter, center: egui::Pos2, label: &str, fill: egui::Color32) {
    painter.circle_filled(center, 16.0, fill);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(18.0),
        egui::Color32::BLACK,
    );
}

fn rich_label(text: &str, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text).size(size).color(color).strong()
}
