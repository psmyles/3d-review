//! Anti Aliasing tool options: the scene MSAA level and an FXAA toggle.
//!
//! Both write straight into [`UiState::anti_aliasing`], which the viewport
//! callback reads each frame — there is no separate commit step. MSAA levels the
//! active adapter can't render are omitted from the dropdown entirely (invariant
//! 4); the currently-selected level is always offered so the menu can't strand
//! its own value.

use review_render::{AntiAliasing, MsaaSamples};

use crate::state::UiState;
use crate::widgets::{labeled_checkbox, labeled_combo, panel_grid, reset_button};

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "anti_aliasing", |ui| {
        msaa_row(ui, state);
        labeled_checkbox(ui, "FXAA", &mut state.anti_aliasing.fxaa);
    });
    if reset_button(ui).clicked() {
        state.anti_aliasing = AntiAliasing::default();
    }
}

/// "MSAA" row: a dropdown over the multisample levels. Only levels the active
/// adapter can actually render are listed; the rest are omitted so the menu is
/// honest about what the GPU can do.
fn msaa_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        "MSAA",
        "anti_aliasing_msaa",
        msaa_label(state.anti_aliasing.msaa),
        |ui| {
            for level in MsaaSamples::ALL {
                // List the active level unconditionally (so the menu can't strand
                // its own value); otherwise only list adapter-supported levels. An
                // empty support list means "not yet known", so only the current
                // selection shows until the adapter is queried.
                let offered =
                    level == state.anti_aliasing.msaa || state.supported_msaa.contains(&level);
                if offered {
                    ui.selectable_value(&mut state.anti_aliasing.msaa, level, msaa_label(level));
                }
            }
        },
    );
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
