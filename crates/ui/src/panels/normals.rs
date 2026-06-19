//! Face- and vertex-normal tool options. Both share [`NormalPanelState`]: a line
//! length slider and a line color swatch row, plus a reset to that view's
//! defaults.

use crate::state::{
    DEFAULT_NORMAL_LENGTH, NORMAL_LENGTH_MAX, NORMAL_LENGTH_MIN, NormalPanelState, UiState,
};
use crate::theme::{color, size};
use crate::widgets::{color_swatch_row, labeled_slider_with_value, wide_reset_button};

pub(super) fn face_body(ui: &mut egui::Ui, state: &mut UiState) {
    normal_body(ui, &mut state.face_normals, color::FACE_NORMAL_DEFAULT);
}

pub(super) fn vertex_body(ui: &mut egui::Ui, state: &mut UiState) {
    normal_body(ui, &mut state.vertex_normals, color::VERTEX_NORMAL_DEFAULT);
}

fn normal_body(ui: &mut egui::Ui, normals: &mut NormalPanelState, default_color: egui::Color32) {
    labeled_slider_with_value(
        ui,
        "Normal Length",
        &mut normals.length,
        NORMAL_LENGTH_MIN..=NORMAL_LENGTH_MAX,
        3,
        0.001,
    );
    ui.add_space(size::PANEL_ROW_GAP);
    color_swatch_row(
        ui,
        "Line Color",
        &mut normals.color,
        &color::NORMAL_SWATCHES,
    );
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        normals.length = DEFAULT_NORMAL_LENGTH;
        normals.color = default_color;
    }
}
