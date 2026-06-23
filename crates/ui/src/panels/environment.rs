//! Environment / image-based-lighting options: which HDR environment lights the
//! scene, whether it is drawn as the background, and an intensity multiplier.
//!
//! All write straight into [`UiState::environment`], which the viewport callback
//! reads each frame; the renderer rebuilds the precomputed IBL maps only when the
//! chosen environment changes. The IBL on/off master toggle is the IBL button in
//! the status bar (see `status_bar.rs`), not a row here.

use review_render::{EnvironmentMap, EnvironmentSettings};

use crate::state::UiState;
use crate::widgets::{
    labeled_checkbox, labeled_combo, labeled_slider_with_value, panel_grid, reset_button,
};

/// Inclusive range for the IBL intensity slider.
const INTENSITY_MIN: f32 = 0.0;
const INTENSITY_MAX: f32 = 3.0;

pub(super) fn body(ui: &mut egui::Ui, state: &mut UiState) {
    panel_grid(ui, "environment", |ui| {
        environment_row(ui, state);
        labeled_checkbox(ui, "Background", &mut state.environment.show_background);
        labeled_slider_with_value(
            ui,
            "Intensity",
            &mut state.environment.intensity,
            INTENSITY_MIN..=INTENSITY_MAX,
            2,
        );
    });

    if reset_button(ui).clicked() {
        // The IBL on/off state is owned by the status-bar toggle, so reset only
        // the panel's own options and leave `ibl_enabled` untouched.
        state.environment = EnvironmentSettings {
            ibl_enabled: state.environment.ibl_enabled,
            ..EnvironmentSettings::default()
        };
    }
}

/// "Environment" row: a dropdown over the built-in HDR maps.
fn environment_row(ui: &mut egui::Ui, state: &mut UiState) {
    labeled_combo(
        ui,
        "Environment",
        "environment_map",
        state.environment.map.label(),
        |ui| {
            for map in EnvironmentMap::ALL {
                ui.selectable_value(&mut state.environment.map, map, map.label());
            }
        },
    );
}
