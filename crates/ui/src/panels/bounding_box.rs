//! Bounding-box tool options: which geometry the box wraps and its edge color,
//! plus a reset to defaults.

use crate::docs::Page;
use crate::keys;
use crate::state::{BoundingBoxPanelState, BoundsScope, UiState};
use crate::theme::color;
use crate::widgets::{Tip, color_swatch_row, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsBoundingBox;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "bounding_box", |ui| {
        scope_row(ui, state);
        color_swatch_row(
            ui,
            Tip::new(keys::ui_panels::BOX_COLOR)
                .describe(keys::ui_panels::BOX_COLOR_DESCRIPTION)
                .page(PAGE),
            &mut state.bounding_box.color,
            &color::BOUNDING_BOX_SWATCHES,
        );
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.bounding_box = BoundingBoxPanelState::default();
    }
}

/// "Bounds" row: a dropdown choosing whether the box covers all meshes, only the
/// current Outliner selection, or only the Outliner's currently-visible meshes.
fn scope_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::BOUNDS_SCOPE)
            .describe(keys::ui_panels::BOUNDS_SCOPE_DESCRIPTION)
            .page(PAGE),
        "bounding_box_scope",
        state.bounding_box.scope.label(),
        |ui| {
            for scope in BoundsScope::ALL {
                ui.selectable_value(&mut state.bounding_box.scope, scope, scope.label());
            }
        },
    );
}
