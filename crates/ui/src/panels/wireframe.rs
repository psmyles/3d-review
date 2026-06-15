//! Wireframe tool options: just the line color.

use crate::state::UiState;
use crate::theme::color;
use crate::widgets::color_swatch_row;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    color_swatch_row(
        ui,
        "Wireframe color",
        &mut state.wireframe.color,
        &color::WIREFRAME_SWATCHES,
    );
}
