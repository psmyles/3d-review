//! Wireframe tool options: color, thickness mode and a reset to defaults.

use crate::state::{
    UiState, WIREFRAME_SCREEN_THICKNESS_MAX, WIREFRAME_SCREEN_THICKNESS_MIN,
    WIREFRAME_WORLD_THICKNESS_DISPLAY_MAX, WIREFRAME_WORLD_THICKNESS_DISPLAY_MIN,
    WIREFRAME_WORLD_THICKNESS_DISPLAY_SCALE, WireframePanelState,
};
use crate::theme::{color, size};
use crate::widgets::{
    color_swatch_row, labeled_checkbox, labeled_slider_with_value, wide_reset_button,
};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    color_swatch_row(
        ui,
        "Wireframe color",
        &mut state.wireframe.color,
        &color::WIREFRAME_SWATCHES,
    );
    ui.add_space(size::PANEL_ROW_GAP);
    if state.wireframe.use_world_units {
        // World thickness is stored in true world units but presented on the
        // friendlier 0.1–1.0 display scale; convert in, edit, convert back out.
        let mut shown = state.wireframe.world_thickness * WIREFRAME_WORLD_THICKNESS_DISPLAY_SCALE;
        labeled_slider_with_value(
            ui,
            "Thickness",
            &mut shown,
            WIREFRAME_WORLD_THICKNESS_DISPLAY_MIN..=WIREFRAME_WORLD_THICKNESS_DISPLAY_MAX,
            2,
            0.01,
        );
        state.wireframe.world_thickness = shown / WIREFRAME_WORLD_THICKNESS_DISPLAY_SCALE;
    } else {
        labeled_slider_with_value(
            ui,
            "Thickness",
            &mut state.wireframe.screen_thickness,
            WIREFRAME_SCREEN_THICKNESS_MIN..=WIREFRAME_SCREEN_THICKNESS_MAX,
            1,
            0.05,
        );
    }
    ui.add_space(size::PANEL_ROW_GAP);
    labeled_checkbox(ui, "Use World Units", &mut state.wireframe.use_world_units);
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        state.wireframe = WireframePanelState::default();
    }
}
