//! Tonemapper options: which tone-map operator the composite pass applies when
//! converting the scene's linear HDR radiance to display range.
//!
//! Writes straight into [`UiState::tonemap`], which the viewport callback reads
//! each frame; there is no separate commit step. The tone-mapping on/off master
//! toggle is the tonemapper button in the status bar (see `status_bar.rs`), not a
//! row here.

use review_render::{TonemapOperator, TonemapSettings};

use crate::state::UiState;
use crate::widgets::{labeled_combo, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "tonemap", |ui| {
        operator_row(ui, state);
    });

    ui.separator();
    if reset_button(ui).clicked() {
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
        "Method",
        "tonemap_operator",
        state.tonemap.operator.label(),
        |ui| {
            for operator in TonemapOperator::ALL {
                ui.selectable_value(&mut state.tonemap.operator, operator, operator.label());
            }
        },
    );
}
