//! Tonemapper options: which tone-map operator the composite pass applies when
//! converting the scene's linear HDR radiance to display range.
//!
//! Writes straight into [`UiState::tonemap`], which the viewport callback reads
//! each frame; there is no separate commit step. The tone-mapping on/off master
//! toggle is the tonemapper button in the status bar (see `status_bar.rs`), not a
//! row here.

use review_render::{TonemapOperator, TonemapSettings};

use crate::state::UiState;
use crate::theme::size;
use crate::widgets::{compact_combo, table_label_cell, wide_reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    operator_row(ui, state);
    ui.add_space(size::PANEL_ACTION_GAP);

    if wide_reset_button(ui).clicked() {
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "Method");
        compact_combo(
            ui,
            "tonemap_operator",
            control_w,
            state.tonemap.operator.label(),
            |ui| {
                for operator in TonemapOperator::ALL {
                    ui.selectable_value(&mut state.tonemap.operator, operator, operator.label());
                }
            },
        );
    });
}
