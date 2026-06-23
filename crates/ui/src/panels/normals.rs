//! Face- and vertex-normal tool options. Both share [`NormalPanelState`]: a line
//! length slider and a line color swatch row, plus a reset to that view's
//! defaults.

use crate::state::{
    DEFAULT_NORMAL_LENGTH, NORMAL_LENGTH_MAX, NORMAL_LENGTH_MIN, NormalPanelState, UiState,
};
use crate::theme::color;
use crate::widgets::{labeled_color32, labeled_slider_with_value, panel_grid, reset_button};

pub(super) fn face_body(ui: &mut egui::Ui, state: &mut UiState) {
    normal_body(ui, &mut state.face_normals, color::FACE_NORMAL_DEFAULT);
}

pub(super) fn vertex_body(ui: &mut egui::Ui, state: &mut UiState) {
    normal_body(ui, &mut state.vertex_normals, color::VERTEX_NORMAL_DEFAULT);
}

fn normal_body(ui: &mut egui::Ui, normals: &mut NormalPanelState, default_color: egui::Color32) {
    panel_grid(ui, "normals", |ui| {
        labeled_slider_with_value(
            ui,
            "Normal Length",
            &mut normals.length,
            NORMAL_LENGTH_MIN..=NORMAL_LENGTH_MAX,
            3,
        );
        labeled_color32(ui, "Line Color", &mut normals.color);
    });
    if reset_button(ui).clicked() {
        normals.length = DEFAULT_NORMAL_LENGTH;
        normals.color = default_color;
    }
}
