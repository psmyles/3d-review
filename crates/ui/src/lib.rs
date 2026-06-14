use review_model::ModelStats;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    ThreeD,
    Uv,
    Texture,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugToggles {
    pub wireframe: bool,
    pub face_normals: bool,
    pub vertex_normals: bool,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub mode: WorkspaceMode,
    pub debug: DebugToggles,
    pub status: String,
    pub stats: ModelStats,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            mode: WorkspaceMode::ThreeD,
            debug: DebugToggles::default(),
            status: "Ready".to_owned(),
            stats: ModelStats::default(),
        }
    }
}

pub fn draw_viewport_guides(ctx: &egui::Context, state: &UiState) {
    if state.mode != WorkspaceMode::ThreeD {
        return;
    }

    let rect = ctx.input(|input| input.screen_rect());
    let painter = ctx.layer_painter(egui::LayerId::background());
    let center = egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.12);
    let grid_extent = 12;
    let cell_x = (rect.width() / 34.0).clamp(22.0, 48.0);
    let cell_y = cell_x * 0.42;

    let project =
        |x: f32, z: f32| egui::pos2(center.x + (x - z) * cell_x, center.y + (x + z) * cell_y);

    for line in -grid_extent..=grid_extent {
        let strong = line % 4 == 0;
        let color = if strong {
            egui::Color32::from_rgba_unmultiplied(108, 124, 136, 72)
        } else {
            egui::Color32::from_rgba_unmultiplied(96, 104, 112, 34)
        };
        let width = if strong { 1.15 } else { 0.75 };

        painter.line_segment(
            [
                project(line as f32, -grid_extent as f32),
                project(line as f32, grid_extent as f32),
            ],
            egui::Stroke::new(width, color),
        );
        painter.line_segment(
            [
                project(-grid_extent as f32, line as f32),
                project(grid_extent as f32, line as f32),
            ],
            egui::Stroke::new(width, color),
        );
    }

    painter.line_segment(
        [
            project(-grid_extent as f32, 0.0),
            project(grid_extent as f32, 0.0),
        ],
        egui::Stroke::new(2.0, egui::Color32::from_rgb(215, 79, 82)),
    );
    painter.line_segment(
        [
            project(0.0, -grid_extent as f32),
            project(0.0, grid_extent as f32),
        ],
        egui::Stroke::new(2.0, egui::Color32::from_rgb(70, 136, 220)),
    );
    painter.circle_filled(
        project(0.0, 0.0),
        3.0,
        egui::Color32::from_rgb(224, 230, 235),
    );
}

pub fn draw_overlay(ctx: &egui::Context, state: &mut UiState) {
    egui::Area::new(egui::Id::new("top_toolbar"))
        .fixed_pos(egui::pos2(12.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        state.status = "Open requested".to_owned();
                    }

                    ui.separator();
                    ui.selectable_value(&mut state.mode, WorkspaceMode::ThreeD, "3D");
                    ui.add_enabled_ui(false, |ui| {
                        ui.selectable_value(&mut state.mode, WorkspaceMode::Uv, "UV");
                        ui.selectable_value(&mut state.mode, WorkspaceMode::Texture, "Tex");
                    });
                });
            });
        });

    egui::Area::new(egui::Id::new("left_options"))
        .fixed_pos(egui::pos2(12.0, 58.0))
        .show(ctx, |ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.set_width(210.0);
                ui.heading("Viewport");
                ui.checkbox(&mut state.debug.wireframe, "Wireframe");
                ui.checkbox(&mut state.debug.face_normals, "Face normals");
                ui.checkbox(&mut state.debug.vertex_normals, "Vertex normals");
            });
        });

    egui::Area::new(egui::Id::new("stats_overlay"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 58.0))
        .show(ctx, |ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.set_width(190.0);
                ui.label(format!("Polygons: {}", state.stats.polygon_count));
                ui.label(format!("Triangles: {}", state.stats.triangle_count));
                ui.label(format!("Vertices: {}", state.stats.vertex_count));
                ui.label(format!("UV sets: {}", state.stats.uv_set_count));
                ui.label(format!("Draws: {}", state.stats.draw_count));
            });
        });

    egui::Area::new(egui::Id::new("bottom_status"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -10.0))
        .show(ctx, |ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.label(&state.status);
            });
        });
}
