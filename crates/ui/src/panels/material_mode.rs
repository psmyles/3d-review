//! Material Mode tool options (behind the Source Material button): how the filled
//! faces are shaded — the imported source material (default), one uniform standard
//! material, or a unique randomized hue per mesh part. The renderer applies the
//! choice as an *effective* material table; the imported materials are untouched.

use review_render::MaterialMode;

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::UiState;
use crate::widgets::{Tip, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsMaterialMode;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "material_mode", |ui| {
        mode_row(ui, &mut state.debug.material_mode);
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.debug.material_mode = MaterialMode::default();
    }
}

/// "Mode" row: a dropdown choosing how the source-material faces are shaded.
fn mode_row(ui: &mut egui::Ui, mode: &mut MaterialMode) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::MATERIAL_MODE)
            .describe(keys::ui_panels::MATERIAL_MODE_DESCRIPTION)
            .page(PAGE),
        "material_mode_combo",
        labels::material_mode(*mode),
        |ui| {
            for option in MaterialMode::ALL {
                ui.selectable_value(mode, option, labels::material_mode(option));
            }
        },
    );
}
