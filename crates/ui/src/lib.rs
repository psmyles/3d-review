use std::sync::Arc;

use review_model::{ModelData, ModelStats};
use review_render::{OrbitCamera, SceneCallback};

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
            status: "LMB orbit  RMB pan  Wheel zoom  F reset".to_owned(),
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
        SceneCallback::new(camera, output_format, model, model_revision),
    );
    painter.add(callback);
}

pub fn draw_overlay(ctx: &egui::Context, state: &mut UiState) -> Option<UiAction> {
    let mut action = None;

    egui::Area::new(egui::Id::new("top_toolbar"))
        .fixed_pos(egui::pos2(12.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        action = Some(UiAction::OpenModel);
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

    action
}
