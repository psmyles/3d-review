//! Wireframe tool options: color, thickness mode and a reset to defaults.

use crate::state::{
    UiState, WIREFRAME_SCREEN_THICKNESS_MAX, WIREFRAME_SCREEN_THICKNESS_MIN,
    WIREFRAME_WORLD_THICKNESS_MAX, WIREFRAME_WORLD_THICKNESS_MIN, WireframePanelState,
};
use crate::theme::{color, size};
use crate::widgets::{color_swatch_row, labeled_checkbox, labeled_slider, wide_reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    color_swatch_row(
        ui,
        "Wireframe color",
        &mut state.wireframe.color,
        &color::WIREFRAME_SWATCHES,
    );
    ui.add_space(size::PANEL_ROW_GAP);
    let (thickness, range) = if state.wireframe.use_world_units {
        (
            &mut state.wireframe.world_thickness,
            WIREFRAME_WORLD_THICKNESS_MIN..=WIREFRAME_WORLD_THICKNESS_MAX,
        )
    } else {
        (
            &mut state.wireframe.screen_thickness,
            WIREFRAME_SCREEN_THICKNESS_MIN..=WIREFRAME_SCREEN_THICKNESS_MAX,
        )
    };
    labeled_slider(ui, "Thickness", thickness, range);
    ui.add_space(size::PANEL_ROW_GAP);
    labeled_checkbox(ui, "Use World Units", &mut state.wireframe.use_world_units);
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}
