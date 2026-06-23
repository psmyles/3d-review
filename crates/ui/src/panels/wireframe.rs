//! Wireframe tool options: color and a reset to defaults.

use crate::state::{UiState, WireframePanelState};
use crate::widgets::{labeled_color32, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "wireframe", |ui| {
        labeled_color32(ui, "Wireframe color", &mut state.wireframe.color);
    });
    if reset_button(ui).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}
