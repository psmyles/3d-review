//! Tonemapper options: which tone-map operator the composite pass applies when
//! converting the scene's linear HDR radiance to display range.
//!
//! Writes straight into [`UiState::tonemap`], which the viewport callback reads
//! each frame; there is no separate commit step. The tone-mapping on/off master
//! toggle is the tonemapper button in the status bar (see `status_bar.rs`), not a
//! row here.

use review_render::{TonemapOperator, TonemapSettings};

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::UiState;
use crate::widgets::{Tip, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsTonemapper;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "tonemap", |ui| {
        operator_row(ui, state);
    });

    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        // The on/off state is owned by the status-bar toggle, so reset only the
        // operator and leave `enabled` untouched.
        state.tonemap = TonemapSettings {
            enabled: state.tonemap.enabled,
            ..TonemapSettings::default()
        };
    }
}

/// "Method" row: a dropdown over the tone-map operators.
fn operator_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::TONEMAP_METHOD)
            .describe(keys::ui_panels::TONEMAP_METHOD_DESCRIPTION)
            .page(PAGE),
        "tonemap_operator",
        labels::tonemap(state.tonemap.operator),
        |ui| {
            for operator in TonemapOperator::ALL {
                ui.selectable_value(
                    &mut state.tonemap.operator,
                    operator,
                    labels::tonemap(operator),
                );
            }
        },
    );
}
