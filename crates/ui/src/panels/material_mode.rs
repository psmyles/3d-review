//! Material Mode tool options (behind the Source Material button): how the filled
//! faces are shaded — the imported source material (default), one uniform standard
//! material, or a unique randomized hue per mesh part. The renderer applies the
//! choice as an *effective* material table; the imported materials are untouched.

use review_render::MaterialMode;

use crate::state::UiState;
use crate::widgets::{labeled_combo, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "material_mode", |ui| {
        mode_row(ui, &mut state.debug.material_mode);
    });
    ui.separator();
    if reset_button(ui).clicked() {
        state.debug.material_mode = MaterialMode::default();
    }
}

/// "Mode" row: a dropdown choosing how the source-material faces are shaded.
fn mode_row(ui: &mut egui::Ui, mode: &mut MaterialMode) {
    labeled_combo(ui, "Mode", "material_mode_combo", mode.label(), |ui| {
        for option in MaterialMode::ALL {
            ui.selectable_value(mode, option, option.label());
        }
    });
}
