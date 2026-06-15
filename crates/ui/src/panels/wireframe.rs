//! Wireframe tool options: the line color, plus a reset to its default.

use crate::state::{UiState, WireframePanelState};
use crate::theme::{color, size};
use crate::widgets::{color_swatch_row, wide_reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    color_swatch_row(
        ui,
        "Wireframe color",
        &mut state.wireframe.color,
        &color::WIREFRAME_SWATCHES,
    );
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}
