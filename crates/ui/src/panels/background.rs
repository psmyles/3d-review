//! Viewport-background tool options: the preset swatch row and a reset to default.

use review_render::ViewportBackground;

use crate::state::UiState;
use crate::widgets::{background_swatch_row, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "background", |ui| {
        background_swatch_row(ui, "Background", &mut state.viewport_background);
    });
    ui.separator();
    if reset_button(ui).clicked() {
        state.viewport_background = ViewportBackground::default();
    }
}
