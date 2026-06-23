//! Bounding-box tool options: the edge color, plus a reset to its default.

use crate::state::{BoundingBoxPanelState, UiState};
use crate::widgets::{labeled_color32, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "bounding_box", |ui| {
        labeled_color32(ui, "Box color", &mut state.bounding_box.color);
    });
    if reset_button(ui).clicked() {
        state.bounding_box = BoundingBoxPanelState::default();
    }
}
