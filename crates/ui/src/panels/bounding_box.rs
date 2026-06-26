//! Bounding-box tool options: which geometry the box wraps and its edge color,
//! plus a reset to defaults.

use crate::state::{BoundingBoxPanelState, BoundsScope, UiState};
use crate::theme::color;
use crate::widgets::{color_swatch_row, labeled_combo, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "bounding_box", |ui| {
        scope_row(ui, state);
        color_swatch_row(
            ui,
            "Box color",
            &mut state.bounding_box.color,
            &color::BOUNDING_BOX_SWATCHES,
        );
    });
    ui.separator();
    if reset_button(ui).clicked() {
        state.bounding_box = BoundingBoxPanelState::default();
    }
}

/// "Bounds" row: a dropdown choosing whether the box covers all meshes, only the
/// current Outliner selection, or only the Outliner's currently-visible meshes.
fn scope_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        "Bounds",
        "bounding_box_scope",
        state.bounding_box.scope.label(),
        |ui| {
            for scope in BoundsScope::ALL {
                ui.selectable_value(&mut state.bounding_box.scope, scope, scope.label());
            }
        },
    );
}
