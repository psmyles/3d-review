//! Anti Aliasing tool options: the scene MSAA level.
//!
//! Writes straight into [`UiState::anti_aliasing`], which the viewport callback
//! reads each frame — there is no separate commit step. MSAA levels the active
//! adapter can't render are omitted from the dropdown entirely (invariant 4); the
//! currently-selected level is always offered so the menu can't strand its own
//! value.

use review_render::{AntiAliasing, MsaaSamples};

use crate::docs::Page;
use crate::keys;
use crate::labels;
use crate::state::UiState;
use crate::widgets::{Tip, labeled_combo, panel_footer, panel_grid};

/// This panel's page in the manual, opened by its footer's `?`.
const PAGE: Page = Page::PanelsAntiAliasing;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "anti_aliasing", |ui| {
        msaa_row(ui, state);
    });
    ui.separator();
    if panel_footer(ui, PAGE).clicked() {
        state.anti_aliasing = AntiAliasing::default();
    }
}

/// "MSAA" row: a dropdown over the multisample levels. Only levels the active
/// adapter can actually render are listed; the rest are omitted so the menu is
/// honest about what the GPU can do.
fn msaa_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        Tip::new(keys::ui_panels::MSAA)
            .describe(keys::ui_panels::MSAA_DESCRIPTION)
            .page(PAGE),
        "anti_aliasing_msaa",
        labels::msaa(state.anti_aliasing.msaa),
        |ui| {
            for level in MsaaSamples::ALL {
                // List the active level unconditionally (so the menu can't strand
                // its own value); otherwise only list adapter-supported levels. An
                // empty support list means "not yet known", so only the current
                // selection shows until the adapter is queried.
                let offered = level == state.anti_aliasing.msaa
                    || state.capabilities.msaa_levels.contains(&level);
                if offered {
                    ui.selectable_value(&mut state.anti_aliasing.msaa, level, labels::msaa(level));
                }
            }
        },
    );
}
