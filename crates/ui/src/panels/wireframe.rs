//! Wireframe tool options: color and a reset to defaults.

use crate::state::{UiState, WireframePanelState};
use crate::theme::color;
use crate::widgets::{color_swatch_row, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "wireframe", |ui| {
        color_swatch_row(
            ui,
            "Wireframe color",
            &mut state.wireframe.color,
            &color::WIREFRAME_SWATCHES,
        );
    });
    ui.separator();
    if reset_button(ui).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}
