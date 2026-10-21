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
        reset(state);
    }
}

/// The footer's reset: the default MSAA level, or the nearest one this adapter
/// can render. The on/off state is owned by the status-bar toggle, as it is for
/// the other tools with one, so a reset leaves `enabled` where the user put it —
/// resetting a level must not switch antialiasing back on.
fn reset(state: &mut UiState) {
    state.anti_aliasing = AntiAliasing {
        enabled: state.anti_aliasing.enabled,
        msaa: state
            .capabilities
            .nearest_msaa(AntiAliasing::default().msaa),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reset_keeps_antialiasing_off_when_the_user_turned_it_off() {
        let mut state = UiState {
            anti_aliasing: AntiAliasing {
                enabled: false,
                msaa: MsaaSamples::X8,
            },
            ..UiState::default()
        };
        reset(&mut state);
        assert!(!state.anti_aliasing.enabled);
        assert_eq!(state.anti_aliasing.msaa, AntiAliasing::default().msaa);
    }

    #[test]
    fn a_reset_never_picks_a_level_the_adapter_cannot_render() {
        let mut state = UiState::default();
        // No 2x here, which is the default.
        state.capabilities.msaa_levels = vec![MsaaSamples::Off, MsaaSamples::X4];
        state.anti_aliasing.msaa = MsaaSamples::X4;
        reset(&mut state);
        assert_eq!(
            state.anti_aliasing.msaa,
            MsaaSamples::Off,
            "the highest level the adapter has below the default"
        );
    }
}
