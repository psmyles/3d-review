//! Anti Aliasing tool options: the scene MSAA level and an FXAA toggle.
//!
//! Both write straight into [`UiState::anti_aliasing`], which the viewport
//! callback reads each frame — there is no separate commit step. MSAA levels the
//! active adapter can't render are omitted from the dropdown entirely (invariant
//! 4); the currently-selected level is always offered so the menu can't strand
//! its own value.

use review_render::{AntiAliasing, MsaaSamples};

use crate::state::UiState;
use crate::theme::size;
use crate::widgets::{compact_combo, labeled_checkbox, table_label_cell, wide_reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    msaa_row(ui, state);
    ui.add_space(size::PANEL_ROW_GAP);
    labeled_checkbox(ui, "FXAA", &mut state.anti_aliasing.fxaa);
    ui.add_space(size::PANEL_ACTION_GAP);
    if wide_reset_button(ui).clicked() {
        state.anti_aliasing = AntiAliasing::default();
    }
}

/// "MSAA" row: a dropdown over the multisample levels. Only levels the active
/// adapter can actually render are listed; the rest are omitted so the menu is
/// honest about what the GPU can do.
fn msaa_row(ui: &mut egui::Ui, state: &mut UiState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, "MSAA");
        compact_combo(
            ui,
            "anti_aliasing_msaa",
            control_w,
            msaa_label(state.anti_aliasing.msaa),
            |ui| {
                for level in MsaaSamples::ALL {
                    // List the active level unconditionally (so the menu can't
                    // strand its own value); otherwise only list adapter-supported
                    // levels. An empty support list means "not yet known", so only
                    // the current selection shows until the adapter is queried.
                    let offered =
                        level == state.anti_aliasing.msaa || state.supported_msaa.contains(&level);
                    if offered {
                        ui.selectable_value(
                            &mut state.anti_aliasing.msaa,
                            level,
                            msaa_label(level),
                        );
                    }
                }
            },
        );
    });
}

fn msaa_label(level: MsaaSamples) -> &'static str {
    match level {
        MsaaSamples::Off => "Off",
        MsaaSamples::X2 => "2x",
        MsaaSamples::X4 => "4x",
        MsaaSamples::X8 => "8x",
        MsaaSamples::X16 => "16x",
    }
}
